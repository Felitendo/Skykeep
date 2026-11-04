//! The library as every command shares it: one connection that writes, behind one lock, and
//! read-only connections beside it for the screens.
//!
//! A sync pass stores what it fetched with the library held: a full Notes or Photos pass, or a
//! first sync, holds it for seconds. Every screen used to read through that same lock, so
//! Notes, Reminders and Contacts drew nothing — and an autosave queued — until the pass was in.
//! WAL lets a second connection read what was committed while the writer works, so a read
//! goes to one of those instead whenever only a pass stands in its way.
//!
//! Only a pass. A change made in the window (a note saved, a reminder ticked) still comes
//! first: while anything other than a pass holds the lock or waits for it, a read waits too,
//! in turn, exactly as it always did — so a read asked for after a save never shows the
//! library from before it. A pass is one transaction (`Library::atomically`), so a reader sees
//! all of it or none of it, never half.

use quince_core::Library;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, MutexGuard};

/// The most read-only connections kept open between reads. More are opened when more reads
/// run at once; past this they are closed after use rather than kept.
const KEPT_READERS: usize = 4;

/// The shared library: the connection that writes, and the readers beside it.
pub struct LibraryLock {
    library: Mutex<Library>,
    /// Holders of the lock, and those waiting for it, that are not a sync pass: while there is
    /// one, a read waits its turn on the lock rather than reading beside it.
    others: AtomicUsize,
    /// Where the library's file is; `None` (an in-memory library, in tests) reads through the
    /// lock alone.
    dir: Option<PathBuf>,
    readers: std::sync::Mutex<Vec<Reader>>,
}

/// A read-only connection, and which file it was opened on: an account's library set aside
/// for another's moves the file under it, and a reader on the old one is not reused.
pub struct Reader {
    id: (u64, u64),
    library: Library,
}

impl Reader {
    /// A read-only connection on the library under `dir`, if it can be had.
    pub fn open(dir: &Path) -> Option<Self> {
        let id = identity(dir)?;
        let library = Library::open_reader(dir).ok()?;
        Some(Self { id, library })
    }

    /// Still on the file that is the library now.
    pub fn current(&self, dir: &Path) -> bool {
        identity(dir) == Some(self.id)
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    /// A number that moves each time another connection commits a change to the library, and
    /// only then (SQLite's `data_version`): what this reader last read is still what is there
    /// for as long as it stands.
    pub fn data_version(&self) -> Option<i64> {
        self.library
            .conn()
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .ok()
    }
}

fn identity(dir: &Path) -> Option<(u64, u64)> {
    let file = dir.join("library.db");
    std::fs::metadata(&file)
        .ok()
        .map(|m| crate::file_identity(&m))
}

/// The library held, as `tokio::sync::MutexGuard` holds it; counted, while it is held or
/// waited for, as something a read has to wait behind.
pub struct Held<'a> {
    guard: MutexGuard<'a, Library>,
    _turn: Turn<'a>,
}

impl Deref for Held<'_> {
    type Target = Library;
    fn deref(&self) -> &Library {
        &self.guard
    }
}

impl DerefMut for Held<'_> {
    fn deref_mut(&mut self) -> &mut Library {
        &mut self.guard
    }
}

/// One holder or waiter counted for as long as it lives, however it ends — dropped while still
/// waiting (its task cancelled) included.
struct Turn<'a>(&'a AtomicUsize);

impl<'a> Turn<'a> {
    fn take(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(count)
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl LibraryLock {
    pub fn new(library: Library, dir: Option<PathBuf>) -> Self {
        Self {
            library: Mutex::new(library),
            others: AtomicUsize::new(0),
            dir,
            readers: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The library, to change it or to read it in turn with changes.
    pub async fn lock(&self) -> Held<'_> {
        let turn = Turn::take(&self.others);
        let guard = self.library.lock().await;
        Held { guard, _turn: turn }
    }

    /// [`Self::lock`], from a thread that is not one of the runtime's (a folder's own threads).
    pub fn blocking_lock(&self) -> Held<'_> {
        let turn = Turn::take(&self.others);
        let guard = self.library.blocking_lock();
        Held { guard, _turn: turn }
    }

    pub fn try_lock(&self) -> Result<Held<'_>, tokio::sync::TryLockError> {
        let turn = Turn::take(&self.others);
        let guard = self.library.try_lock()?;
        Ok(Held { guard, _turn: turn })
    }

    /// The library, for a sync pass to store what it fetched as one transaction: reads carry on
    /// beside it on their own connections, seeing the library as it was until the pass is in.
    /// Only for a store made through `Library::atomically`, so no reader sees half of one.
    pub async fn lock_for_pass(&self) -> MutexGuard<'_, Library> {
        self.library.lock().await
    }

