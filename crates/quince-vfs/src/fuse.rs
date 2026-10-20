//! The engine as a Linux folder, through FUSE (fuser).
//!
//! Read-only for now. Names, sizes and dates come from the library, never the network, so a
//! file manager browsing is never kept waiting; a folder never opened before is fetched once.
//! A file's bytes come from the cache, each part fetched from Apple the first time it is read.

use crate::{is_thumbnailer, named, Cache, Inodes, Remote, Store};
use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
    LockOwner, MountOption, OpenAccMode, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEntry, ReplyOpen, ReplyStatfs, Request,
};
use quince_core::drive::DriveItem;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

/// How long the kernel may keep what it was told about a name or a file. Short, as rclone's
/// is: a file changed on another device must not be read with its old length.
const TTL: Duration = Duration::from_secs(1);

/// How long a folder never opened before may take to come from Apple before the file manager
/// is told it could not be read (and can ask again).
const LIST_WAIT: Duration = Duration::from_secs(30);

pub struct DriveFs {
    remote: Arc<dyn Remote>,
    store: Arc<dyn Store>,
    cache: Arc<Cache>,
    inodes: Mutex<Inodes>,
    rt: tokio::runtime::Handle,
    uid: u32,
    gid: u32,
}

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
        Self {
            remote,
            store,
            cache,
            inodes: Mutex::new(Inodes::new()),
            rt,
            uid,
            gid,
        }
    }

    fn item(&self, ino: INodeNo) -> Option<DriveItem> {
        self.inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .item(ino.0)
            .cloned()
    }

    fn attr(&self, ino: u64, item: &DriveItem) -> FileAttr {
        let secs = |s: Option<i64>| {
            UNIX_EPOCH + Duration::from_secs(u64::try_from(s.unwrap_or(0)).unwrap_or(0))
        };
        let dir = item.is_container();
        let size = if dir {
            0
        } else {
            u64::try_from(item.size.unwrap_or(0)).unwrap_or(0)
        };
        FileAttr {
            ino: INodeNo(ino),
            size,
            blocks: size.div_ceil(512),
            atime: secs(item.modified_at),
            mtime: secs(item.modified_at),
            ctime: secs(item.modified_at),
            crtime: secs(item.created_at.or(item.modified_at)),
            kind: if dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm: if dir { 0o555 } else { 0o444 },
            nlink: if dir { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// A folder's items with their names and inodes: from the library, or — never opened
    /// before — from Apple, once.
    fn children(&self, ino: INodeNo) -> Result<Vec<(String, u64, DriveItem)>, Errno> {
        let item = self.item(ino).ok_or(Errno::ENOENT)?;
        if !item.is_container() {
            return Err(Errno::ENOTDIR);
        }
        let stored = self.store.children(&item.drivewsid);
        // A folder Apple says has things in it, with nothing stored, was never listed.
        let listed = stored.filter(|s| !s.is_empty() || item.child_count == Some(0));
        let items = match listed {
            Some(items) => items,
            None => {
                let remote = Arc::clone(&self.remote);
                let id = item.drivewsid.clone();
                self.rt
                    .block_on(
                        async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await },
                    )
                    .map_err(|_| Errno::ETIMEDOUT)?
                    .map_err(|_| Errno::EIO)?
            }
        };
        let mut inodes = self
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(named(&items)
            .into_iter()
            .map(|(name, it)| (name, inodes.ino(&it), it))
            .collect())
    }
}

impl Filesystem for DriveFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        match self.children(parent) {
            Ok(children) => match children.into_iter().find(|(n, ..)| OsStr::new(n) == name) {
                Some((_, ino, item)) => reply.entry(&TTL, &self.attr(ino, &item), Generation(0)),
                None => reply.error(Errno::ENOENT),
            },
            Err(e) => reply.error(e),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.item(ino) {
            Some(item) => reply.attr(&TTL, &self.attr(ino.0, &item)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let children = match self.children(ino) {
            Ok(c) => c,
            Err(e) => return reply.error(e),
        };
        let parent = self
            .item(ino)
            .and_then(|i| i.parent_id)
            .and_then(|p| self.inodes.lock().ok().and_then(|n| n.ino_of(&p)))
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
        let Some(item) = self.item(ino) else {
            return reply.error(Errno::ENOENT);
        };
        if item.is_container() {
            return reply.error(Errno::EISDIR);
        }
        if flags.acc_mode() != OpenAccMode::O_RDONLY {
            return reply.error(Errno::EROFS);
        }
        // A thumbnailer is not let download a file only to draw it small (see is_thumbnailer).
        let pid = req.pid();
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        if is_thumbnailer(&comm, &cmdline) && !self.cache.whole(&item) {
            return reply.error(Errno::EACCES);
        }
        reply.opened(FileHandle(0), FopenFlags::empty());
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
        let Some(item) = self.item(ino) else {
            return reply.error(Errno::ENOENT);
        };
        let (cache, remote) = (Arc::clone(&self.cache), Arc::clone(&self.remote));
        // Answered when the bytes are here, without holding up the folder's other requests.
        self.rt.spawn(async move {
            match cache
                .read(remote.as_ref(), &item, offset, u64::from(size))
                .await
            {
                Ok(bytes) => reply.data(&bytes),
                Err(_) => reply.error(Errno::EIO),
            }
        });
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        // Read-only: nothing free to write into, as a mounted disc says.
        reply.statfs(0, 0, 0, 0, 0, 4096, 255, 4096);
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
        MountOption::RO,
        MountOption::NoSuid,
        MountOption::NoDev,
        MountOption::NoAtime,
        MountOption::DefaultPermissions,
    ];
    config.n_threads = Some(4);
    fuser::spawn_mount(fs, at, &config)
}
