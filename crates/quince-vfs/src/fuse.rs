//! The engine as a Linux folder, through FUSE (fuser).
//!
//! Names, sizes and dates come from the library, never the network, so a file manager
//! browsing is never kept waiting; a folder never opened before is fetched once. A file's
//! bytes come from the cache, each part fetched from Apple the first time it is read.
//! Folders, renames, moves and deletes are made at Apple as they happen. A file's own bytes
//! are written here first and sent behind the program writing them (see writes.rs).

use crate::writes::{self, Pending, Writes};
use crate::{is_thumbnailer, named, Cache, Inodes, Remote, Store};
use fuser::{
    BsdFileFlags, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenAccMode, OpenFlags, RenameFlags, ReplyAttr,
    ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs,
    ReplyWrite, Request, TimeOrNow, WriteFlags,
};
use quince_core::drive::DriveItem;
use std::ffi::OsStr;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long the kernel may keep what it was told about a name or a file. Short, as rclone's
/// is: a file changed on another device must not be read with its old length.
/// This computer's own disk refusing, told to the app as the disk told it — "No space left on
/// device" when it is full, never a vague input/output error that reads as a broken drive.
fn disk(e: std::io::Error) -> Errno {
    e.raw_os_error().map_or(Errno::EIO, Errno::from_i32)
}

const TTL: Duration = Duration::from_secs(1);

/// How long a folder never opened before may take to come from Apple before the file manager
/// is told it could not be read (and can ask again).
const LIST_WAIT: Duration = Duration::from_secs(30);
/// How long a folder's listing is taken as current before a look at it asks Apple again (in
/// the background: what is stored is shown at once). Apple pushes nothing for iCloud Drive.
const FRESH: Duration = Duration::from_secs(10);

/// How long a change (a new folder, a rename, a delete) may take at Apple.
const CHANGE_WAIT: Duration = Duration::from_secs(60);

/// The longest wait between tries of a send Apple would not take.
const LONGEST_RETRY: Duration = Duration::from_secs(300);

/// What the folder's threads and its sends share.
struct Inner {
    remote: Arc<dyn Remote>,
    store: Arc<dyn Store>,
    cache: Arc<Cache>,
    inodes: Mutex<Inodes>,
    rt: tokio::runtime::Handle,
    uid: u32,
    gid: u32,
    mounted: SystemTime,
    /// Where the folder is mounted and the thumbnail cache's root, when thumbnails are wanted.
    thumbs: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// Folders whose thumbnails were written, by their items' versions: written once.
    filled: Mutex<std::collections::HashSet<String>>,
    /// Files written here and not yet at Apple as written. `None`: the folder is read-only.
    writes: Option<Arc<Writes>>,
    /// When each folder was last asked of Apple, by id.
    looked: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The account's storage as last asked, `(used, total)`.
    space: Mutex<Option<((u64, u64), std::time::Instant)>>,
    /// Nothing in it can be changed.
    read_only: bool,
    /// New files can be added and anything deleted, nothing else (the Photos folder: a photo is
    /// added or deleted, never rewritten in place, renamed or given a folder).
    add_only: bool,
}

pub struct DriveFs(Arc<Inner>);

impl DriveFs {
    #[must_use]
    pub fn new(
        remote: Arc<dyn Remote>,
        store: Arc<dyn Store>,
        cache: Arc<Cache>,
        rt: tokio::runtime::Handle,
    ) -> Self {
        // SAFETY: getuid and getgid cannot fail and touch no memory.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        Self(Arc::new(Inner {
            remote,
            store,
            cache,
            inodes: Mutex::new(Inodes::new()),
            rt,
            uid,
            gid,
            mounted: SystemTime::now(),
            thumbs: None,
            filled: Mutex::new(std::collections::HashSet::new()),
            writes: None,
            space: Mutex::new(None),
            looked: Mutex::new(std::collections::HashMap::new()),
            read_only: false,
            add_only: false,
        }))
    }

    fn inner_mut(&mut self) -> &mut Inner {
        Arc::get_mut(&mut self.0).expect("configured before it is mounted")
    }

    /// Apple's pictures of each folder listed put in the thumbnail cache (the crate's
    /// write_thumbnails), for a folder mounted at `mount`.
    #[must_use]
    pub fn with_thumbnails(mut self, mount: std::path::PathBuf, cache: std::path::PathBuf) -> Self {
        self.inner_mut().thumbs = Some((mount, cache));
        self
    }

    /// What is written here and not yet at Apple, for the app to show (before mounting).
    #[must_use]
    pub fn writes(&self) -> Option<Arc<Writes>> {
        self.0.writes.clone()
    }

    /// Nothing in it can be changed: every change answered "read-only file system".
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.inner_mut().read_only = true;
        self
    }

    /// New files can be added (into folders the remote `accepts`) and files deleted; nothing
    /// is rewritten, renamed, moved or made a folder — "read-only file system" to all of that.
    #[must_use]
    pub fn add_only(mut self) -> Self {
        self.inner_mut().add_only = true;
        self
    }

    /// Files can be written, their bytes kept in `dir` until Apple has them. What was waiting
    /// when the folder last went away is picked up again and sent.
    ///
    /// # Errors
    /// When the folder for the bytes cannot be made.
    pub fn with_writes(mut self, dir: &Path) -> std::io::Result<Self> {
        let writes = Arc::new(Writes::new(dir)?);
        let waiting = writes.load();
        self.inner_mut().writes = Some(Arc::clone(&writes));
        for (old, pending) in waiting {
            let ino = self
                .0
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .local(&pending.folder, &pending.name);
            // Its bytes moved to the inode it has now.
            let from = writes.staging(old, &pending.name);
            let to = writes.staging(ino, &pending.name);
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::rename(&from, &to)?;
            writes.drop_staging(old);
            writes
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    ino,
                    Pending {
                        dirty: true,
                        ..pending
                    },
                );
            Inner::schedule(&self.0, ino, Duration::from_millis(500));
        }
        writes.save();
        Ok(self)
    }
}

