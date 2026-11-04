//! Moving a file into place, made to hold on Windows.
//!
//! A file just written is opened straight after by the virus scanner (Defender scans each new
//! file as it is closed), the search indexer, or Explorer making a thumbnail. While one of them
//! has it open, Windows refuses to rename it, or the folder it is in, with "access is denied"
//! (ERROR_ACCESS_DENIED) or "being used by another process" (ERROR_SHARING_VIOLATION,
//! ERROR_LOCK_VIOLATION) — for a moment, until they let go. A rename that failed on that at
//! once failed an export at its very last step, and a download that had already arrived. Here
//! the same rename is tried again for a while before it is given up on; anything else is said
//! at once, and elsewhere nothing changes.

use std::io;
use std::path::Path;
use std::time::Duration;

/// How long a rename waits for Windows to let go of a file by default: a scan of a large
/// photo or video takes a few seconds.
pub const HELD_FOR: Duration = Duration::from_secs(5);

/// The pause between two tries.
const STEP: Duration = Duration::from_millis(50);

/// Whether a Windows error code says the file is held by someone else for now: access denied
/// (5), a sharing violation (32), a lock violation (33).
fn held_code(code: Option<i32>) -> bool {
    matches!(code, Some(5 | 32 | 33))
}

/// Whether `e` is Windows saying a file is held by someone else for now. Never elsewhere: the
/// same numbers mean other things there (5 is EIO on Linux).
pub fn held_elsewhere(e: &io::Error) -> bool {
    cfg!(windows) && held_code(e.raw_os_error())
}

/// `op`, tried again while it fails with an error `held` says will pass, for up to `within`
/// (`wait` is the pause between tries). Any other answer is returned as it is.
pub fn patiently<T>(
    within: Duration,
    held: impl Fn(&io::Error) -> bool,
    mut wait: impl FnMut(Duration),
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut waited = Duration::ZERO;
    loop {
        match op() {
            Err(e) if held(&e) && waited < within => {
                wait(STEP);
                waited += STEP;
            }
            done => return done,
        }
    }
}

/// [`std::fs::rename`], waiting up to [`HELD_FOR`] for a file Windows says is held.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    rename_within(from, to, HELD_FOR)
}

/// [`std::fs::rename`], waiting up to `within` for a file Windows says is held.
pub fn rename_within(from: &Path, to: &Path, within: Duration) -> io::Result<()> {
    patiently(within, held_elsewhere, std::thread::sleep, || {
        std::fs::rename(from, to)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn sharing_violation() -> io::Error {
        io::Error::from_raw_os_error(32)
    }

    /// Held for a moment, as Defender holds a file it is scanning: tried again until it goes.
    #[test]
    fn a_file_held_for_a_moment_is_moved_once_it_is_let_go() {
        let tries = Cell::new(0);
        let waits = Cell::new(0);
        let done = patiently(
            Duration::from_secs(5),
            |e| held_code(e.raw_os_error()),
            |_| waits.set(waits.get() + 1),
            || {
                tries.set(tries.get() + 1);
                if tries.get() < 4 {
                    Err(sharing_violation())
                } else {
                    Ok("moved")
                }
            },
        );
        assert_eq!(done.unwrap(), "moved");
        assert_eq!(tries.get(), 4);
        assert_eq!(waits.get(), 3);
    }

    /// Held for good: given up on once the time is spent, with Windows' own error.
    #[test]
    fn a_file_held_for_good_is_given_up_on_in_time() {
        let tries = Cell::new(0);
        let done: io::Result<()> = patiently(
            Duration::from_millis(500),
            |e| held_code(e.raw_os_error()),
            |_| {},
            || {
                tries.set(tries.get() + 1);
                Err(sharing_violation())
            },
        );
        assert_eq!(done.unwrap_err().raw_os_error(), Some(32));
        assert_eq!(
            tries.get(),
            11,
            "ten waits of 50 ms, and the try after them"
        );
    }

    /// Any other failure — the file gone, a full disk — is said at once.
    #[test]
    fn any_other_failure_is_said_at_once() {
        let tries = Cell::new(0);
        let done: io::Result<()> = patiently(
            Duration::from_secs(5),
            |e| held_code(e.raw_os_error()),
            |_| panic!("no wait for an error that will not pass"),
            || {
                tries.set(tries.get() + 1);
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
        );
        assert_eq!(done.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(tries.get(), 1);
    }

    #[test]
    fn only_windows_holding_a_file_counts() {
        assert!(held_code(Some(5)) && held_code(Some(32)) && held_code(Some(33)));
        assert!(!held_code(Some(2)) && !held_code(Some(112)) && !held_code(None));
        // Elsewhere the same numbers are other errors, never waited on.
        assert_eq!(
            held_elsewhere(&io::Error::from_raw_os_error(5)),
            cfg!(windows)
        );
    }

    #[test]
    fn a_rename_moves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("a.part"), dir.path().join("a.pdf"));
        std::fs::write(&from, b"x").unwrap();
        rename(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"x");
        assert_eq!(
            rename(&from, &to).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
