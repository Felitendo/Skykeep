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
