//! iCloud Drive as a folder other apps can use, on demand.
//!
//! The platform-free engine: which item an inode is, what a folder holds, and a file's bytes
//! read in chunks into a sparse cache on disk — only the parts an app actually reads, each
//! fetched once (Apple's download links answer byte ranges; proven live). The platform's
//! own front end ([`fuse`] on Linux) asks it and never waits on the network for what the
//! library already knows. Design: docs/mount-design.md.

#[cfg(target_os = "linux")]
pub mod fuse;
pub mod writes;

use quince_core::drive::DriveItem;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A future the engine waits on, as the app's own async code gives it.
pub type Fut<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// Apple's side, as the app reaches it with the signed-in session.
pub trait Remote: Send + Sync + 'static {
    /// A folder's items, fetched from Apple (and kept in the library by the app).
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>>;
    /// `len` bytes of a file from `offset` (fewer only at its end).
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>>;
    /// A new folder in `parent`, as Apple made it. Read-only by default.
    fn mkdir(&self, _parent: &DriveItem, _name: &str) -> Fut<DriveItem> {
        Box::pin(async { Err("read-only".to_string()) })
    }
    /// An item given a new name in its folder.
    fn rename(&self, _item: &DriveItem, _name: &str) -> Fut<()> {
        Box::pin(async { Err("read-only".to_string()) })
    }
    /// An item moved into another folder, its name kept.
    fn move_to(&self, _item: &DriveItem, _folder: &DriveItem) -> Fut<()> {
        Box::pin(async { Err("read-only".to_string()) })
    }
    /// A file sent into `folder`, named after the file at `path`; Apple names it "name 2"
    /// when the name is taken. Answers the file as Apple filed it.
    fn upload(&self, _folder: &DriveItem, _path: &Path) -> Fut<DriveItem> {
        Box::pin(async { Err("read-only".to_string()) })
    }
    /// Whether new files may be written into `folder`. Every folder by default; a folder that
    /// is a view, not a place (the Photos folder's Favorites), says no.
    fn accepts(&self, _folder: &DriveItem) -> bool {
        true
    }
    /// An item deleted — into Apple's Recently Deleted, where it can be put back for 30 days.
    fn delete(&self, _item: &DriveItem) -> Fut<()> {
        Box::pin(async { Err("read-only".to_string()) })
    }
    /// Where a file's whole bytes are already kept on this computer by the app (fetched once if
    /// need be): read from there, not copied into the folder's own cache. `None`: no such
    /// place, the cache is used.
    fn file(&self, _item: &DriveItem) -> Option<Fut<PathBuf>> {
        None
    }
    /// Whether a file's bytes are on this computer already, without fetching them.
    fn is_local(&self, _item: &DriveItem) -> bool {
        false
    }
    /// The account's iCloud storage, `(used, total)` in bytes: a file manager checks there is
    /// room before it copies anything in.
    fn space(&self) -> Fut<(u64, u64)> {
        Box::pin(async { Err("unknown".to_string()) })
    }
    /// Apple's own pictures of a folder's files, by item id: for a file manager's thumbnails,
    /// so it never downloads a file to draw one. None by default.
    fn thumbnails(&self, _folder: &str) -> Fut<HashMap<String, Vec<u8>>> {
        Box::pin(async { Ok(HashMap::new()) })
    }
}

/// What the library already knows, read without the network.
pub trait Store: Send + Sync + 'static {
    /// A folder's items as last stored; `None` when it has never been listed.
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>>;
}

/// The root of iCloud Drive, as Apple names it.
pub const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

/// Inode numbers for Apple's items, the same for an item for as long as the folder is
/// mounted. 1 is the root, as FUSE requires.
#[derive(Default)]
pub struct Inodes {
    by_ino: HashMap<u64, DriveItem>,
    by_id: HashMap<String, u64>,
    /// Where each item is in the folder, as the file manager names it ("Work/Deck.key").
    paths: HashMap<u64, String>,
    next: u64,
}

impl Inodes {
    #[must_use]
    pub fn new() -> Self {
        let mut me = Self {
            next: 2,
            ..Self::default()
        };
        let root = DriveItem {
            drivewsid: ROOT.into(),
            parent_id: None,
            name: "iCloud Drive".into(),
            extension: None,
            kind: "FOLDER".into(),
            size: None,
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
        };
        me.by_ino.insert(1, root);
        me.by_id.insert(ROOT.into(), 1);
        me
    }

