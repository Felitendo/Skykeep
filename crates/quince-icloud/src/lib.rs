//! Clients for Apple's private iCloud web services, riding an authenticated session.
//!
//! These are reverse-engineered endpoints, not a published API. Every response is treated as
//! untrusted: a missing field or a malformed date returns an error, never a panic.

/// What went wrong, said on the terminal in a development build only, and never with anything
/// of the person's in it — no names, record ids, addresses or Apple's answers. A release build
/// says nothing at all: Quince keeps no diagnostics and sends none anywhere.
macro_rules! quiet {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($t)*);
        }
    };
}

pub mod calendar;
pub mod ckdocument;
pub mod cloudkit;
pub mod contacts;
pub mod drive;
pub mod mergeable;
pub mod note_media;
pub mod notes;
pub mod occasions;
pub mod photos;
pub mod photos_upload;
pub mod push;
pub mod reminders;
pub mod shared_albums;
pub mod sync;
pub mod table;
pub mod topotext;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Said in words, never with the address: an address carries the account's id and this
    /// install's client id, and was shown on screen whole.
    #[error("{}", network_words(.0))]
    Network(#[from] reqwest::Error),
    #[error("iCloud sent back something unexpected: {0}")]
    Protocol(String),
    #[error("this needs a fresh sign-in")]
    ReauthRequired,
    #[error("this changed on another device first — refresh and try again")]
    Conflict,
    /// The zone was mid-write for someone else and the change was not applied; it is sent
    /// again (cloudkit's modify), and only reaches a person as Refused if it stays busy.
    #[error("iCloud is busy right now. Nothing was changed — try again in a minute.")]
    Busy,
    /// Apple turned a request down, said in words a person can act on. What Apple actually
    /// sent goes to the log, not the screen: a path and a JSON body told nobody anything.
    #[error("{0}")]
    Refused(String),
    /// Apple has no such record — it was deleted elsewhere, or never kept.
    #[error("that is no longer in iCloud")]
    NotFound,
    /// CloudKit would not read the cursor we held. Handled where the walk happens by starting
    /// over, so this should never reach a person.
    #[error("iCloud no longer recognises where we had got to")]
    StaleCursor,
    #[error(transparent)]
    Storage(#[from] quince_core::Error),
    #[error("couldn't write the file: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What went wrong reaching Apple, in words a person can act on — the address left out.
fn network_words(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "iCloud took too long to answer. Check your connection and try again.".into()
    } else if e.is_connect() {
        "Couldn't reach iCloud. Check your connection and try again.".into()
    } else if e.is_body() || e.is_decode() {
        "iCloud's answer was cut off. Try again.".into()
    } else {
        "The connection to iCloud dropped. Try again.".into()
    }
}