    /// Read the library: beside a pass on a read-only connection, or — while a change of the
    /// window's is held or waiting, or no reader can be had — through the lock, in turn.
    ///
    /// A read that fails on a reader is asked again through the lock, so a reader never makes
    /// a read fail that would not have failed before.
    pub async fn read<T>(
        &self,
        read: impl Fn(&Library) -> quince_core::Result<T>,
    ) -> quince_core::Result<T> {
        if let Some(Ok(found)) = self.beside(&read) {
            return Ok(found);
        }
        read(&*self.lock().await)
    }

    /// [`Self::read`], from a thread that is not one of the runtime's.
    pub fn blocking_read<T>(
        &self,
        read: impl Fn(&Library) -> quince_core::Result<T>,
    ) -> quince_core::Result<T> {
        if let Some(Ok(found)) = self.beside(&read) {
            return Ok(found);
        }
        read(&self.blocking_lock())
    }

    /// The read on a reader, when nothing but a pass stands in its way; `None` when it has to
    /// wait its turn.
    fn beside<T>(
        &self,
        read: &impl Fn(&Library) -> quince_core::Result<T>,
    ) -> Option<quince_core::Result<T>> {
        if self.others.load(Ordering::Acquire) != 0 {
            return None;
        }
        let dir = self.dir.as_deref()?;
        let reader = self.take_reader(dir)?;
        let found = read(reader.library());
        self.give_back(reader);
        Some(found)
    }

    fn take_reader(&self, dir: &Path) -> Option<Reader> {
        let kept = {
            let mut readers = self
                .readers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            readers.pop()
        };
        match kept {
            Some(r) if r.current(dir) => Some(r),
            // On a file that is not the library any more: let go, and a fresh one opened.
            _ => Reader::open(dir),
        }
    }

    fn give_back(&self, reader: Reader) {
        // Where an open file cannot be moved (Windows), none is kept open between reads: an
        // account's library set aside would otherwise fail to move.
        if cfg!(not(unix)) {
            return;
        }
        let mut readers = self
            .readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if readers.len() < KEPT_READERS {
            readers.push(reader);
        }
    }

    /// Let go of every kept reader: before the library's files are moved.
    pub fn forget_readers(&self) {
        self.readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn library(dir: &Path) -> LibraryLock {
        LibraryLock::new(Library::open(dir).unwrap(), Some(dir.to_path_buf()))
    }

    fn state(l: &Library) -> quince_core::Result<Option<String>> {
        l.state("k")
    }

    /// A read while a pass holds the library answers at once, with the library as it was
    /// before the pass — never waiting for it, and never half of it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_read_never_waits_on_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let shared = Arc::new(library(dir.path()));
        shared.lock().await.set_state("k", "before").unwrap();

        let mut pass = shared.lock_for_pass().await;
        pass.conn().execute_batch("SAVEPOINT p").unwrap();
        pass.set_state("k", "during").unwrap();

        let read = tokio::time::timeout(Duration::from_secs(2), shared.read(state))
            .await
            .expect("the read waited for the pass")
            .unwrap();
        assert_eq!(read.as_deref(), Some("before"));

        pass.conn().execute_batch("RELEASE p").unwrap();
        drop(pass);
        assert_eq!(shared.read(state).await.unwrap().as_deref(), Some("during"));
    }

    /// A change of the window's waiting behind a pass keeps its turn: a read asked for after
    /// it waits too, and sees it, as it did when everything read through the lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_read_after_a_change_sees_the_change() {
        let dir = tempfile::tempdir().unwrap();
        let shared = Arc::new(library(dir.path()));
        shared.lock().await.set_state("k", "before").unwrap();

