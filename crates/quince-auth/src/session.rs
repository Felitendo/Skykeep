//! Session state, and the keyring that makes it outlive the process.
//!
//! The reason every Electron iCloud wrapper signs you out is that it keeps cookies in a bare
//! browser partition and never asks Apple for the one durable credential that exists: the
//! trust token from `/2sv/trust`. We capture it and hand it to the OS keyring, so a restart
//! resumes instead of re-prompting.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEYRING_ENTRY: &str = "session";

/// Identifiers the app was published under before, newest first: the keyring services its
/// session may still be under, and the folders `src-tauri/src/migrate.rs` moves. The current
/// identifier is never written here — it is tauri.conf.json's. Renaming the app means adding
/// the identifier being replaced to the front of this list.
pub const PREVIOUS_IDENTIFIERS: &[&str] =
    &["io.github.yashwanthkamireddi.Pomander", "dev.quince.client"];

/// The keyring services: the one the session is kept under (the app's identifier), and the
/// ones it was kept under before — read when the current one holds nothing, moved across
/// (written, read back, only then deleted there), never written.
struct Services {
    current: String,
    previous: Vec<&'static str>,
}

/// The app's identifier, from tauri.conf.json itself, so every client — the app, a probe, a
/// test — uses the same service without being told, and a rename is one edit there.
fn configured_identifier() -> Option<String> {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).ok()?;
    Some(conf.get("identifier")?.as_str()?.to_string())
}

fn services() -> Result<&'static Services, String> {
    static SERVICES: std::sync::OnceLock<Services> = std::sync::OnceLock::new();
    Ok(SERVICES.get_or_init(|| {
        let current =
            configured_identifier().unwrap_or_else(|| PREVIOUS_IDENTIFIERS[0].to_string());
        let previous = PREVIOUS_IDENTIFIERS
            .iter()
            .copied()
            .filter(|p| *p != current)
            .collect();
        Services { current, previous }
    }))
}

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
    // The app's directory under either identifier: the data directory moves with the
    // identifier, and a launch that could not move it still runs from the old one.
    let ours = services().is_ok_and(|s| {
        std::iter::once(s.current.as_str())
            .chain(s.previous.iter().copied())
            .any(|id| state_dir.ends_with(id))
    });
    if ours {
        return KEYRING_ENTRY.to_string();
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(state_dir.to_string_lossy().as_bytes());
    format!("{KEYRING_ENTRY}:{:x}", digest)[..KEYRING_ENTRY.len() + 17].to_string()
}

