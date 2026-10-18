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

    /// The Apple ID and password, so a session that lapsed can sign itself back in.
    ///
    /// Apple keeps a web session alive for days, not months: once its token lapses only the
    /// password opens a new one, so without it every few days meant signing in by hand —
    /// which is what someone promised "sign in once" ran into. With this machine still
    /// trusted (the 60-day trust token), that sign-in needs no code. It lives here, in the OS
    /// keyring with the rest of this, never in a file; signing out erases it.
    #[serde(default)]
    pub apple_id: Option<String>,
    #[serde(default)]
    pub password: Option<String>,

    /// The account's name and address as Apple gives them at sign-in (`dsInfo`), for the
    /// account shown in the sidebar and Settings.
    #[serde(default)]
    pub account_name: Option<String>,
    #[serde(default)]
    pub account_email: Option<String>,
    /// Whether iCloud's data may be reached from the web at all (`dsInfo.isWebAccessAllowed`).
    /// Off on an account with Advanced Data Protection until the person turns on "Access
    /// iCloud Data on the Web" — and then nothing here can be read, whatever else is right.
    #[serde(default)]
    pub web_access_allowed: Option<bool>,
    /// Advanced Data Protection, which Apple calls iCloud Data Protection (`iCDPEnabled`).
    #[serde(default)]
    pub data_protection: Option<bool>,

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
        // A lapsed session token is not the end: with the trust token and the password kept,
        // a sign-in needs no code. Requiring the session token alone meant one failed
        // attempt that cleared it sent every later launch to the sign-in screen.
        self.session_token.is_some() || (self.trust_token.is_some() && self.password.is_some())
    }
}

/// Where secrets are kept.
///
/// Normally the OS keyring (gnome-keyring / kwallet over the Secret Service API). Tests use
/// a file instead, so a test run never touches the user's real keyring.
enum Backend {
    /// The keyring entry's name: one per state directory.
    Keyring(String),
    File(PathBuf),
}

pub struct Store {
    cookie_path: PathBuf,
    cooldown_path: PathBuf,
    backend: Backend,
}

/// How long to wait before trying to sign in again, kept where the keyring cannot lose it.
///
/// This is not secret — it is a count and a timestamp — so it does not belong in the keyring,
/// and keeping it there made it depend on the keyring working. On a machine with none, or one
/// that is locked, the cooldown lived only in memory: closing the window forgot it, and
/// somebody mistyping a password could go on doing it as fast as they could reopen the app.
/// Hammering sign-in is what gets an account temporarily locked, and it is the person's own
/// account at stake.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Cooldown {
    /// Which account owes the wait, as a hash rather than an address — this file sits in the
    /// clear beside the cookies and there is no reason for somebody's email to be in it.
    ///
    /// Held per account because both things are true at once: signing out to use a different
    /// Apple ID must not inherit the first one's wait, and signing out must not be the way to
    /// wipe a wait and carry on hammering the same one.
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub retry_after: Option<u64>,
}

/// Which account a wait belongs to, without writing down who they are.
#[must_use]
pub fn account_fingerprint(apple_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(apple_id.trim().to_lowercase().as_bytes());
    format!("{:x}", hasher.finalize())[..32].to_string()
}