    /// The inode of an item, given one the first time it is seen; its details kept fresh.
    pub fn ino(&mut self, item: &DriveItem) -> u64 {
        if let Some(&ino) = self.by_id.get(&item.drivewsid) {
            if ino != 1 {
                self.by_ino.insert(ino, item.clone());
            }
            return ino;
        }
        let ino = self.next;
        self.next += 1;
        self.by_id.insert(item.drivewsid.clone(), ino);
        self.by_ino.insert(ino, item.clone());
        ino
    }

    /// Where an item is, from the folder's top, as last listed.
    #[must_use]
    pub fn path(&self, ino: u64) -> Option<&str> {
        if ino == 1 {
            return Some("");
        }
        self.paths.get(&ino).map(String::as_str)
    }

    /// Record where an item is: its folder's path and its name in it.
    pub fn place(&mut self, ino: u64, parent: u64, name: &str) {
        let at = match self.path(parent) {
            Some("") => name.to_string(),
            Some(p) => format!("{p}/{name}"),
            None => return,
        };
        self.paths.insert(ino, at);
    }

    /// An inode for a file made on this computer and not yet at Apple: its own id
    /// ("LOCAL::<inode>") until Apple files it.
    pub fn local(&mut self, folder: &str, name: &str) -> u64 {
        let ino = self.next;
        self.next += 1;
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), Some(e.to_string())),
            _ => (name.to_string(), None),
        };
        let item = DriveItem {
            drivewsid: format!("LOCAL::{ino}"),
            parent_id: Some(folder.to_string()),
            name: stem,
            extension: ext,
            kind: "FILE".into(),
            size: Some(0),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
        };
        self.by_id.insert(item.drivewsid.clone(), ino);
        self.by_ino.insert(ino, item);
        ino
    }

    /// An inode given to the item Apple filed for it (a local file sent, or a file replaced):
    /// the same inode, so a program holding it open carries on.
    pub fn bind(&mut self, ino: u64, item: &DriveItem) {
        if let Some(old) = self.by_ino.get(&ino).map(|i| i.drivewsid.clone()) {
            if old != item.drivewsid {
                self.by_id.remove(&old);
            }
        }
        if let Some(other) = self.by_id.insert(item.drivewsid.clone(), ino) {
            if other != ino {
                self.by_ino.remove(&other);
            }
        }
        self.by_ino.insert(ino, item.clone());
    }

    /// The inode an item already has, by Apple's id.
    #[must_use]
    pub fn ino_of(&self, id: &str) -> Option<u64> {
        self.by_id.get(id).copied()
    }

    #[must_use]
    pub fn item(&self, ino: u64) -> Option<&DriveItem> {
        self.by_ino.get(&ino)
    }
}

/// A folder's items as names a file manager shows: the name with its extension, NFC as Apple
/// keeps them. Two that would share a name (Apple's names differ by case or by a character a
/// path cannot hold) are told apart, never one hiding the other.
#[must_use]
pub fn named(items: &[DriveItem]) -> Vec<(String, DriveItem)> {
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        // "/" cannot be in a name on Linux, and NUL nowhere: Apple allows the first.
        let base: String = item
            .display_name()
            .chars()
            .map(|c| match c {
                '/' => '∕',
                '\0' => '�',
                c => c,
            })
            .collect();
        let base = if base == "." || base == ".." || base.is_empty() {
            format!("{base}_")
        } else {
            base
        };
        let n = taken.entry(base.clone()).or_insert(0);
        *n += 1;
        let name = if *n == 1 {
            base
        } else {
            // "Report (2).pdf": the second of two with the same name.
            match base.rsplit_once('.') {
                Some((stem, ext)) if !item.is_container() && !stem.is_empty() => {
                    format!("{stem} ({n}).{ext}")
                }
                _ => format!("{base} ({n})"),
            }
        };
        out.push((name, item.clone()));
    }
    out
}

/// How much of a file is fetched at a time: a picture or a page in one request, a film in a
/// few. Each chunk is fetched once and kept.
pub const CHUNK: u64 = 4 * 1024 * 1024;