        let pass = shared.lock_for_pass().await;
        let writer = {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                shared.lock().await.set_state("k", "saved").unwrap();
            })
        };
        // The change is queued behind the pass.
        while shared.others.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
        let reader = {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move { shared.read(state).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!reader.is_finished(), "the read went ahead of the change");
        drop(pass);
        writer.await.unwrap();
        assert_eq!(reader.await.unwrap().as_deref(), Some("saved"));
    }

    /// A reader on a file that is no longer the library (an account's set aside, another's put
    /// in its place) is not used: the read is of the library that is there now.
    #[tokio::test]
    async fn a_reader_follows_the_library_to_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let shared = library(dir.path());
        shared.lock().await.set_state("k", "first").unwrap();
        assert_eq!(shared.read(state).await.unwrap().as_deref(), Some("first"));

        let elsewhere = tempfile::tempdir().unwrap();
        {
            let mut held = shared.lock().await;
            *held = Library::in_memory().unwrap();
            for name in ["library.db", "library.db-wal", "library.db-shm"] {
                let _ = std::fs::rename(dir.path().join(name), elsewhere.path().join(name));
            }
            *held = Library::open(dir.path()).unwrap();
            held.set_state("k", "second").unwrap();
        }
        assert_eq!(shared.read(state).await.unwrap().as_deref(), Some("second"));
    }

    /// The reader's version moves when the writer commits, and stands while it does not: the
    /// Drive folder keeps a listing for exactly as long as it stands.
    #[test]
    fn a_readers_version_moves_with_each_commit_and_only_then() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Library::open(dir.path()).unwrap();
        let reader = Reader::open(dir.path()).unwrap();
        let first = reader.data_version().unwrap();
        assert_eq!(reader.data_version(), Some(first), "nothing committed");
        writer.set_state("k", "v").unwrap();
        let second = reader.data_version().unwrap();
        assert_ne!(second, first, "a commit moves it");
        assert_eq!(reader.data_version(), Some(second));
        // A pass's whole transaction moves it once it is in, not before.
        writer
            .atomically(|l| {
                l.set_state("k", "w")?;
                assert_eq!(reader.data_version(), Some(second), "not while it is open");
                Ok::<_, quince_core::Error>(())
            })
            .unwrap();
        assert_ne!(reader.data_version(), Some(second));
    }

    use std::time::Duration;
}

/// The one Skykeep that has this library, as long as it runs: an exclusive lock on
/// `skykeep.lock` beside it (`flock` on Linux, a file opened shared with nobody on Windows), let
/// go of by the system however the process ends.
///
/// The single-instance plugin finds a running Skykeep on the session bus, and with no session
/// bus (over SSH, after `su`, some kiosks) it found none and a second one ran: both sending
/// what waits for Apple — the same change twice — both mounting the folders, and each one's
/// writes failing on the other's lock. `None`: another Skykeep holds it.
pub fn hold(dir: &Path) -> std::io::Result<Option<std::fs::File>> {
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    // Windows: opened shared with nobody, which is the lock — a second open is refused with a
    // sharing violation until this handle closes.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = match options.open(dir.join("skykeep.lock")) {
        Ok(file) => file,
        #[cfg(windows)]
        Err(e) if e.raw_os_error() == Some(32) => return Ok(None),
        Err(e) => return Err(e),
    };
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: flock on a descriptor this function owns; nothing else is touched.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            return if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Ok(None)
            } else {
                Err(e)
            };
        }
    }
    Ok(Some(file))
}