/// The keyring entry a state directory's session lives under.
///
/// It used to be one fixed name for every directory, so two Quince processes with separate
/// libraries — the app and a probe, or two installs — kept one session between them: each
/// sign-in overwrote the other's tokens while their cookies stayed apart, and each then found
/// itself signed out. The app's own directory keeps the name it always had, so nobody is
/// signed out by this change; any other directory gets an entry of its own.
fn entry_name(state_dir: &Path) -> String {
    if state_dir.ends_with(KEYRING_SERVICE) {
        return KEYRING_ENTRY.to_string();
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(state_dir.to_string_lossy().as_bytes());
    format!("{KEYRING_ENTRY}:{:x}", digest)[..KEYRING_ENTRY.len() + 17].to_string()
}

impl Store {
    /// Production: secrets in the OS keyring.
    pub fn keyring(state_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        Ok(Self {
            cookie_path: state_dir.join("cookies.json"),
            cooldown_path: state_dir.join("cooldown.json"),
            backend: Backend::Keyring(entry_name(state_dir)),
        })
    }

    /// Tests only: secrets in a file under `state_dir`. Never use for real accounts —
    /// it writes tokens to disk in the clear.
    pub fn file(state_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        Ok(Self {
            cookie_path: state_dir.join("cookies.json"),
            cooldown_path: state_dir.join("cooldown.json"),
            backend: Backend::File(state_dir.join("session.json")),
        })
    }

    pub fn cookie_path(&self) -> &Path {
        &self.cookie_path
    }

    /// What this account owes before the next sign-in attempt. Unreadable, absent, or
    /// belonging to somebody else is no wait — which is right for a first run and right for
    /// signing in as a different person.
    pub fn cooldown_for(&self, apple_id: &str) -> Cooldown {
        let stored = self.load_cooldown();
        if stored.account.as_deref() == Some(account_fingerprint(apple_id).as_str()) {
            stored
        } else {
            Cooldown::default()
        }
    }

    /// What is owed, whoever owes it.
    pub fn load_cooldown(&self) -> Cooldown {
        std::fs::read_to_string(&self.cooldown_path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// Written on its own, next to the cookies rather than into the keyring, so a machine
    /// with no keyring still remembers to wait.
    pub fn save_cooldown(&self, cooldown: &Cooldown) -> std::io::Result<()> {
        let json = serde_json::to_string(cooldown).unwrap_or_else(|_| "{}".into());
        std::fs::write(&self.cooldown_path, json)
    }

    fn entry(&self) -> Result<keyring::Entry, keyring::Error> {
        let name = match &self.backend {
            Backend::Keyring(name) => name.as_str(),
            Backend::File(_) => KEYRING_ENTRY,
        };
        keyring::Entry::new(KEYRING_SERVICE, name)
    }

    /// Load the stored session. A missing entry is normal (first run), not an error.
    pub fn load(&self) -> Session {
        self.try_load().unwrap_or_default()
    }

    /// [`Self::load`], telling "nothing stored" apart from "the keyring would not answer" —
    /// locked, or not started yet at login. Read as nothing stored, a locked keyring showed
    /// the sign-in screen, and signing in then replaced the entry and its trust token.
    pub fn try_load(&self) -> Result<Session, crate::Error> {
        let json = match &self.backend {
            Backend::Keyring(_) => match self.entry().and_then(|e| e.get_password()) {
                Ok(j) => j,
                Err(keyring::Error::NoEntry) => return Ok(Session::default()),
                Err(e) => return Err(crate::Error::Keyring(e.to_string())),
            },
            Backend::File(path) => match std::fs::read_to_string(path) {
                Ok(j) => j,
                Err(_) => return Ok(Session::default()),
            },
        };
        Ok(serde_json::from_str(&json).unwrap_or_default())
    }

    pub fn save(&self, session: &Session) -> Result<(), crate::Error> {
        let json = serde_json::to_string(session)?;
        match &self.backend {
            Backend::Keyring(_) => {
                self.entry()
                    .and_then(|e| e.set_password(&json))
                    .map_err(|e| crate::Error::Keyring(e.to_string()))?;

                // Storing the trust token is the whole reason sign-in survives a restart, so
                // prove it landed rather than assume. A keyring that accepts a write and
                // loses it looks identical to working until the user reopens the app.
                if session.trust_token.is_some() {
                    let stored = self.entry().and_then(|e| e.get_password()).ok();
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
            Backend::Keyring(_) => {
                if let Ok(entry) = self.entry() {
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
        // The cooldown stays. It is owed to Apple, not to the session — clearing it here
        // would make signing out a way to wipe the wait and start hammering again, which is
        // exactly what it exists to prevent.
        Ok(())
    }
}

/// Remembering to wait, on a machine that cannot keep secrets.
#[cfg(test)]
mod cooldown_tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::file(dir.path()).expect("a store");
        (dir, store)
    }

    /// A first run owes nothing.
    #[test]
    fn nothing_stored_is_no_wait() {
        let (_dir, store) = store();
        let waiting = store.cooldown_for("someone@example.invalid");
        assert_eq!(waiting.consecutive_failures, 0);
        assert_eq!(waiting.retry_after, None);
    }

    /// The point of keeping it in a file: a cooldown that is forgotten when the window closes
    /// is no cooldown at all, and it used to live only in the keyring — so a machine with none
    /// forgot it every time.
    #[test]
    fn a_wait_survives_being_read_back_by_something_new() {
        let (dir, store) = store();
        store
            .save_cooldown(&Cooldown {
                account: Some(account_fingerprint("someone@example.invalid")),
                consecutive_failures: 4,
                retry_after: Some(1_789_000_000),
            })
            .expect("written");

        let reopened = Store::file(dir.path()).expect("a second store over the same directory");
        let waiting = reopened.cooldown_for("someone@example.invalid");
        assert_eq!(waiting.consecutive_failures, 4);
        assert_eq!(waiting.retry_after, Some(1_789_000_000));
    }

    /// Signing out must not wipe it, or signing out becomes the way round it.
    #[test]
    fn forgetting_the_session_does_not_forget_the_wait() {
        let (_dir, store) = store();
        store
            .save_cooldown(&Cooldown {
                account: Some(account_fingerprint("someone@example.invalid")),
                consecutive_failures: 3,
                retry_after: Some(1_789_000_000),
            })
            .expect("written");
        store.save(&Session::default()).expect("a session");

        store.clear().expect("cleared");

        let waiting = store.cooldown_for("someone@example.invalid");
        assert_eq!(
            waiting.retry_after,
            Some(1_789_000_000),
            "the wait is owed to Apple, not to the session"
        );
    }

    /// Rubbish in the file is not a reason to refuse to sign in for ever.
    #[test]
    fn an_unreadable_file_is_read_as_no_wait() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("cooldown.json"), "not json at all").expect("written");
        assert_eq!(
            store.cooldown_for("someone@example.invalid").retry_after,
            None
        );
    }

    /// The schedule itself: nearly free at first, and a high ceiling, because the penalty for
    /// hammering is somebody's own account being locked.
    #[test]
    fn the_wait_grows_and_stops_growing() {
        let waits: Vec<u64> = (0..7).map(backoff_seconds).collect();
        assert_eq!(waits, vec![0, 5, 15, 60, 180, 600, 600]);
        assert!(
            waits.windows(2).all(|w| w[1] >= w[0]),
            "never goes backwards"
        );
    }
}

#[cfg(test)]
mod per_account_tests {
    use super::*;

    /// Both things have to be true at once, and they pull in opposite directions: signing out
    /// to use a different Apple ID must not inherit the first one's wait, and signing out must
    /// not be the way to wipe a wait and carry on hammering the same one. Holding it per
    /// account is what satisfies both.
    #[test]
    fn a_wait_follows_the_account_and_not_the_session() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::file(dir.path()).expect("a store");
        store
            .save_cooldown(&Cooldown {
                account: Some(account_fingerprint("first@example.invalid")),
                consecutive_failures: 5,
                retry_after: Some(1_789_000_000),
            })
            .expect("written");

        assert_eq!(
            store.cooldown_for("first@example.invalid").retry_after,
            Some(1_789_000_000),
            "the account that earned it still waits"
        );
        assert_eq!(
            store.cooldown_for("second@example.invalid").retry_after,
            None,
            "somebody else does not"
        );
    }

    /// An address is an address however it was typed.
    #[test]
    fn the_same_account_is_recognised_however_it_was_typed() {
        let one = account_fingerprint("Someone@Example.Invalid");
        assert_eq!(one, account_fingerprint("someone@example.invalid"));
        assert_eq!(one, account_fingerprint("  someone@example.invalid  "));
        assert_ne!(one, account_fingerprint("someone.else@example.invalid"));
    }

    /// The reason it is a file of its own rather than part of the session.
    ///
    /// The session lives in the keyring. On a machine with no keyring service, or one that is
    /// locked, storing it fails — and the wait went with it, so closing the window forgot it
    /// and somebody could go on hammering as fast as they could reopen the app. Keeping it
    /// beside the cookies means losing the secrets does not lose the wait.
    #[test]
    fn the_wait_outlives_a_secret_store_that_cannot_keep_anything() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::file(dir.path()).expect("a store");
        store
            .save_cooldown(&Cooldown {
                account: Some(account_fingerprint("someone@example.invalid")),
                consecutive_failures: 4,
                retry_after: Some(1_789_000_000),
            })
            .expect("written");
        store.save(&Session::default()).expect("a session");

        // Everything the secret store held, gone — as it is on a machine where the keyring
        // never accepted it in the first place.
        std::fs::remove_file(dir.path().join("session.json")).expect("removed");

        let reopened = Store::file(dir.path()).expect("a store over the same directory");
        assert!(
            reopened.load().trust_token.is_none(),
            "the secrets really are gone"
        );
        assert_eq!(
            reopened.cooldown_for("someone@example.invalid").retry_after,
            Some(1_789_000_000),
            "and the wait is still owed"
        );
    }

    /// The file sits in the clear beside the cookies, so it must not say who anybody is.
    #[test]
    fn the_file_does_not_hold_an_address() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::file(dir.path()).expect("a store");
        store
            .save_cooldown(&Cooldown {
                account: Some(account_fingerprint("someone@example.invalid")),
                consecutive_failures: 1,
                retry_after: Some(1),
            })
            .expect("written");

        let written = std::fs::read_to_string(dir.path().join("cooldown.json")).expect("read");
        assert!(!written.contains("someone"), "no local part");
        assert!(!written.contains("example.invalid"), "no domain");
        assert!(!written.contains('@'), "nothing that looks like an address");
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    #[test]
    fn each_library_keeps_its_own_session_and_the_app_keeps_its_old_one() {
        let app = Path::new("/home/someone/.local/share/dev.quince.client");
        let probe = Path::new("/home/someone/.local/share/quince-probe");
        assert_eq!(
            entry_name(app),
            "session",
            "the app is not signed out by the change"
        );
        assert_ne!(entry_name(probe), entry_name(app));
        assert_eq!(entry_name(probe), entry_name(probe), "stable");
    }
}