impl Inner {
    /// In the background, once per folder version: the folder's thumbnails written.
    fn fill_thumbnails(&self, folder: &DriveItem, files: &[(String, u64, DriveItem)]) {
        let Some((mount, cache)) = self.thumbs.clone() else {
            return;
        };
        let mut paths = Vec::new();
        let mut fingerprint = folder.drivewsid.clone();
        {
            let inodes = self
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (_, ino, item) in files
                .iter()
                .filter(|(_, _, i)| !i.is_container() && !i.drivewsid.starts_with("LOCAL::"))
            {
                if let Some(p) = inodes.path(*ino) {
                    paths.push((p.to_string(), item.clone()));
                    fingerprint.push_str(item.etag.as_deref().unwrap_or(""));
                }
            }
        }
        if paths.is_empty()
            || !self
                .filled
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(fingerprint)
        {
            return;
        }
        let (remote, id) = (Arc::clone(&self.remote), folder.drivewsid.clone());
        self.rt.spawn(async move {
            let Ok(pictures) = remote.thumbnails(&id).await else {
                return;
            };
            if !pictures.is_empty() {
                let _ = tokio::task::spawn_blocking(move || {
                    crate::write_thumbnails(&mount, &cache, &paths, &pictures)
                })
                .await;
            }
        });
    }

