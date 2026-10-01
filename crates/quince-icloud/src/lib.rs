//! Clients for Apple's private iCloud web services, riding an authenticated session.
//!
//! These are reverse-engineered endpoints, not a published API. Every response is treated as
//! untrusted: a missing field or a malformed date returns an error, never a panic.

pub mod calendar;
pub mod ckdocument;
pub mod cloudkit;
pub mod contacts;
pub mod drive;
pub mod mergeable;
pub mod notes;
pub mod reminders;
pub mod sync;
pub mod topotext;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't reach iCloud: {0}")]
    Network(#[from] reqwest::Error),
    #[error("iCloud sent back something unexpected: {0}")]
    Protocol(String),
    #[error("this needs a fresh sign-in")]
    ReauthRequired,
    #[error("this changed on another device first — refresh and try again")]
    Conflict,
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
