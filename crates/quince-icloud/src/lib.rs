//! Clients for Apple's private iCloud web services, riding an authenticated session.
//!
//! These are reverse-engineered endpoints, not a published API. Every response is treated as
//! untrusted: a missing field or a malformed date returns an error, never a panic.

pub mod calendar;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't reach iCloud: {0}")]
    Network(#[from] reqwest::Error),
    #[error("iCloud sent back something unexpected: {0}")]
    Protocol(String),
    #[error("this needs a fresh sign-in")]
    ReauthRequired,
    #[error(transparent)]
    Storage(#[from] quince_core::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