    /// A change made at Apple, waited for (the file manager waits on it too), then the folders
    /// it touched read again so the library — and so the next listing — is Apple's.
    fn change<T: Send + 'static>(
        &self,
        work: crate::Fut<T>,
        touched: &[&DriveItem],
    ) -> Result<T, Errno> {
        let done = self
            .rt
            .block_on(async { tokio::time::timeout(CHANGE_WAIT, work).await })
            .map_err(|_| Errno::ETIMEDOUT)?
            .map_err(|e| {
                quiet_note(&e);
                Errno::EIO
            })?;
        for folder in touched {
            self.relist(&folder.drivewsid);
        }
        Ok(done)
    }

    /// Whether a folder was asked of Apple within `FRESH`; if not, it is marked asked now (so
    /// many looks at once ask once).
    fn stale(&self, folder: &str) -> bool {
        let mut looked = self
            .looked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        if looked
            .get(folder)
            .is_some_and(|at| now.duration_since(*at) < FRESH)
        {
            return false;
        }
        looked.insert(folder.to_string(), now);
        true
    }

    /// A folder looked at: shown from what is stored, and asked of Apple behind that when its
    /// listing is older than `FRESH` — a file saved on the phone is there on the next look.
    fn revalidate(&self, folder: &str) {
        if self.stale(folder) {
            let (remote, id) = (Arc::clone(&self.remote), folder.to_string());
            self.rt.spawn(async move {
                let _ = tokio::time::timeout(LIST_WAIT, remote.list(&id)).await;
            });
        }
    }

    fn relist(&self, folder: &str) {
        let (remote, id) = (Arc::clone(&self.remote), folder.to_string());
        let _ = self
            .rt
            .block_on(async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await });
    }

    fn item(&self, ino: u64) -> Option<DriveItem> {
        self.inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .item(ino)
            .cloned()
    }

    fn pending(&self, ino: u64) -> Option<Pending> {
        self.writes
            .as_ref()?
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&ino)
            .cloned()
    }

    fn with_pending<R>(&self, ino: u64, f: impl FnOnce(&mut Pending) -> R) -> Option<R> {
        let writes = self.writes.as_ref()?;
        let mut items = writes
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.get_mut(&ino).map(f)
    }

    fn attr(&self, ino: u64, item: &DriveItem) -> FileAttr {
        // Apple lists no dates for a folder: it shows when the folder was mounted rather than
        // 1 January 1970, which a file manager sorts and labels as if it were real.
        let mounted = self.mounted;
        let secs = |s: Option<i64>| {
            s.filter(|&s| s > 0).map_or(mounted, |s| {
                UNIX_EPOCH + Duration::from_secs(u64::try_from(s).unwrap_or(0))
            })
        };
        let dir = item.is_container();
        let (mut size, mut mtime) = (
            if dir {
                0
            } else {
                u64::try_from(item.size.unwrap_or(0)).unwrap_or(0)
            },
            secs(item.modified_at),
        );
        // A file being written shows what has been written, as of when.
        if let (Some(p), Some(w)) = (self.pending(ino), self.writes.as_ref()) {
            size = std::fs::metadata(w.staging(ino, &p.name)).map_or(size, |m| m.len());
            mtime = p.modified;
        }
        FileAttr {
            ino: INodeNo(ino),
            size,
            blocks: size.div_ceil(512),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: secs(item.created_at.or(item.modified_at)),
            kind: if dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm: match (dir, self.writes.is_some() && !self.read_only) {
                (true, _) if self.read_only => 0o555,
                (true, _) => 0o755,
                (false, true) if !self.add_only => 0o644,
                (false, true) => 0o444,
                (false, false) => 0o444,
            },
            nlink: if dir { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// A folder's items with their names and inodes: from the library — or, never opened
    /// before, from Apple once — with the files being written here among them.
    fn children(&self, ino: u64) -> Result<Vec<(String, u64, DriveItem)>, Errno> {
        let folder = self.item(ino).ok_or(Errno::ENOENT)?;
        if !folder.is_container() {
            return Err(Errno::ENOTDIR);
        }
        let stored = self.store.children(&folder.drivewsid);
        // A folder Apple says has things in it, with nothing stored, was never listed.
        let listed = stored.filter(|s| !s.is_empty() || folder.child_count == Some(0));
        let mut items = match listed {
            Some(items) => items,
            None => {
                let remote = Arc::clone(&self.remote);
                let id = folder.drivewsid.clone();
                self.rt
                    .block_on(
                        async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await },
                    )
                    .map_err(|_| Errno::ETIMEDOUT)?
                    .map_err(|_| Errno::EIO)?
            }
        };
        // Files being written here: a new one is shown beside Apple's; one taking another's
        // place is shown in it, under its own inode.
        let mut local: Vec<(u64, Pending)> = Vec::new();
        if let Some(w) = &self.writes {
            let pending = w
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            local = pending
                .iter()
                .filter(|(_, p)| p.folder == folder.drivewsid)
                .map(|(i, p)| (*i, p.clone()))
                .collect();
        }
        let mut inodes = self
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (pino, p) in &local {
            if let Some(r) = &p.replaces {
                items.retain(|i| i.drivewsid != r.drivewsid);
            }
            // Its own inode keeps the name it is being written under.
            if let Some(it) = inodes.item(*pino).cloned() {
                if it.drivewsid.starts_with("LOCAL::")
                    || p.replaces
                        .as_ref()
                        .is_some_and(|r| r.drivewsid == it.drivewsid)
                {
                    items.retain(|i| i.drivewsid != it.drivewsid);
                }
            }
        }
        let mut out: Vec<(String, u64, DriveItem)> = named(&items)
            .into_iter()
            .map(|(name, it)| {
                let child = inodes.ino(&it);
                inodes.place(child, ino, &name);
                (name, child, it)
            })
            .collect();
        for (pino, p) in local {
            let Some(it) = inodes.item(pino).cloned().or_else(|| p.replaces.clone()) else {
                continue;
            };
            inodes.place(pino, ino, &p.name);
            out.push((p.name.clone(), pino, it));
        }
        Ok(out)
    }

    /// The item called `name` in folder `parent`.
    fn child(&self, parent: u64, name: &OsStr) -> Result<(u64, DriveItem), Errno> {
        self.children(parent)?
            .into_iter()
            .find(|(n, ..)| OsStr::new(n) == name)
            .map(|(_, ino, item)| (ino, item))
            .ok_or(Errno::ENOENT)
    }

    /// A file of Apple's made writable: its bytes copied here (unless they are about to be
    /// thrown away), to be sent back in its place.
    fn begin_writing(&self, ino: u64, item: &DriveItem, keep: bool) -> Result<(), Errno> {
        if self.add_only {
            return Err(Errno::EROFS);
        }
        let writes = self.writes.as_ref().ok_or(Errno::EROFS)?;
        if self.pending(ino).is_some() {
            return Ok(());
        }
        let name = {
            let inodes = self
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inodes
                .path(ino)
                .and_then(|p| p.rsplit('/').next())
                .map_or_else(|| item.display_name(), str::to_string)
        };
        let staged = writes.staging(ino, &name);
        std::fs::create_dir_all(staged.parent().ok_or(Errno::EIO)?).map_err(disk)?;
        let file = std::fs::File::create(&staged).map_err(disk)?;
        if keep {
            let size = u64::try_from(item.size.unwrap_or(0)).unwrap_or(0);
            let (cache, remote, it) = (
                Arc::clone(&self.cache),
                Arc::clone(&self.remote),
                item.clone(),
            );
            let mut at = 0;
            while at < size {
                let len = crate::CHUNK.min(size - at);
                let (c, r, i) = (Arc::clone(&cache), Arc::clone(&remote), it.clone());
                let bytes = self
                    .rt
                    .block_on(async move { c.read(r.as_ref(), &i, at, len).await })
                    .map_err(|_| Errno::EIO)?;
                file.write_all_at(&bytes, at).map_err(disk)?;
                at += len;
            }
        }
        writes
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                ino,
                Pending {
                    folder: item
                        .parent_id
                        .clone()
                        .unwrap_or_else(|| crate::ROOT.to_string()),
                    name,
                    replaces: Some(item.clone()),
                    open: 0,
                    dirty: !keep,
                    generation: 0,
                    modified: SystemTime::now(),
                    sender: false,
                    quiet_since: None,
                    sending: false,
                    failure: None,
                },
            );
        Ok(())
    }

    /// Sent to Apple once left alone a moment (writes::SETTLE), and tried again until Apple
    /// takes it. Only the bytes that were sent are counted as sent.
    fn schedule(me: &Arc<Self>, ino: u64, after: Duration) {
        // One sender per file: a second close, or a read-back, never starts a second send of
        // the same bytes. Claimed under the same lock that counts who has it open.
        let claimed = me
            .with_pending(ino, |p| {
                p.quiet_since = Some(std::time::Instant::now());
                if p.sender {
                    false
                } else {
                    p.sender = true;
                    true
                }
            })
            .unwrap_or(false);
        if !claimed {
            return;
        }
        let me = Arc::clone(me);
        let rt = me.rt.clone();
        rt.spawn(async move {
            tokio::time::sleep(after).await;
            let mut wait = Duration::from_secs(5);
            loop {
                // Left alone long enough? Open again, or nothing to send: the sender stands
                // down, and the next close claims it again.
                loop {
                    let state = me.with_pending(ino, |p| {
                        if p.open > 0 || !p.dirty || writes::stays_local(&p.name) {
                            p.sender = false;
                            Some(false)
                        } else if p.quiet_since.is_some_and(|q| q.elapsed() < writes::SETTLE) {
                            None
                        } else {
                            Some(true)
                        }
                    });
                    match state {
                        None | Some(Some(false)) => return,
                        Some(Some(true)) => break,
                        Some(None) => tokio::time::sleep(Duration::from_millis(250)).await,
                    }
                }
                let Some(p) = me.pending(ino) else { return };
                let generation = p.generation;
                let Some(writes) = me.writes.clone() else {
                    return;
                };
                let folder = {
                    let inodes = me
                        .inodes
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    inodes
                        .ino_of(&p.folder)
                        .and_then(|f| inodes.item(f).cloned())
                };
                let Some(folder) = folder else {
                    me.with_pending(ino, |p| p.sender = false);
                    return;
                };
                let key = format!("{}/{}", p.folder, p.name);
                // Behind any send of the same name still under way, so this one finds it
                // finished and replaces it as this computer's own.
                let lane = writes.lane(&key);
                let _turn = lane.lock().await;
                let ours = writes
                    .sent
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&key)
                    .cloned();
                // Sent from a copy of the bytes as they are now: a write during the send
                // changes the file, never what is on its way.
                let out_dir = writes.staging(ino, &p.name).with_file_name(".out");
                let out = out_dir.join(&p.name);
                let copied = std::fs::create_dir_all(&out_dir).is_ok()
                    && std::fs::copy(writes.staging(ino, &p.name), &out).is_ok();
                me.with_pending(ino, |p| p.sending = true);
                let result = if copied {
                    // Where only adding is, a send is an addition and nothing else: never a
                    // replacement of a file found under the same name.
                    let r = if me.add_only {
                        me.remote
                            .upload(&folder, &out)
                            .await
                            .map(|item| writes::Sent {
                                item,
                                conflicted: false,
                            })
                    } else {
                        writes::send(
                            me.remote.as_ref(),
                            &folder,
                            &out,
                            p.replaces.as_ref(),
                            ours.as_ref(),
                        )
                        .await
                    };
                    let _ = std::fs::remove_dir_all(&out_dir);
                    r
                } else {
                    Err("the bytes could not be read".into())
                };
                me.with_pending(ino, |p| {
                    p.sending = false;
                    p.failure = result.as_ref().err().map(|e| e.chars().take(160).collect());
                });
                match result {
                    Ok(sent) => {
                        if !sent.conflicted {
                            // This computer's own, so its next save replaces it as such.
                            writes
                                .sent
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .insert(key, sent.item.clone());
                        }
                        let done = me
                            .with_pending(ino, |now| {
                                if now.generation == generation {
                                    true
                                } else {
                                    // Written again while on its way: what Apple has now is
                                    // what the next send replaces.
                                    now.replaces = Some(sent.item.clone());
                                    false
                                }
                            })
                            .unwrap_or(true);
                        let _ = me.remote.list(&folder.drivewsid).await;
                        if done {
                            if let Some(w) = &me.writes {
                                w.items
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .remove(&ino);
                                w.drop_staging(ino);
                                w.save();
                            }
                            if !sent.conflicted {
                                me.inodes
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .bind(ino, &sent.item);
                            }
                            return;
                        }
                    }
                    Err(e) => {
                        quiet_note(&e);
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(LONGEST_RETRY);
                    }
                }
            }
        });
    }

    /// A file or (an empty) folder deleted, into Apple's Recently Deleted. A file made here
    /// and never sent is simply let go of.
    fn remove(&self, parent: u64, name: &OsStr, folder: bool) -> Result<(), Errno> {
        let from = self.item(parent).ok_or(Errno::ENOENT)?;
        let (ino, item) = self.child(parent, name)?;
        if folder != item.is_container() {
            return Err(if folder {
                Errno::ENOTDIR
            } else {
                Errno::EISDIR
            });
        }
        if let Some(p) = self.pending(ino) {
            if let Some(w) = &self.writes {
                w.items
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&ino);
                w.drop_staging(ino);
                w.save();
            }
            return match p.replaces {
                Some(old) => self.change(self.remote.delete(&old), &[&from]),
                None => Ok(()),
            };
        }
        // A folder goes only when empty, as on any disk: Apple would take everything in it,
        // and a file manager or `rm -r` empties it first, file by file.
        if folder && !self.children(ino).map(|c| c.is_empty()).unwrap_or(false) {
            return Err(Errno::ENOTEMPTY);
        }
        self.change(self.remote.delete(&item), &[&from])
    }

    /// An item moved and renamed at Apple: into `to`, then given `newname`.
    fn move_and_rename(
        &self,
        item: &DriveItem,
        from: &DriveItem,
        to: &DriveItem,
        name: &OsStr,
        newname: &str,
    ) -> Result<(), Errno> {
        if from.drivewsid != to.drivewsid {
            self.change(self.remote.move_to(item, to), &[from, to])?;
        }
        if name != OsStr::new(newname) {
            let moved = self
                .store
                .children(&to.drivewsid)
                .and_then(|c| c.into_iter().find(|i| i.drivewsid == item.drivewsid))
                .unwrap_or_else(|| item.clone());
            self.change(self.remote.rename(&moved, newname), &[to])?;
        }
        Ok(())
    }

    fn rename(
        me: &Arc<Self>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
    ) -> Result<(), Errno> {
        let (from, to) = (
            me.item(parent).ok_or(Errno::ENOENT)?,
            me.item(newparent).ok_or(Errno::ENOENT)?,
        );
        let (ino, item) = me.child(parent, name)?;
        let newname = newname.to_str().ok_or(Errno::EINVAL)?;
        let target = me.child(newparent, OsStr::new(newname)).ok();
        if let Some((tino, titem)) = &target {
            if *tino == ino {
                return Ok(());
            }
            if titem.is_container() || item.is_container() {
                return Err(Errno::EEXIST);
            }
        }
        let pending = me.pending(ino);
        // Added and not yet sent, under another name or among other new files — a copy's
        // temporary name made its own — is all that can be renamed where only adding is.
        if me.add_only
            && (pending.is_none()
                || target
                    .as_ref()
                    .is_some_and(|(t, _)| me.pending(*t).is_none())
                || !me.remote.accepts(&to))
        {
            return Err(Errno::EROFS);
        }
        match (pending, target) {
            // A file written here taking another's name — an editor's save: what it replaces
            // (Apple's file, or another being written) goes, and Apple's into Recently Deleted
            // only once the new bytes are there (writes::send).
            (Some(p), Some((tino, titem))) => {
                let theirs = me.pending(tino);
                if let Some(w) = &me.writes {
                    w.items
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&tino);
                    w.drop_staging(tino);
                }
                let replaced = theirs.map_or(Some(titem), |t| t.replaces);
                // The file this one was copied from, if it was Apple's and is left behind by
                // the rename, goes as a rename would take it.
                if let Some(old) = p
                    .replaces
                    .as_ref()
                    .filter(|o| replaced.as_ref().is_none_or(|r| r.drivewsid != o.drivewsid))
                {
                    me.change(me.remote.delete(old), &[&from])?;
                }
                me.restage(ino, &p.name, newname)?;
                me.with_pending(ino, |now| {
                    now.folder = to.drivewsid.clone();
                    now.name = newname.to_string();
                    now.replaces = replaced;
                    now.dirty = true;
                    now.generation += 1;
                });
            }
            // Apple's file taking another's name: the other into Recently Deleted first.
            (None, Some((tino, titem))) => {
                match me.pending(tino) {
                    Some(t) => {
                        if let Some(w) = &me.writes {
                            w.items
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .remove(&tino);
                            w.drop_staging(tino);
                        }
                        if let Some(old) = t.replaces {
                            me.change(me.remote.delete(&old), &[&to])?;
                        }
                    }
                    None => me.change(me.remote.delete(&titem), &[&to])?,
                }
                me.move_and_rename(&item, &from, &to, name, newname)?;
            }
            // A file written here, to a free name: Apple's copy (if any) is moved and renamed
            // there, and what is waiting follows it.
            (Some(p), None) => {
                if let Some(old) = &p.replaces {
                    me.move_and_rename(old, &from, &to, name, newname)?;
                    let now = me
                        .store
                        .children(&to.drivewsid)
                        .and_then(|c| c.into_iter().find(|i| i.drivewsid == old.drivewsid));
                    me.with_pending(ino, |w| {
                        if let Some(n) = now {
                            // Its version moved with the rename; not a change made elsewhere.
                            w.replaces = Some(n);
                        }
                    });
                }
                me.restage(ino, &p.name, newname)?;
                me.with_pending(ino, |now| {
                    now.folder = to.drivewsid.clone();
                    now.name = newname.to_string();
                });
            }
            (None, None) => me.move_and_rename(&item, &from, &to, name, newname)?,
        }
        if let Some(w) = &me.writes {
            w.save();
        }
        if me.pending(ino).is_some_and(|p| p.open == 0 && p.dirty) {
            Self::schedule(me, ino, writes::SETTLE);
        }
        Ok(())
    }

    fn restage(&self, ino: u64, from: &str, to: &str) -> Result<(), Errno> {
        self.writes
            .as_ref()
            .ok_or(Errno::EROFS)?
            .rename_staging(ino, from, to)
            .map_err(|_| Errno::EIO)
    }
}