/// A file's bytes kept on disk as they are read: a sparse file the file's own size, and which
/// of its chunks are there. Keyed by the item and its version (Apple's etag), so a file
/// changed on another device is never served from an older copy.
pub struct Cache {
    dir: PathBuf,
    have: Mutex<HashMap<PathBuf, Vec<bool>>>,
    fetching: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    /// The most disk it may use, in bytes (`u64::MAX`: no limit).
    budget: u64,
    /// Bytes fetched since it was last counted, to know when to count again.
    since: std::sync::atomic::AtomicU64,
}

/// A file read in the last few minutes is never let go of: it may well be open.
const IN_USE: std::time::Duration = std::time::Duration::from_secs(5 * 60);

impl Cache {
    /// # Errors
    /// When the folder cannot be made.
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            have: Mutex::new(HashMap::new()),
            fetching: Mutex::new(HashMap::new()),
            budget: u64::MAX,
            since: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// The most disk the cache may use. Past it, the files read longest ago are let go of —
    /// to fetch again if they are read again — down to four fifths of it (rclone's
    /// vfs-cache-max-size, least recently used first).
    #[must_use]
    pub fn with_budget(mut self, bytes: u64) -> Self {
        self.budget = bytes;
        self
    }

    /// Bytes on disk now: what the sparse files really take, not their full sizes.
    #[must_use]
    pub fn used(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::read_dir(&self.dir)
            .map(|d| {
                d.flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.blocks() * 512)
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Down to four fifths of the budget, the files read longest ago first; never one read in
    /// the last few minutes. Answers how many bytes were let go of.
    pub fn evict(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        let mut used = self.used();
        if used <= self.budget {
            return 0;
        }
        let target = self.budget / 5 * 4;
        let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = std::fs::read_dir(&self.dir)
            .map(|d| {
                d.flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .is_none_or(|x| x != "have" && x != "part")
                    })
                    .filter_map(|e| {
                        let m = e.metadata().ok()?;
                        let have = std::fs::metadata(e.path().with_extension("have"))
                            .map_or(0, |h| h.blocks() * 512);
                        Some((m.modified().ok()?, e.path(), m.blocks() * 512 + have))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by_key(|(t, ..)| *t);
        let mut freed = 0;
        for (when, path, bytes) in files {
            if used <= target {
                break;
            }
            if when.elapsed().map_or(true, |e| e < IN_USE) {
                continue;
            }
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("have"));
            self.have
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&path);
            used = used.saturating_sub(bytes);
            freed += bytes;
        }
        freed
    }

    fn path(&self, item: &DriveItem) -> PathBuf {
        let safe = |s: &str| -> String {
            s.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect()
        };
        self.dir.join(format!(
            "{}~{}",
            safe(&item.drivewsid),
            safe(item.etag.as_deref().unwrap_or("-"))
        ))
    }

    fn chunks(size: u64) -> usize {
        usize::try_from(size.div_ceil(CHUNK)).unwrap_or(0)
    }

    /// Which chunks are there, from memory or the file kept beside the bytes.
    fn present(&self, path: &Path, size: u64) -> Vec<bool> {
        let mut have = self
            .have
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        have.entry(path.to_path_buf())
            .or_insert_with(|| {
                let n = Self::chunks(size);
                let mut v = vec![false; n];
                if let Ok(bytes) = std::fs::read(path.with_extension("have")) {
                    for (i, b) in bytes.iter().take(n).enumerate() {
                        v[i] = *b == 1;
                    }
                }
                v
            })
            .clone()
    }

    fn mark(&self, path: &Path, chunk: usize, size: u64) -> std::io::Result<()> {
        let mut have = self
            .have
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let v = have
            .entry(path.to_path_buf())
            .or_insert_with(|| vec![false; Self::chunks(size)]);
        if chunk < v.len() {
            v[chunk] = true;
        }
        let bytes: Vec<u8> = v.iter().map(|&b| u8::from(b)).collect();
        // Written beside, after the bytes themselves: a chunk is never claimed before it is
        // on disk, so a crash between the two only costs fetching it again.
        let part = path.with_extension("have.part");
        std::fs::write(&part, bytes)?;
        std::fs::rename(&part, path.with_extension("have"))
    }

    /// Whether every byte of a file is on disk already.
    #[must_use]
    pub fn whole(&self, item: &DriveItem) -> bool {
        let size = u64::try_from(item.size.unwrap_or(0)).unwrap_or(0);
        size == 0 || self.present(&self.path(item), size).iter().all(|&b| b)
    }

    /// `len` bytes of a file from `offset`: from disk, each chunk missing fetched once.
    ///
    /// # Errors
    /// When Apple cannot be reached for a missing chunk, or the disk refuses.
    pub async fn read(
        &self,
        remote: &dyn Remote,
        item: &DriveItem,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, String> {
        let size = u64::try_from(item.size.unwrap_or(0)).unwrap_or(0);
        if offset >= size || len == 0 {
            return Ok(Vec::new());
        }
        let end = (offset + len).min(size);
        let path = self.path(item);
        let lock = {
            let mut f = self
                .fetching
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(f.entry(path.clone()).or_default())
        };
        {
            // One fetch at a time for a file: two readers of the same part wait for one fetch.
            let _one = lock.lock().await;
            let first = usize::try_from(offset / CHUNK).unwrap_or(0);
            let last = usize::try_from((end - 1) / CHUNK).unwrap_or(0);
            for chunk in first..=last {
                if self
                    .present(&path, size)
                    .get(chunk)
                    .copied()
                    .unwrap_or(false)
                {
                    continue;
                }
                let from = chunk as u64 * CHUNK;
                let want = CHUNK.min(size - from);
                let bytes = remote.read(item, from, want).await?;
                if bytes.len() as u64 != want {
                    return Err(format!("Apple sent {} of {want} bytes", bytes.len()));
                }
                let p = path.clone();
                tokio::task::spawn_blocking(move || put(&p, from, &bytes, size))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                self.mark(&path, chunk, size).map_err(|e| e.to_string())?;
                // Counted again only once a good part of the budget has come in since.
                let came = self
                    .since
                    .fetch_add(want, std::sync::atomic::Ordering::Relaxed)
                    + want;
                if self.budget != u64::MAX && came > self.budget / 10 {
                    self.since.store(0, std::sync::atomic::Ordering::Relaxed);
                    // A walk of one folder's names and sizes: quick, and only now and then.
                    self.evict();
                }
            }
        }
        let p = path.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = get(&p, offset, end - offset)?;
            // Read now: the last to be let go of (its time moved at most once a minute).
            if let Ok(f) = std::fs::File::options().write(true).open(&p) {
                let stale = f
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok());
                if stale.is_some_and(|e| e > std::time::Duration::from_secs(60)) {
                    let _ = f.set_modified(std::time::SystemTime::now());
                }
            }
            Ok::<_, std::io::Error>(bytes)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
    }
}

/// Bytes written at `at` in the sparse file, made its full size first.
fn put(path: &Path, at: u64, bytes: &[u8], size: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    if file.metadata()?.len() != size {
        file.set_len(size)?;
    }
    file.write_all_at(bytes, at)?;
    file.sync_data()
}

fn get(path: &Path, at: u64, len: u64) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::open(path)?;
    let mut out = vec![0u8; usize::try_from(len).unwrap_or(0)];
    file.read_exact_at(&mut out, at)?;
    Ok(out)
}

/// Whether the program asking is a file manager's thumbnailer. Reading a whole file only to
/// draw a small picture of it would download every file a folder shows: such a read of a
/// file not on this computer is refused, and the file manager draws its plain icon instead.
/// (GLib treats a FUSE folder as local — glocalfile.c's is_remote_fs_type — so GNOME Files
/// thumbnails everything in it.)
///
/// Named exactly, never broadly: a viewer opening a picture (Loupe's glycin loaders), a
/// Flatpak app (bwrap) or KDE copying a file (a kioworker) must still read it. KDE's thumbnail
/// worker is a kioworker running thumbnail.so, told apart by its command line.
#[must_use]
pub fn is_thumbnailer(comm: &str, cmdline: &str) -> bool {
    let comm = comm.trim();
    let named = [
        "gnome-thumbnail",
        "totem-video-thu",
        "evince-thumbnai",
        "ffmpegthumbnail",
        "gst-video-thumb",
        "gdk-pixbuf-thum",
        "tumblerd",
        "papers-thumbnai",
    ]
    .iter()
    .any(|t| comm.starts_with(t));
    named || (comm == "kioworker" && cmdline.contains("thumbnail"))
}

/// Apple's pictures of a folder's files written into the freedesktop thumbnail cache, where
/// GNOME Files, Dolphin and every other file manager look before reading a file to draw it:
/// thumbnails appear at once, and nothing is downloaded for them. Each is named by the MD5 of
/// its file's URI and carries the file's time, which the file manager checks; one already
/// there for the same version is left alone. `mount` is where the folder is mounted, `cache`
/// the thumbnail cache's root (~/.cache/thumbnails).
pub fn write_thumbnails(
    mount: &Path,
    cache: &Path,
    files: &[(String, DriveItem)],
    pictures: &HashMap<String, Vec<u8>>,
) {
    use md5::Digest;
    use std::os::unix::fs::PermissionsExt;
    for (path, item) in files {
        let Some(picture) = pictures.get(&item.drivewsid) else {
            continue;
        };
        let uri = quince_media::file_uri(&mount.join(path));
        let mtime = item.modified_at.unwrap_or(0);
        let name = format!("{:x}.png", md5::Md5::digest(uri.as_bytes()));
        for (dir, px) in [("normal", 128), ("large", 256), ("x-large", 512)] {
            let at = cache.join(dir).join(&name);
            if std::fs::read(&at)
                .ok()
                .and_then(|b| quince_media::thumbnail_mtime(&b))
                == Some(mtime)
            {
                continue;
            }
            let Ok(png) = quince_media::freedesktop_thumbnail(picture, px, &uri, mtime) else {
                continue;
            };
            let _ = std::fs::create_dir_all(cache.join(dir));
            // As the specification asks: private to its owner, and never seen half-written.
            let part = at.with_extension("png.part");
            if std::fs::write(&part, &png).is_ok() {
                let _ = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600));
                let _ = std::fs::rename(&part, &at);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        id: &str,
        name: &str,
        ext: Option<&str>,
        kind: &str,
        size: i64,
        etag: &str,
    ) -> DriveItem {
        DriveItem {
            drivewsid: id.into(),
            parent_id: Some(ROOT.into()),
            name: name.into(),
            extension: ext.map(Into::into),
            kind: kind.into(),
            size: Some(size),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: Some(etag.into()),
        }
    }

    #[test]
    fn names_are_never_lost_to_each_other() {
        let items = [
            item("a", "Report", Some("pdf"), "FILE", 1, "e"),
            item("b", "Report", Some("pdf"), "FILE", 1, "e"),
            item("c", "a/b", None, "FOLDER", 0, "e"),
            item("d", "..", None, "FOLDER", 0, "e"),
        ];
        let names: Vec<String> = named(&items).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Report.pdf", "Report (2).pdf", "a∕b", ".._"]);
    }

    #[test]
    fn inodes_are_stable() {
        let mut inodes = Inodes::new();
        let a = item("FILE::x::1", "a", None, "FILE", 1, "e");
        let first = inodes.ino(&a);
        assert_eq!(inodes.ino(&a), first);
        assert_ne!(first, 1);
        assert_eq!(inodes.item(1).map(|i| i.drivewsid.as_str()), Some(ROOT));
    }

    struct Bytes(Vec<u8>, Mutex<Vec<(u64, u64)>>);
    impl Remote for Bytes {
        fn list(&self, _: &str) -> Fut<Vec<DriveItem>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn read(&self, _: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
            self.1.lock().unwrap().push((offset, len));
            let out = self.0
                [usize::try_from(offset).unwrap()..usize::try_from(offset + len).unwrap()]
                .to_vec();
            Box::pin(async move { Ok(out) })
        }
    }

    /// Only the chunks read are fetched, each once; the bytes are the file's; a new version
    /// of the file is never served from the old one's copy.
    #[tokio::test]
    async fn reads_fetch_each_chunk_once() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path()).unwrap();
        let size = CHUNK * 2 + 100;
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let remote = Bytes(data.clone(), Mutex::new(Vec::new()));
        let f = item(
            "FILE::x::1",
            "film",
            Some("mov"),
            "FILE",
            i64::try_from(size).unwrap(),
            "v1",
        );
        let got = cache.read(&remote, &f, CHUNK - 10, 20).await.unwrap();
        assert_eq!(
            got,
            data[usize::try_from(CHUNK - 10).unwrap()..usize::try_from(CHUNK + 10).unwrap()]
        );
        assert_eq!(
            remote.1.lock().unwrap().clone(),
            vec![(0, CHUNK), (CHUNK, CHUNK)],
            "the two chunks it spans"
        );
        cache.read(&remote, &f, 5, 5).await.unwrap();
        assert_eq!(remote.1.lock().unwrap().len(), 2, "kept, not fetched again");
        let tail = cache.read(&remote, &f, size - 50, 500).await.unwrap();
        assert_eq!(tail.len(), 50, "cut at the end of the file");
        assert!(cache.whole(&f));
        // Kept across a restart: a new cache over the same folder knows what is there.
        let again = Cache::new(dir.path()).unwrap();
        assert!(again.whole(&f));
        // Changed on another device: a new version is fetched afresh.
        let f2 = item(
            "FILE::x::1",
            "film",
            Some("mov"),
            "FILE",
            i64::try_from(size).unwrap(),
            "v2",
        );
        assert!(!again.whole(&f2));
    }

    /// Past its budget, the cache lets go of the files read longest ago — never one just read
    /// — and one let go of is fetched again when read again.
    #[tokio::test]
    async fn the_cache_keeps_to_its_budget() {
        let dir = tempfile::tempdir().unwrap();
        // Room for about two whole files.
        let cache = Cache::new(dir.path())
            .unwrap()
            .with_budget(CHUNK * 2 + CHUNK / 2);
        let data: Vec<u8> = (0..CHUNK).map(|i| (i % 249) as u8).collect();
        let remote = Bytes(data.clone(), Mutex::new(Vec::new()));
        let files: Vec<DriveItem> = (0..4)
            .map(|n| {
                item(
                    &format!("FILE::x::{n}"),
                    "f",
                    Some("bin"),
                    "FILE",
                    i64::try_from(CHUNK).unwrap(),
                    "v1",
                )
            })
            .collect();
        for (n, f) in files.iter().enumerate() {
            cache.read(&remote, f, 0, CHUNK).await.unwrap();
            // Read long ago, oldest first — as if over days.
            let p = cache.path(f);
            let ago = std::time::SystemTime::now()
                - std::time::Duration::from_secs(3600 * (10 - n as u64));
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(ago)
                .unwrap();
        }
        // Let go of as the reads came in: kept to its budget all along.
        cache.evict();
        assert!(
            cache.used() <= CHUNK * 2 + CHUNK / 2,
            "kept to its budget: {} used",
            cache.used()
        );
        assert!(
            !cache.whole(&files[0]) && !cache.whole(&files[1]) && cache.whole(&files[3]),
            "the oldest went, the newest stayed"
        );
        let fetched = remote.1.lock().unwrap().len();
        assert_eq!(
            cache.read(&remote, &files[0], 0, 10).await.unwrap(),
            data[..10],
            "fetched again when read again"
        );
        assert_eq!(remote.1.lock().unwrap().len(), fetched + 1);
        // One read just now is never let go of, whatever the budget.
        let tight = Cache::new(dir.path()).unwrap().with_budget(1);
        tight.evict();
        assert!(tight.whole(&files[0]), "just read: kept");
    }

    #[test]
    fn thumbnailers_are_known() {
        assert!(is_thumbnailer("gnome-thumbnail", ""));
        assert!(is_thumbnailer("totem-video-thu\n", ""));
        assert!(is_thumbnailer(
            "kioworker",
            "/usr/lib/kf6/kioworker /usr/lib/qt6/plugins/kf6/kio/thumbnail.so"
        ));
        assert!(!is_thumbnailer(
            "kioworker",
            "/usr/lib/kf6/kioworker /usr/lib/qt6/plugins/kf6/kio/file.so"
        ));
        assert!(
            !is_thumbnailer("glycin-image-rs", ""),
            "a viewer opening a picture"
        );
        assert!(!is_thumbnailer("libreoffice", ""));
    }
}