/// One secret in the system keyring, split over as many entries as the platform needs.
///
/// Windows' Credential Manager keeps at most 2,560 bytes in one credential, and the keyring
/// crate stores text there as UTF-16 — 1,280 characters. A signed-in session is larger, so on
/// Windows sign-in failed outright ("longer than platform limit of 2560 chars"). There the
/// secret is written as parts (`<name>#g<k>#1`, `<name>#g<k>#2`, …, a generation `k` of their
/// own each save) and the entry itself holds only `pomander-parts:g<k>:<n>`, written last, so a
/// save cut short never leaves a half that reads as whole (see [`write_secret`]). Elsewhere the whole secret stays in the one entry, as it always has; reading takes
/// either shape on every platform.
trait Secrets {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// The system keyring, under one service name.
struct SystemKeyring(&'static str);

/// Where the session is kept now.
fn current() -> Result<SystemKeyring, String> {
    Ok(SystemKeyring(services()?.current.as_str()))
}

/// Where it was kept under earlier identifiers: read, and moved, newest first.
fn previous() -> Result<Vec<SystemKeyring>, String> {
    Ok(services()?
        .previous
        .iter()
        .map(|p| SystemKeyring(p))
        .collect())
}

impl Secrets for SystemKeyring {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(self.0, name).and_then(|e| e.get_password()) {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(self.0, name)
            .and_then(|e| e.set_password(value))
            .map_err(|e| e.to_string())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        match keyring::Entry::new(self.0, name).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

const PARTS: &str = "pomander-parts:";
/// The most parts a secret may take: 32 × 1,000 characters is far past any session.
const MOST_PARTS: usize = 32;

/// How many characters one entry may hold here: Windows' limit, with room to spare; none
/// elsewhere.
fn part_size() -> Option<usize> {
    if cfg!(windows) {
        Some(1000)
    } else {
        None
    }
}

fn part_name(name: &str, i: usize) -> String {
    format!("{name}#{i}")
}

/// A part of generation `generation`: `name#g<k>#<i>`. Each save writes a generation of its
/// own, beside the one the head names, and switches the head over last.
fn generation_part(name: &str, generation: u64, i: usize) -> String {
    format!("{name}#g{generation}#{i}")
}

/// What a head says: the secret itself, or parts — the generation they are under (`None`:
/// written before generations, as `name#<i>`) and how many.
enum Head {
    Whole(String),
    Parts(Option<u64>, usize),
}

fn head_of(value: String) -> Result<Head, String> {
    let Some(rest) = value.strip_prefix(PARTS) else {
        return Ok(Head::Whole(value));
    };
    let damaged = || "the stored sign-in is damaged".to_string();
    let (generation, n) = match rest.strip_prefix('g').and_then(|r| r.split_once(':')) {
        Some((g, n)) => (Some(g.parse().map_err(|_| damaged())?), n),
        None => (None, rest),
    };
    let n: usize = n.parse().map_err(|_| damaged())?;
    if n == 0 || n > MOST_PARTS {
        return Err(damaged());
    }
    Ok(Head::Parts(generation, n))
}

fn part_of(name: &str, generation: Option<u64>, i: usize) -> String {
    match generation {
        Some(g) => generation_part(name, g, i),
        None => part_name(name, i),
    }
}

/// The secret under `name`, whole or reassembled from its parts.
fn read_secret(k: &dyn Secrets, name: &str) -> Result<Option<String>, String> {
    let Some(head) = k.get(name)? else {
        return Ok(None);
    };
    let (generation, n) = match head_of(head)? {
        Head::Whole(value) => return Ok(Some(value)),
        Head::Parts(generation, n) => (generation, n),
    };
    let mut whole = String::new();
    for i in 1..=n {
        let part = k
            .get(&part_of(name, generation, i))?
            .ok_or_else(|| "part of the stored sign-in is missing".to_string())?;
        whole.push_str(&part);
    }
    Ok(Some(whole))
}

/// Keep `value` under `name`, in parts where one entry cannot hold it.
///
/// The parts were written over the very names the stored session's parts had, and only then
/// the head: a crash between them left the old head over a mix of new and old parts, read as
/// damaged — the session lost, and two-factor asked for again. Each save's parts now go under
/// names of their own (a new generation), the head is switched to them in one write, and only
/// then are the old ones let go of: a crash anywhere leaves either the old session whole or
/// the new one.
fn write_secret(
    k: &dyn Secrets,
    name: &str,
    value: &str,
    size: Option<usize>,
) -> Result<(), String> {
    let chars: Vec<char> = value.chars().collect();
    let parts: Vec<String> = match size {
        Some(size) if chars.len() > size => {
            chars.chunks(size).map(|c| c.iter().collect()).collect()
        }
        _ => Vec::new(),
    };
    if parts.len() > MOST_PARTS {
        return Err("the sign-in is too large to keep".into());
    }
    // What the head names now, to be let go of once the new one is in place. A damaged head
    // names nothing to keep.
    let before = match k.get(name)?.map(head_of) {
        Some(Ok(Head::Parts(generation, n))) => Some((generation, n)),
        _ => None,
    };
    let generation = before.and_then(|(g, _)| g).map_or(1, |g| g.wrapping_add(1));
    if parts.is_empty() {
        k.set(name, value)?;
    } else {
        for (i, part) in parts.iter().enumerate() {
            k.set(&generation_part(name, generation, i + 1), part)?;
        }
        k.set(name, &format!("{PARTS}g{generation}:{}", parts.len()))?;
    }
    // The generation before, and parts left under the names used before generations.
    if let Some((Some(old), n)) = before {
        for i in 1..=n {
            k.delete(&generation_part(name, old, i))?;
        }
    }
    for i in 1..=MOST_PARTS {
        if k.get(&part_name(name, i))?.is_none() {
            break;
        }
        k.delete(&part_name(name, i))?;
    }
    Ok(())
}

/// Forget the secret under `name`, and every part of it.
fn delete_secret(k: &dyn Secrets, name: &str) -> Result<(), String> {
    if let Some(Ok(Head::Parts(Some(generation), n))) = k.get(name)?.map(head_of) {
        for i in 1..=n {
            k.delete(&generation_part(name, generation, i))?;
        }
    }
    k.delete(name)?;
    for i in 1..=MOST_PARTS {
        if k.get(&part_name(name, i))?.is_none() {
            break;
        }
        k.delete(&part_name(name, i))?;
    }
    Ok(())
}

/// The secret under `name`, from `current`; or, where that holds nothing, from the first of
/// `earlier` that has it (`legacy`) — moved across on the way.
///
/// The move is a copy first: written to `current`, read back and compared, and only then
/// deleted from `legacy`. Any step failing leaves `legacy` as it was and answers its value, so
/// the session is never lost to a keyring that refuses a write, and the move is tried again on
/// the next read. A `current` that will not answer at all (locked) is an error, as it always
/// was: never read as "nothing there", which would fall through to a stale legacy entry.
fn read_moving(
    current: &dyn Secrets,
    earlier: &[&dyn Secrets],
    name: &str,
    size: Option<usize>,
) -> Result<Option<String>, String> {
    if let Some(found) = read_secret(current, name)? {
        return Ok(Some(found));
    }
    // An earlier service not answering is no reason to fail a read that found nothing.
    let Some((legacy, old)) = earlier
        .iter()
        .find_map(|k| read_secret(*k, name).ok().flatten().map(|v| (*k, v)))
    else {
        return Ok(None);
    };
    let landed = write_secret(current, name, &old, size)
        .and_then(|()| read_secret(current, name))
        .is_ok_and(|back| back.as_deref() == Some(old.as_str()));
    if landed {
        if let Err(e) = delete_secret(legacy, name) {
            // Harmless: `current` answers from now on, and save and clear delete it again.
            quiet!("quince: keyring: the old entry stayed after the move: {e}");
        }
    } else {
        quiet!(
            "quince: keyring: could not move the session to the new name; still read from the old"
        );
        // Nothing half-written left under the new name to be read in place of the old.
        let _ = delete_secret(current, name);
    }
    Ok(Some(old))
}

/// Write `bytes` to `path` whole or not at all: to a file beside it, made sure of on disk, then
/// renamed over it. Truncated in place, a crash or a full disk part way left an empty or cut-off
/// cookie jar, read as no cookies at all: the session checked again from the start, and maybe
/// a code asked for. `private`: readable by this user alone.
pub(crate) fn write_whole(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    use std::io::Write;
    let part = path.with_file_name(format!(
        ".{}.part",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&part)?;
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            // A part left by a run before kept the mode it was made with: set it, too.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let _ = private;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&part, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
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
        write_whole(&self.cooldown_path, json.as_bytes(), false)
    }

    fn name(&self) -> &str {
        match &self.backend {
            Backend::Keyring(name) => name.as_str(),
            Backend::File(_) => KEYRING_ENTRY,
        }
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
            Backend::Keyring(_) => match current().and_then(|now| {
                let before = previous()?;
                let before: Vec<&dyn Secrets> = before.iter().map(|k| k as &dyn Secrets).collect();
                read_moving(&now, &before, self.name(), part_size())
            }) {
                Ok(Some(j)) => j,
                Ok(None) => return Ok(Session::default()),
                Err(e) => return Err(crate::Error::Keyring(e)),
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
                let now = current().map_err(crate::Error::Keyring)?;
                write_secret(&now, self.name(), &json, part_size())
                    .map_err(crate::Error::Keyring)?;
                // Anything still under an earlier name is older than this: gone, so a later
                // sign-out cannot bring it back.
                for before in previous().unwrap_or_default() {
                    let _ = delete_secret(&before, self.name());
                }

                // Storing the trust token is the whole reason sign-in survives a restart, so
                // prove it landed rather than assume. A keyring that accepts a write and
                // loses it looks identical to working until the user reopens the app.
                if session.trust_token.is_some() {
                    let stored = read_secret(&now, self.name()).ok().flatten();
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
                let now = current().map_err(crate::Error::Keyring)?;
                delete_secret(&now, self.name()).map_err(crate::Error::Keyring)?;
                // And under every earlier name, or the next read would move it back.
                for before in previous().map_err(crate::Error::Keyring)? {
                    delete_secret(&before, self.name()).map_err(crate::Error::Keyring)?;
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
    fn the_service_is_the_identifier_in_tauri_conf() {
        let s = services().unwrap();
        assert_eq!(Some(s.current.clone()), configured_identifier());
        assert!(!s.previous.contains(&s.current.as_str()));
        assert_eq!(s.previous, PREVIOUS_IDENTIFIERS);
    }

    #[test]
    fn each_library_keeps_its_own_session_and_the_app_keeps_its_old_one() {
        let share = Path::new("/home/someone/.local/share");
        let app = &share.join(&services().unwrap().current);
        let before = &share.join(PREVIOUS_IDENTIFIERS[0]);
        assert_eq!(
            entry_name(before),
            "session",
            "nor by the identifier changing"
        );
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

#[cfg(test)]
mod whole_tests {
    use super::*;

    /// Written whole or not at all, owner-only, and never a part file left behind.
    #[test]
    fn a_file_is_replaced_whole_and_kept_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        std::fs::write(&path, "old cookies").unwrap();
        write_whole(&path, b"new cookies", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new cookies");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        // One that cannot be written leaves what was there as it was.
        let gone = dir.path().join("missing").join("cookies.json");
        assert!(write_whole(&gone, b"x", true).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new cookies");
    }
}

/// A secret kept in parts where one entry cannot hold it — Windows' Credential Manager — and
/// read back whole, whichever shape it was written in.
#[cfg(test)]
mod parts_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<String, String>>, Option<usize>);
    impl Secrets for Memory {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            Ok(self.0.borrow().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            // The platform's own limit, as Windows enforces it.
            if self.1.is_some_and(|most| value.chars().count() > most) {
                return Err("longer than platform limit".into());
            }
            self.0.borrow_mut().insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(name);
            Ok(())
        }
    }

    #[test]
    fn a_session_too_large_for_one_entry_is_kept_in_parts() {
        let k = Memory(RefCell::default(), Some(1280));
        let session = "x".repeat(3500) + "ü€";
        write_secret(&k, "s", &session, Some(1000)).unwrap();
        assert_eq!(
            k.0.borrow().get("s").map(String::as_str),
            Some("pomander-parts:g1:4")
        );
        assert_eq!(
            read_secret(&k, "s").unwrap().as_deref(),
            Some(session.as_str())
        );
        // Shorter next time, under a generation of its own: the old parts all go.
        write_secret(&k, "s", &"y".repeat(1500), Some(1000)).unwrap();
        assert_eq!(k.0.borrow().len(), 3, "{:?}", k.0.borrow().keys());
        assert_eq!(read_secret(&k, "s").unwrap(), Some("y".repeat(1500)));
        delete_secret(&k, "s").unwrap();
        assert!(k.0.borrow().is_empty());
        assert_eq!(read_secret(&k, "s").unwrap(), None);
    }

    #[test]
    fn where_one_entry_holds_it_the_secret_stays_whole() {
        let k = Memory::default();
        let session = "z".repeat(5000);
        write_secret(&k, "s", &session, None).unwrap();
        assert_eq!(k.0.borrow().len(), 1);
        assert_eq!(read_secret(&k, "s").unwrap(), Some(session));
    }

    #[test]
    fn a_session_kept_whole_before_is_read_as_it_is() {
        let k = Memory::default();
        k.set("s", "{\"dsid\":\"1\"}").unwrap();
        assert_eq!(
            read_secret(&k, "s").unwrap().as_deref(),
            Some("{\"dsid\":\"1\"}")
        );
    }

    #[test]
    fn a_missing_part_is_an_error_not_a_session() {
        let k = Memory::default();
        write_secret(&k, "s", &"q".repeat(2500), Some(1000)).unwrap();
        k.delete("s#g1#2").unwrap();
        assert!(read_secret(&k, "s").is_err());
    }

    /// A save that stops part way — the process killed, the machine off — leaves the session
    /// before it whole. Writing the new parts over the old ones' names, it left the old head
    /// over a mix, and the person was signed out.
    #[test]
    fn a_save_cut_short_leaves_the_last_session_whole() {
        let k = Memory(RefCell::default(), Some(1280));
        let first = "a".repeat(2500);
        write_secret(&k, "s", &first, Some(1000)).unwrap();
        // Cut short after its first part: the store refuses everything after.
        struct CutShort<'a>(&'a Memory, std::cell::Cell<usize>);
        impl Secrets for CutShort<'_> {
            fn get(&self, name: &str) -> Result<Option<String>, String> {
                self.0.get(name)
            }
            fn set(&self, name: &str, value: &str) -> Result<(), String> {
                if self.1.get() == 0 {
                    return Err("the process ended".into());
                }
                self.1.set(self.1.get() - 1);
                self.0.set(name, value)
            }
            fn delete(&self, name: &str) -> Result<(), String> {
                self.0.delete(name)
            }
        }
        let cut = CutShort(&k, std::cell::Cell::new(1));
        assert!(write_secret(&cut, "s", &"b".repeat(2500), Some(1000)).is_err());
        assert_eq!(read_secret(&k, "s").unwrap(), Some(first));
        // The next save goes through, and nothing of the cut one is left.
        write_secret(&k, "s", &"c".repeat(1500), Some(1000)).unwrap();
        assert_eq!(read_secret(&k, "s").unwrap(), Some("c".repeat(1500)));
    }

    /// Parts written before generations are read, and replaced cleanly by the next save.
    #[test]
    fn parts_kept_before_generations_still_read_and_go() {
        let k = Memory::default();
        k.set("s#1", "ab").unwrap();
        k.set("s#2", "cd").unwrap();
        k.set("s", "pomander-parts:2").unwrap();
        assert_eq!(read_secret(&k, "s").unwrap().as_deref(), Some("abcd"));
        write_secret(&k, "s", &"e".repeat(1500), Some(1000)).unwrap();
        assert!(k.0.borrow().get("s#1").is_none() && k.0.borrow().get("s#2").is_none());
        assert_eq!(read_secret(&k, "s").unwrap(), Some("e".repeat(1500)));
    }
}

/// The session moving from the old identifier's keyring service to the new one.
#[cfg(test)]
mod move_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A keyring in memory. `refuse`: every write fails. `forget`: writes are accepted and
    /// dropped — the keyring that says yes and keeps nothing. `locked`: nothing answers.
    #[derive(Default)]
    struct Mock {
        held: RefCell<BTreeMap<String, String>>,
        refuse: bool,
        forget: bool,
        locked: bool,
    }
    impl Secrets for Mock {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            if self.locked {
                return Err("locked".into());
            }
            Ok(self.held.borrow().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            if self.refuse || self.locked {
                return Err("refused".into());
            }
            if !self.forget {
                self.held.borrow_mut().insert(name.into(), value.into());
            }
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            if self.locked {
                return Err("locked".into());
            }
            self.held.borrow_mut().remove(name);
            Ok(())
        }
    }

    fn holding(name: &str, value: &str, size: Option<usize>) -> Mock {
        let k = Mock::default();
        write_secret(&k, name, value, size).unwrap();
        k
    }

    #[test]
    fn the_session_moves_and_the_old_entry_goes_only_after() {
        let old = holding("session", "{\"trust\":\"t\"}", None);
        let new = Mock::default();
        let read = read_moving(&new, &[&old], "session", None).unwrap();
        assert_eq!(read.as_deref(), Some("{\"trust\":\"t\"}"));
        assert_eq!(
            new.held.borrow().get("session").map(String::as_str),
            Some("{\"trust\":\"t\"}")
        );
        assert!(old.held.borrow().is_empty(), "moved, not copied");
        // Read again: from the new name, the old one is not needed.
        assert_eq!(read_moving(&new, &[&old], "session", None).unwrap(), read);
    }

    #[test]
    fn a_session_in_parts_moves_whole_with_every_part() {
        // Windows: the Credential Manager's targets are `session` and `session#g1#1..n`.
        let big = "w".repeat(3500);
        let old = holding("session", &big, Some(1000));
        assert_eq!(old.held.borrow().len(), 5);
        let new = Mock::default();
        assert_eq!(
            read_moving(&new, &[&old], "session", Some(1000)).unwrap(),
            Some(big.clone())
        );
        assert_eq!(
            new.held.borrow().get("session").map(String::as_str),
            Some("pomander-parts:g1:4")
        );
        assert_eq!(read_secret(&new, "session").unwrap(), Some(big));
        assert!(
            old.held.borrow().is_empty(),
            "every part gone from the old name"
        );
    }

    #[test]
    fn a_new_keyring_that_refuses_keeps_the_old_entry_and_reads_it() {
        let old = holding("session", "s", None);
        let new = Mock {
            refuse: true,
            ..Mock::default()
        };
        assert_eq!(
            read_moving(&new, &[&old], "session", None)
                .unwrap()
                .as_deref(),
            Some("s")
        );
        assert_eq!(
            old.held.borrow().get("session").map(String::as_str),
            Some("s")
        );
    }

    #[test]
    fn a_new_keyring_that_drops_the_write_keeps_the_old_entry() {
        let old = holding("session", "s", None);
        let new = Mock {
            forget: true,
            ..Mock::default()
        };
        assert_eq!(
            read_moving(&new, &[&old], "session", None)
                .unwrap()
                .as_deref(),
            Some("s")
        );
        assert_eq!(
            old.held.borrow().get("session").map(String::as_str),
            Some("s")
        );
    }

    #[test]
    fn a_locked_keyring_is_an_error_not_a_fall_back_to_the_old_entry() {
        let old = holding("session", "stale", None);
        let new = Mock {
            locked: true,
            ..Mock::default()
        };
        assert!(read_moving(&new, &[&old], "session", None).is_err());
        assert_eq!(old.held.borrow().len(), 1, "untouched");
    }

    #[test]
    fn the_new_entry_wins_over_a_leftover_old_one() {
        let old = holding("session", "older", None);
        let new = holding("session", "newer", None);
        assert_eq!(
            read_moving(&new, &[&old], "session", None)
                .unwrap()
                .as_deref(),
            Some("newer")
        );
    }

    #[test]
    fn the_first_earlier_name_holding_it_is_moved_and_the_rest_left() {
        let older = holding("session", "oldest", None);
        let old = holding("session", "old", None);
        let empty = Mock::default();
        let new = Mock::default();
        assert_eq!(
            read_moving(&new, &[&empty, &old, &older], "session", None)
                .unwrap()
                .as_deref(),
            Some("old")
        );
        assert!(old.held.borrow().is_empty());
    }

    #[test]
    fn nothing_under_either_name_is_nothing() {
        assert_eq!(
            read_moving(&Mock::default(), &[&Mock::default()], "session", None).unwrap(),
            None
        );
    }

    /// The real Secret Service, isolated: run only by the script that starts a private session
    /// bus and a throwaway gnome-keyring (QUINCE_ISOLATED_KEYRING names its bus), never
    /// against the person's own keyring.
    #[test]
    #[ignore = "needs an isolated Secret Service; see the comment"]
    fn the_move_through_a_real_secret_service() {
        let Ok(bus) = std::env::var("QUINCE_ISOLATED_KEYRING") else {
            return;
        };
        assert_eq!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
            Some(bus.as_str()),
            "refusing: not on the isolated bus"
        );
        let name = "session";
        let (now, before) = (
            SystemKeyring("org.example.Renamed"),
            SystemKeyring("dev.quince.client"),
        );
        before.set(name, "{\"trust\":\"isolated\"}").unwrap();
        assert_eq!(now.get(name).unwrap(), None);
        let read = read_moving(&now, &[&before], name, None).unwrap();
        assert_eq!(read.as_deref(), Some("{\"trust\":\"isolated\"}"));
        assert_eq!(
            now.get(name).unwrap().as_deref(),
            Some("{\"trust\":\"isolated\"}")
        );
        assert_eq!(before.get(name).unwrap(), None);
        now.delete(name).unwrap();

        // A client nobody configured (a probe's own library): through Store, as it reads it.
        let dir = std::env::temp_dir().join(format!("quince-probe-{}", std::process::id()));
        let store = Store::keyring(&dir).unwrap();
        let entry = entry_name(&dir);
        assert_ne!(entry, KEYRING_ENTRY, "a probe has its own entry");
        let (now, before) = (current().unwrap(), SystemKeyring(PREVIOUS_IDENTIFIERS[0]));
        before.set(&entry, "{}").unwrap();
        assert!(store.try_load().is_ok(), "read, never failed");
        assert_eq!(
            now.get(&entry).unwrap().as_deref(),
            Some("{}"),
            "moved in place"
        );
        assert_eq!(before.get(&entry).unwrap(), None);
        // A second run: read from the current service, nothing lost.
        assert!(store.try_load().is_ok());
        assert_eq!(now.get(&entry).unwrap().as_deref(), Some("{}"));
        now.delete(&entry).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