/// Why a change failed, said in a development build (see the app's quiet!): never a name.
fn quiet_note(e: &str) {
    if cfg!(debug_assertions) {
        eprintln!(
            "drive folder: a change was refused: {}",
            e.chars().take(120).collect::<String>()
        );
    }
}

impl Filesystem for DriveFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let inner = &self.0;
        let found = inner.child(parent.0, name);
        // A name not there in a listing gone stale — a file just saved on the phone, asked for
        // by its path — has its folder asked of Apple behind the answer, never in front of it:
        // file managers ask after names that are not there all the time (.hidden, desktop.ini),
        // and Apple's first answer can take seconds.
        if matches!(found, Err(Errno::ENOENT)) {
            if let Some(folder) = inner.item(parent.0) {
                inner.revalidate(&folder.drivewsid);
            }
        }
        match found {
            Ok((ino, item)) => reply.entry(&TTL, &inner.attr(ino, &item), Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.0.item(ino.0) {
            Some(item) => reply.attr(&TTL, &self.0.attr(ino.0, &item)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let inner = &self.0;
        let Some(item) = inner.item(ino.0) else {
            return reply.error(Errno::ENOENT);
        };
        // Only a file's length is Apple's to keep; modes and times are taken as they are.
        if size.is_some() && inner.read_only {
            return reply.error(Errno::EROFS);
        }
        if let (Some(size), false) = (size, item.is_container()) {
            if let Err(e) = inner.begin_writing(ino.0, &item, size > 0) {
                return reply.error(e);
            }
            let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) else {
                return reply.error(Errno::EROFS);
            };
            let ok = std::fs::OpenOptions::new()
                .write(true)
                .open(w.staging(ino.0, &p.name))
                .and_then(|f| f.set_len(size))
                .is_ok();
            if !ok {
                return reply.error(Errno::EIO);
            }
            inner.with_pending(ino.0, |p| {
                p.dirty = true;
                p.generation += 1;
                p.modified = SystemTime::now();
            });
            if inner.pending(ino.0).is_some_and(|p| p.open == 0) {
                Inner::schedule(inner, ino.0, writes::SETTLE);
            }
        }
        reply.attr(&TTL, &inner.attr(ino.0, &item));
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let inner = &self.0;
        let children = match inner.children(ino.0) {
            Ok(c) => c,
            Err(e) => return reply.error(e),
        };
        if offset == 0 {
            if let Some(folder) = inner.item(ino.0) {
                inner.fill_thumbnails(&folder, &children);
                inner.revalidate(&folder.drivewsid);
            }
        }
        let parent = inner
            .item(ino.0)
            .and_then(|i| i.parent_id)
            .and_then(|p| inner.inodes.lock().ok().and_then(|n| n.ino_of(&p)))
            .unwrap_or(1);
        let mut all: Vec<(u64, FileType, String)> = vec![
            (ino.0, FileType::Directory, ".".into()),
            (parent, FileType::Directory, "..".into()),
        ];
        all.extend(children.into_iter().map(|(name, i, item)| {
            (
                i,
                if item.is_container() {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                },
                name,
            )
        }));
        for (n, (i, kind, name)) in all
            .into_iter()
            .enumerate()
            .skip(usize::try_from(offset).unwrap_or(0))
        {
            if reply.add(INodeNo(i), n as u64 + 1, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&self, req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let inner = &self.0;
        let Some(item) = inner.item(ino.0) else {
            return reply.error(Errno::ENOENT);
        };
        if item.is_container() {
            return reply.error(Errno::EISDIR);
        }
        let writing = flags.acc_mode() != OpenAccMode::O_RDONLY;
        let truncate = flags.0 & libc::O_TRUNC != 0;
        if writing && (inner.writes.is_none() || inner.read_only) {
            return reply.error(Errno::EROFS);
        }
        if inner.pending(ino.0).is_none() {
            if writing {
                if let Err(e) = inner.begin_writing(ino.0, &item, !truncate) {
                    return reply.error(e);
                }
            } else {
                // A thumbnailer is not let download a file only to draw it small.
                let pid = req.pid();
                let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
                let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                if is_thumbnailer(&comm, &cmdline)
                    && !inner.cache.whole(&item)
                    && !inner.remote.is_local(&item)
                {
                    return reply.error(Errno::EACCES);
                }
                return reply.opened(FileHandle(0), FopenFlags::empty());
            }
        }
        if truncate {
            if let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) {
                let _ = std::fs::OpenOptions::new()
                    .write(true)
                    .open(w.staging(ino.0, &p.name))
                    .and_then(|f| f.set_len(0));
            }
        }
        inner.with_pending(ino.0, |p| {
            p.open += 1;
            if truncate {
                p.dirty = true;
                p.generation += 1;
                p.modified = SystemTime::now();
            }
        });
        // The kernel's page cache is not kept for a file being written: every read is ours.
        reply.opened(FileHandle(ino.0), FopenFlags::FOPEN_DIRECT_IO);
    }

    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        if self.0.read_only {
            return reply.error(Errno::EROFS);
        }
        let inner = &self.0;
        let Some(writes) = inner.writes.clone() else {
            return reply.error(Errno::EROFS);
        };
        let Some(folder) = inner.item(parent.0) else {
            return reply.error(Errno::ENOENT);
        };
        let Some(name) = name.to_str() else {
            return reply.error(Errno::EINVAL);
        };
        if inner.add_only && !inner.remote.accepts(&folder) {
            return reply.error(Errno::EROFS);
        }
        if let Ok((ino, item)) = inner.child(parent.0, OsStr::new(name)) {
            if flags & libc::O_EXCL != 0 {
                return reply.error(Errno::EEXIST);
            }
            if item.is_container() {
                return reply.error(Errno::EISDIR);
            }
            let truncate = flags & libc::O_TRUNC != 0;
            if inner.pending(ino).is_none() {
                if let Err(e) = inner.begin_writing(ino, &item, !truncate) {
                    return reply.error(e);
                }
            }
            if truncate {
                if let Some(p) = inner.pending(ino) {
                    let _ = std::fs::OpenOptions::new()
                        .write(true)
                        .open(writes.staging(ino, &p.name))
                        .and_then(|f| f.set_len(0));
                }
            }
            inner.with_pending(ino, |p| {
                p.open += 1;
                if truncate {
                    p.dirty = true;
                    p.generation += 1;
                }
            });
            let _ = req;
            return reply.created(
                &TTL,
                &inner.attr(ino, &item),
                Generation(0),
                FileHandle(ino),
                FopenFlags::FOPEN_DIRECT_IO,
            );
        }
        let ino = inner
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .local(&folder.drivewsid, name);
        let staged = writes.staging(ino, name);
        let made = staged
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::File::create(&staged));
        if let Err(e) = made {
            return reply.error(disk(e));
        }
        writes
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                ino,
                Pending {
                    folder: folder.drivewsid.clone(),
                    name: name.to_string(),
                    replaces: None,
                    open: 1,
                    dirty: true,
                    generation: 1,
                    modified: SystemTime::now(),
                    sender: false,
                    quiet_since: None,
                    sending: false,
                    failure: None,
                },
            );
        inner
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .place(ino, parent.0, name);
        writes.save();
        let Some(item) = inner.item(ino) else {
            return reply.error(Errno::EIO);
        };
        reply.created(
            &TTL,
            &inner.attr(ino, &item),
            Generation(0),
            FileHandle(ino),
            FopenFlags::FOPEN_DIRECT_IO,
        );
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let inner = &self.0;
        // A file being written reads what has been written.
        if let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) {
            let mut buf = vec![0u8; size as usize];
            return match std::fs::File::open(w.staging(ino.0, &p.name))
                .and_then(|f| f.read_at(&mut buf, offset))
            {
                Ok(n) => reply.data(&buf[..n]),
                Err(_) => reply.error(Errno::EIO),
            };
        }
        let Some(item) = inner.item(ino.0) else {
            return reply.error(Errno::ENOENT);
        };
        // Kept whole by the app already (a photo's original): read from there, no second copy.
        if let Some(path) = inner.remote.file(&item) {
            inner.rt.spawn(async move {
                let bytes = path.await.and_then(|p| {
                    let mut buf = vec![0u8; size as usize];
                    let n = std::fs::File::open(p)
                        .and_then(|f| f.read_at(&mut buf, offset))
                        .map_err(|e| e.to_string())?;
                    buf.truncate(n);
                    Ok(buf)
                });
                match bytes {
                    Ok(b) => reply.data(&b),
                    Err(_) => reply.error(Errno::EIO),
                }
            });
            return;
        }
        let (cache, remote) = (Arc::clone(&inner.cache), Arc::clone(&inner.remote));
        // Answered when the bytes are here, without holding up the folder's other requests.
        inner.rt.spawn(async move {
            match cache
                .read(remote.as_ref(), &item, offset, u64::from(size))
                .await
            {
                Ok(bytes) => reply.data(&bytes),
                Err(_) => reply.error(Errno::EIO),
            }
        });
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let inner = &self.0;
        let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) else {
            return reply.error(Errno::EBADF);
        };
        let written = std::fs::OpenOptions::new()
            .write(true)
            .open(w.staging(ino.0, &p.name))
            .and_then(|f| f.write_all_at(data, offset));
        if let Err(e) = written {
            return reply.error(disk(e));
        }
        inner.with_pending(ino.0, |p| {
            p.dirty = true;
            p.generation += 1;
            p.modified = SystemTime::now();
        });
        reply.written(u32::try_from(data.len()).unwrap_or(u32::MAX));
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock: LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        // On this computer's disk is as far as a sync can promise; Apple's copy follows.
        if let (Some(w), Some(p)) = (&self.0.writes, self.0.pending(ino.0)) {
            let _ = std::fs::File::open(w.staging(ino.0, &p.name)).and_then(|f| f.sync_all());
        }
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        _lock: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let inner = &self.0;
        let last = inner.with_pending(ino.0, |p| {
            p.open = p.open.saturating_sub(1);
            p.open == 0 && p.dirty
        });
        if last == Some(true) {
            if let Some(w) = &inner.writes {
                w.save();
            }
            Inner::schedule(inner, ino.0, writes::SETTLE);
        }
        reply.ok();
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        if self.0.read_only || self.0.add_only {
            return reply.error(Errno::EROFS);
        }
        let inner = &self.0;
        let Some(folder) = inner.item(parent.0) else {
            return reply.error(Errno::ENOENT);
        };
        let Some(name) = name.to_str() else {
            return reply.error(Errno::EINVAL);
        };
        if inner.child(parent.0, OsStr::new(name)).is_ok() {
            return reply.error(Errno::EEXIST);
        }
        match inner.change(inner.remote.mkdir(&folder, name), &[&folder]) {
            Ok(_) => match inner.child(parent.0, OsStr::new(name)) {
                Ok((ino, item)) => reply.entry(&TTL, &inner.attr(ino, &item), Generation(0)),
                Err(e) => reply.error(e),
            },
            Err(e) => reply.error(e),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        if self.0.read_only {
            return reply.error(Errno::EROFS);
        }
        match self.0.remove(parent.0, name, false) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        if self.0.read_only || self.0.add_only {
            return reply.error(Errno::EROFS);
        }
        match self.0.remove(parent.0, name, true) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        if self.0.read_only {
            return reply.error(Errno::EROFS);
        }
        match Inner::rename(&self.0, parent.0, name, newparent.0, newname) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        let inner = &self.0;
        // The account's own storage, asked at most once a minute: a file manager checks there
        // is room before copying in.
        let known = *inner
            .space
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let space = match known {
            Some((s, at)) if at.elapsed() < Duration::from_secs(60) => Some(s),
            _ => {
                let remote = Arc::clone(&inner.remote);
                let got = inner
                    .rt
                    .block_on(async move {
                        tokio::time::timeout(Duration::from_secs(5), remote.space()).await
                    })
                    .ok()
                    .and_then(Result::ok);
                if let Some(s) = got {
                    *inner
                        .space
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some((s, std::time::Instant::now()));
                }
                got.or(known.map(|(s, _)| s))
            }
        };
        let block = 4096u64;
        match (space, inner.writes.is_some()) {
            (Some((used, total)), true) => {
                let free = total.saturating_sub(used) / block;
                reply.statfs(total / block, free, free, 0, 0, 4096, 255, 4096);
            }
            _ => reply.statfs(0, 0, 0, 0, 0, 4096, 255, 4096),
        }
    }
}

/// The folder mounted at `at`, served on its own threads until the returned session is
/// dropped. A mount left behind by a crash ("Transport endpoint is not connected") is let go
/// of first.
///
/// # Errors
/// When the folder cannot be made or FUSE refuses the mount.
pub fn mount(fs: DriveFs, at: &Path) -> std::io::Result<fuser::BackgroundSession> {
    if let Err(e) = std::fs::metadata(at) {
        if e.raw_os_error() == Some(libc::ENOTCONN) {
            let _ = std::process::Command::new("fusermount3")
                .arg("-uz")
                .arg(at)
                .status();
        }
    }
    std::fs::create_dir_all(at)?;
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName("quince".into()),
        MountOption::Subtype("quince".into()),
        MountOption::NoSuid,
        MountOption::NoDev,
        MountOption::NoAtime,
        MountOption::DefaultPermissions,
    ];
    config.n_threads = Some(4);
    fuser::spawn_mount(fs, at, &config)
}