/// What keeps a second Skykeep off this library for as long as this one runs.
#[derive(Debug)]
pub enum Hold {
    /// The lock on `skykeep.lock` ([`hold`]): held by being kept open, never read.
    Locked(#[allow(dead_code)] std::fs::File),
    /// Where no lock can be taken: `skykeep.pid`, made only if no other file of that name is
    /// there ([`mark`]).
    Marked(PathBuf),
}

/// [`hold`], and where the folder's disk refuses locks (a network share without them),
/// [`mark`] instead: never a Skykeep running beside another on one library because neither
/// could tell. `Ok(None)`: another Skykeep has it. `Err`: neither could be had, said in words
/// to show before quitting.
pub fn hold_or_mark(dir: &Path) -> Result<Option<Hold>, String> {
    match hold(dir) {
        Ok(Some(lock)) => Ok(Some(Hold::Locked(lock))),
        Ok(None) => Ok(None),
        Err(why) => mark(dir).map_err(|e| {
            format!(
                "Skykeep could not make sure it is the only copy using {}: the disk refused a \
                 lock ({why}), and the file that stands in for one could not be made ({e}).",
                dir.display()
            )
        }),
    }
}

/// `skykeep.pid` beside the library, created only if no file of that name exists, holding this
/// process's id: the lock where the disk takes none. One left by a Skykeep that ended without
/// letting go — its process gone, or another program's now — is taken over; one a running
/// Skykeep made answers `None`.
pub fn mark(dir: &Path) -> std::io::Result<Option<Hold>> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = dir.join("skykeep.pid");
    let me = std::process::id();
    for _ in 0..3 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(me.to_string().as_bytes())?;
                file.sync_all()?;
                drop(file);
                // Two starting at once over a mark left behind may both have cleared it; the
                // one whose id is in it now is the one that runs.
                return Ok(match marked_by(&path) {
                    Some(pid) if pid == me => Some(Hold::Marked(path)),
                    _ => None,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let by = marked_by(&path).filter(|pid| *pid != me);
                if by.is_some_and(is_skykeep) {
                    return Ok(None);
                }
                // Where whose it is cannot be asked, it is the person's to say.
                if by.is_some() && !cfg!(target_os = "linux") {
                    return Err(std::io::Error::other(format!(
                        "{} is there. If no other Skykeep is running, remove it and open \
                         Skykeep again",
                        path.display()
                    )));
                }
                // Left behind: let go of, and made again as this one's.
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("skykeep.pid kept coming back"))
}

fn marked_by(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Whether process `pid` is running and is a Skykeep: the same program as this one. Asked of
/// Linux's `/proc`; elsewhere it cannot be told, and `false` leaves it to [`mark`] to ask.
/// Its program file, or (one updated while it ran, its file replaced) its name.
fn is_skykeep(pid: u32) -> bool {
    let exe = |p: &str| std::fs::read_link(format!("/proc/{p}/exe")).ok();
    let name = |p: &str| std::fs::read_to_string(format!("/proc/{p}/comm")).ok();
    let theirs = pid.to_string();
    let same_file = matches!((exe(&theirs), exe("self")), (Some(a), Some(b)) if a == b);
    let same_name = matches!((name(&theirs), name("self")), (Some(a), Some(b)) if a == b);
    same_file || same_name
}

impl Drop for Hold {
    fn drop(&mut self) {
        if let Hold::Marked(path) = self {
            if marked_by(path) == Some(std::process::id()) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod hold_tests {
    use super::*;

    /// Where the disk takes no lock: the mark stands in. Made by this Skykeep, a second is
    /// told one is running; left by one that ended (its process gone), it is taken over; let
    /// go of, it is gone.
    #[cfg(target_os = "linux")]
    #[test]
    fn where_no_lock_can_be_taken_a_mark_stands_in() {
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("skykeep.pid");
        // A running Skykeep's mark (this test's own process stands in for one).
        let parent = std::os::unix::process::parent_id();
        std::fs::write(&pid, std::process::id().to_string()).unwrap();
        // Ours: taken again as ours.
        let held = mark(dir.path()).unwrap();
        assert!(matches!(held, Some(Hold::Marked(_))));
        drop(held);
        assert!(!pid.exists(), "let go of with the process");

        // Another running process of this same program: refused. A process that is not
        // this program (the test's parent, a shell or cargo), or none at all: taken over.
        // SAFETY: the child only sleeps and exits, both safe after fork in a threaded process.
        let other = unsafe { libc::fork() };
        if other == 0 {
            unsafe {
                libc::sleep(30);
                libc::_exit(0);
            }
        }
        assert!(other > 0);
        std::fs::write(&pid, other.to_string()).unwrap();
        let refused = mark(dir.path()).unwrap();
        // SAFETY: signals and reaps the child made above, nothing else.
        unsafe {
            libc::kill(other, libc::SIGKILL);
            libc::waitpid(other, std::ptr::null_mut(), 0);
        }
        assert!(refused.is_none(), "another Skykeep has it");
        std::fs::write(&pid, parent.to_string()).unwrap();
        assert!(
            mark(dir.path()).unwrap().is_some(),
            "another program's id: left behind"
        );
        std::fs::write(&pid, "4194999").unwrap();
        assert!(mark(dir.path()).unwrap().is_some(), "a process long gone");
    }

    #[test]
    fn only_one_holds_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let first = hold(dir.path()).unwrap().expect("the first holds it");
        assert!(
            hold(dir.path()).unwrap().is_none(),
            "a second is turned away"
        );
        drop(first);
        assert!(
            hold(dir.path()).unwrap().is_some(),
            "free again once the first is gone"
        );
    }
}
