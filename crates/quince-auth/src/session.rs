//! Session state, and the keyring that makes it outlive the process.
//!
//! The reason every Electron iCloud wrapper signs you out is that it keeps cookies in a bare
//! browser partition and never asks Apple for the one durable credential that exists: the
//! trust token from `/2sv/trust`. We capture it and hand it to the OS keyring, so a restart
//! resumes instead of re-prompting.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "dev.quince.client";
const KEYRING_ENTRY: &str = "session";

/// Headers Apple rotates across responses. Any of them can appear on any call, so we capture
/// after every request rather than only at the end of sign-in.
pub const CAPTURED_HEADERS: &[(&str, Field)] = &[
    ("x-apple-id-account-country", Field::AccountCountry),
    ("x-apple-id-session-id", Field::SessionId),
    ("x-apple-session-token", Field::SessionToken),
    ("x-apple-twosv-trust-token", Field::TrustToken),
    ("scnt", Field::Scnt),
];

#[derive(Clone, Copy, Debug)]
pub enum Field {
    AccountCountry,
    SessionId,
    SessionToken,
    TrustToken,
    Scnt,
}

/// Everything needed to resume a signed-in session without a password.
///
/// This is secret material. It lives in the OS keyring, never in a plaintext file.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Session {
    /// Stable per-install client id. Apple ties the trust token to it, so it must not rotate.
    pub client_id: Option<String>,
    pub account_country: Option<String>,
    pub session_id: Option<String>,
    pub session_token: Option<String>,
    /// The durable credential. Roughly 60 days, and the whole reason sign-in survives a restart.
    pub trust_token: Option<String>,
    pub scnt: Option<String>,
    /// Apple's per-account service URL map, from `accountLogin`.
    pub webservices: Option<serde_json::Value>,
    pub dsid: Option<String>,

    /// Failed sign-ins in a row. Drives the local backoff.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Unix seconds before which we refuse to contact Apple again.
    ///
    /// Persisted deliberately: a cooldown that resets when the app restarts is no cooldown
    /// at all, and repeated attempts are what get accounts temporarily locked.
    #[serde(default)]
    pub retry_after: Option<u64>,
}

/// Seconds to wait after `failures` consecutive failed sign-ins.
///
/// Apple rate-limits, and the penalty for ignoring it is a temporary account lock — so the
/// first retry is nearly free and the ceiling is deliberately high.
pub fn backoff_seconds(failures: u32) -> u64 {
    match failures {
        0 => 0,
        1 => 5,
        2 => 15,
        3 => 60,
        4 => 180,
        _ => 600,
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Session {
    pub fn set(&mut self, field: Field, value: String) {
        if value.is_empty() {
            return;
        }
        match field {
            Field::AccountCountry => self.account_country = Some(value),
            Field::SessionId => self.session_id = Some(value),
            Field::SessionToken => self.session_token = Some(value),
            Field::TrustToken => self.trust_token = Some(value),
            Field::Scnt => self.scnt = Some(value),
        }
    }

    /// Stable client id, minted once and then reused for the life of the install.
    pub fn client_id(&mut self) -> String {
        if self.client_id.is_none() {
            self.client_id = Some(format!("auth-{}", uuid::Uuid::new_v4()).to_lowercase());
        }
        self.client_id.clone().expect("just set")
    }

    /// A service's base URL from Apple's map, e.g. `calendar`, `ckdatabasews`, `drivews`.
    pub fn service_url(&self, key: &str) -> Option<String> {
        self.webservices
            .as_ref()?
            .pointer(&format!("/{key}/url"))?
            .as_str()
            .map(|s| s.trim_end_matches('/').to_string())
    }

    /// True when there's enough stored state to try resuming without a password.
    pub fn is_resumable(&self) -> bool {
        self.session_token.is_some()
    }
}

/// Where secrets are kept.
///
/// Normally the OS keyring (gnome-keyring / kwallet over the Secret Service API). Tests use
/// a file instead, so a test run never touches the user's real keyring.
enum Backend {
    Keyring,
    File(PathBuf),
}

pub struct Store {
    cookie_path: PathBuf,
    backend: Backend,
}

impl Store {
    /// Production: secrets in the OS keyring.
    pub fn keyring(state_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        Ok(Self {
            cookie_path: state_dir.join("cookies.json"),
            backend: Backend::Keyring,
        })
    }

    /// Tests only: secrets in a file under `state_dir`. Never use for real accounts —
    /// it writes tokens to disk in the clear.
    pub fn file(state_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        Ok(Self {
            cookie_path: state_dir.join("cookies.json"),
            backend: Backend::File(state_dir.join("session.json")),
        })
    }

    pub fn cookie_path(&self) -> &Path {
        &self.cookie_path
    }

    fn entry() -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENTRY)
    }

    /// Load the stored session. A missing entry is normal (first run), not an error.
    pub fn load(&self) -> Session {
        let json = match &self.backend {
            Backend::Keyring => match Self::entry().and_then(|e| e.get_password()) {
                Ok(j) => j,
                Err(_) => return Session::default(),
            },
            Backend::File(path) => match std::fs::read_to_string(path) {
                Ok(j) => j,
                Err(_) => return Session::default(),
            },
        };
        serde_json::from_str(&json).unwrap_or_default()
    }

    pub fn save(&self, session: &Session) -> Result<(), crate::Error> {
        let json = serde_json::to_string(session)?;
        match &self.backend {
            Backend::Keyring => {
                Self::entry()
                    .and_then(|e| e.set_password(&json))
                    .map_err(|e| crate::Error::Keyring(e.to_string()))?;

                // Storing the trust token is the whole reason sign-in survives a restart, so
                // prove it landed rather than assume. A keyring that accepts a write and
                // loses it looks identical to working until the user reopens the app.
                if session.trust_token.is_some() {
                    let stored = Self::entry().and_then(|e| e.get_password()).ok();
                    if stored.as_deref() != Some(json.as_str()) {
                        return Err(crate::Error::Keyring(
                            "the system keyring accepted the sign-in but did not keep it. \
                             Check that a keyring service (gnome-keyring or kwallet) is \
                             running and unlocked"
                                .into(),
                        ));
                    }
                }
                Ok(())
            }
            Backend::File(path) => Ok(std::fs::write(path, json)?),
        }
    }

    /// Forget everything. Used by sign-out, and when Apple rejects the stored session for good.
    pub fn clear(&self) -> Result<(), crate::Error> {
        match &self.backend {
            Backend::Keyring => {
                if let Ok(entry) = Self::entry() {
                    match entry.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => {}
                        Err(e) => return Err(crate::Error::Keyring(e.to_string())),
                    }
                }
            }
            Backend::File(path) => {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }
        if self.cookie_path.exists() {
            std::fs::remove_file(&self.cookie_path)?;
        }
        Ok(())
    }
}
