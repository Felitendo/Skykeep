//! Apple ID sign-in, and the session that survives a restart.
//!
//! Two things make this different from every Electron iCloud wrapper:
//!
//! 1. **Auth is native.** Chromium splits WebAuthn across its render engine and its browser
//!    layer, and Electron ships only the former — so "Sign in with iPhone" hangs forever on
//!    Linux (electron/electron#24573, open since 2020). Doing SRP ourselves sidesteps that
//!    entirely; no webview is involved in authentication.
//! 2. **We keep the durable credential.** Apple hands back a trust token from `/2sv/trust`.
//!    A webview never asks for one, so it has nothing to persist and signs you out. We store
//!    it in the OS keyring.

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

pub mod releases;
pub mod session;
pub mod srp;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client as HttpClient, Response, StatusCode};
use reqwest_cookie_store::CookieStoreMutex;
use serde_json::{json, Value};
pub use session::Store;
use session::{backoff_seconds, unix_now, Session, CAPTURED_HEADERS};
use std::path::Path;
use std::sync::Arc;

/// Base URLs for Apple's auth and setup services.
///
/// Injectable so the sign-in flow can be exercised against a mock server.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// Origin for the SRP sign-in calls.
    pub auth_root: String,
    /// `{auth_root}/appleauth/auth`
    pub auth: String,
    /// `.../setup/ws/1`
    pub setup: String,
    /// Origin for everything that is not SRP sign-in.
    pub home: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth_root: "https://idmsa.apple.com".into(),
            auth: "https://idmsa.apple.com/appleauth/auth".into(),
            setup: "https://setup.icloud.com/setup/ws/1".into(),
            home: "https://www.icloud.com".into(),
        }
    }
}

impl Endpoints {
    /// Point every service at one base URL. Test seam.
    pub fn at(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth_root: base.into(),
            auth: format!("{base}/appleauth/auth"),
            setup: format!("{base}/setup/ws/1"),
            home: base.into(),
        }
    }
}

/// iCloud web's first-party OAuth widget key. Not a secret — it is public in the web client.
const WIDGET_KEY: &str = "d39ba9916b7251055b22c7f910e2ea796ee65e98b2ddecea8f5dde8d9d1a815d";
/// Bounds on Apple's PBKDF2 iteration count. Apple uses roughly 20k; anything far outside
/// this is either a corrupted response or something trying to make us burn CPU.
const MIN_PBKDF2_ITERATIONS: u32 = 1_000;
const MAX_PBKDF2_ITERATIONS: u32 = 1_000_000;

/// Version strings the iCloud web client sends. Cosmetic to Apple in practice, but sending
/// something plausible keeps our requests looking like the client they are imitating.
const CLIENT_BUILD: &str = "2523Project28";
const CLIENT_MASTERING: &str = "2523B28";

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("That Apple Account or password wasn't accepted.")]
    BadCredentials,
    #[error("That code didn't match. Check your iPhone and try again.")]
    BadCode,
    #[error("Apple didn't return a trust token, so this session wouldn't survive a restart")]
    NoTrustToken,
    #[error("this account signs in a way Skykeep can't handle yet ({0})")]
    UnsupportedAccount(String),
    /// Said without the address, which carries account and client ids.
    #[error("couldn't reach Apple: {}", if .0.is_timeout() { "it took too long to answer" } else if .0.is_connect() { "check your connection" } else { "the connection dropped" })]
    Network(#[from] reqwest::Error),
    #[error("Apple sent back something unexpected: {0}")]
    Protocol(String),
    /// Apple's servers answered but could not serve now — an outage, maintenance, too many
    /// requests (5xx, 429). Like being offline: try again later, never "signed out".
    #[error("Apple's servers are busy right now ({0}). Skykeep will try again shortly.")]
    Unavailable(u16),
    #[error("Too many attempts. Try again in {seconds} seconds.")]
    CoolingDown { seconds: u64 },
    #[error("this needs a fresh sign-in — Apple won't accept a resumed session for it")]
    ReauthRequired,
    #[error(
        "Apple is rate-limiting this account. Wait a few minutes before trying again — \
             repeated attempts risk a temporary lock"
    )]
    RateLimited,
    #[error("couldn't use the system keyring: {0}")]
    Keyring(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// A reply that says "not now" rather than "no": Apple's servers failing or overloaded.
fn transient(status: reqwest::StatusCode) -> Option<Error> {
    (status.is_server_error() || status.as_u16() == 429)
        .then(|| Error::Unavailable(status.as_u16()))
}

/// Apple returns `{"serviceErrors":[{"code":"-21669","message":"..."}]}` on failures.
/// Surface its own words; a bare status code explains nothing.
fn apple_error(context: &str, status: StatusCode, body: &str) -> Error {
    if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
        return Error::RateLimited;
    }
    // 450 is Apple's "re-authenticate for this service": a resumed session is not enough.
    if status.as_u16() == 450 {
        return Error::ReauthRequired;
    }
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    if let Some(v) = &parsed {
        if let Some(errs) = v["serviceErrors"].as_array().filter(|a| !a.is_empty()) {
            let code = errs[0]["code"].as_str().unwrap_or("");
            let message = errs[0]["message"].as_str().unwrap_or("").trim();
            // -21669 is Apple's "incorrect verification code".
            if code == "-21669" {
                return Error::BadCode;
            }
            if message.to_ascii_lowercase().contains("too many") {
                return Error::RateLimited;
            }
            if !message.is_empty() {
                return Error::Protocol(format!(
                    "{context}: {message} (Apple code {code}, HTTP {status})"
                ));
            }
        }
    }
    let snippet: String = body
        .chars()
        .take(220)
        .collect::<String>()
        .replace('\n', " ");
    Error::Protocol(format!("{context}: HTTP {status} {snippet}"))
}

/// A phone Apple will text a code to, when no trusted device responds.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedPhone {
    pub id: u64,
    /// Already masked by Apple, e.g. "+91 ••••• ••123".
    pub masked_number: String,
}

/// Where Apple is willing to send a code. Read after a `NeedsTwoFactor` outcome.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorOptions {
    /// Devices Apple pushed the code to. Zero means nothing will arrive on its own.
    pub trusted_device_count: u64,
    pub phones: Vec<TrustedPhone>,
    /// Apple refuses further sends for a while.
    pub too_many_codes_sent: bool,
}

/// What `sign_in` decided.
#[derive(Debug, PartialEq, Eq)]
pub enum SignInOutcome {
    /// Signed in. Apple already trusted this install.
    Complete,
    /// Apple wants a six-digit code. Call [`Client::submit_two_factor`] next.
    NeedsTwoFactor,
}

/// OAuth headers every idmsa call carries. Deliberately sets no Origin: the HTTP client
/// defaults to the icloud.com origin, which is what the verify, trust and phone endpoints
/// expect. Only SRP overrides it — see [`with_srp_origin`].
fn base_oauth_headers(
    client_id: &str,
    home: &str,
    scnt: Option<&str>,
    session_id: Option<&str>,
) -> HeaderMap {
    let mut h = HeaderMap::new();
    let mut put = |k: &'static str, v: &str| {
        if let Ok(val) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(k), val);
        }
    };
    put("accept", "application/json, text/javascript, */*; q=0.01");
    put("content-type", "application/json");
    put("x-apple-oauth-client-id", WIDGET_KEY);
    put("x-apple-oauth-client-type", "firstPartyAuth");
    put("x-apple-oauth-redirect-uri", home);
    put("x-apple-oauth-require-grant-code", "true");
    put("x-apple-oauth-response-mode", "web_message");
    put("x-apple-oauth-response-type", "code");
    put("x-apple-oauth-state", client_id);
    put("x-apple-widget-key", WIDGET_KEY);
    if let Some(v) = scnt {
        put("scnt", v);
    }
    if let Some(v) = session_id {
        put("x-apple-id-session-id", v);
    }
    h
}

/// `/signin/init` and `/signin/complete` are served from idmsa and reject the icloud.com
/// origin. Applying this anywhere else earns a 409.
fn with_srp_origin(mut h: HeaderMap, auth_root: &str) -> HeaderMap {
    if let Ok(v) = HeaderValue::from_str(auth_root) {
        h.insert("origin", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{auth_root}/")) {
        h.insert("referer", v);
    }
    h
}

pub struct Client {
    http: HttpClient,
    jar: Arc<CookieStoreMutex>,
    store: Store,
    session: Session,
    endpoints: Endpoints,
    /// An Apple ID and password being signed in with, kept only once the sign-in completes.
    pending: Option<(String, String)>,
    /// A lapsed session tried to sign itself back in and Apple wants a code first.
    awaiting_code: bool,
    /// The keyring would not answer when this started — locked, or not up yet. Not the same
    /// as nothing stored: it is asked again before anything else happens.
    keyring_error: Option<String>,
    /// What was last written, as a hash of the session and one of the cookies: a heartbeat that
    /// changed neither writes neither (see [`Client::persist`]).
    persisted: std::sync::Mutex<Persisted>,
}

/// Hashes of the session and the cookie jar as last written; `None` for not written yet.
#[derive(Default)]
struct Persisted {
    session: Option<u64>,
    cookies: Option<u64>,
}

fn hash_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

impl Client {
    pub fn new(state_dir: &Path) -> Result<Self> {
        Self::with(state_dir, Endpoints::default(), Store::keyring(state_dir)?)
    }

    /// Build a client against specific endpoints and a specific secret store.
    pub fn with(state_dir: &Path, endpoints: Endpoints, store: Store) -> Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let (session, keyring_error) = match store.try_load() {
            Ok(session) => (session, None),
            Err(e) => (session::Session::default(), Some(e.to_string())),
        };

        let jar = {
            let path = store.cookie_path();
            let inner = if path.exists() {
                let reader = std::io::BufReader::new(std::fs::File::open(path)?);
                cookie_store::serde::json::load(reader)
                    .unwrap_or_else(|_| cookie_store::CookieStore::default())
            } else {
                cookie_store::CookieStore::default()
            };
            Arc::new(CookieStoreMutex::new(inner))
        };

        // Default origin is icloud.com — what setup.icloud.com and the service endpoints
        // expect. The idmsa SRP endpoints want their own origin and override it per request.
        let mut defaults = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&endpoints.home) {
            defaults.insert("origin", v);
        }
        if let Ok(v) = HeaderValue::from_str(&format!("{}/", endpoints.home)) {
            defaults.insert("referer", v);
        }
        defaults.insert(
            "accept-language",
            HeaderValue::from_static("en-US,en;q=0.9"),
        );

        // Never waits for ever. After a sleep or a change of network a pooled connection can
        // be dead without saying so, and a request on it hung until the kernel gave up — a
        // quarter of an hour or more — holding the sign-in lock, so every screen waited too.
        // A read that brings nothing for a minute is given up (a long download keeps going as
        // long as bytes arrive); idle connections are let go of before they can go stale; and
        // keep-alives notice a dead one.
        let http = apple_only()
            .cookie_provider(Arc::clone(&jar))
            .user_agent(UA)
            .default_headers(defaults)
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(60))
            .pool_idle_timeout(std::time::Duration::from_secs(50))
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            http,
            jar,
            store,
            session,
            endpoints,
            pending: None,
            awaiting_code: false,
            keyring_error,
            persisted: std::sync::Mutex::new(Persisted::default()),
        })
    }

    /// A lapsed session tried to sign itself back in, and Apple wants a code before it will
    /// trust this computer again. The code screen is the next thing to show.
    #[must_use]
    pub fn awaiting_code(&self) -> bool {
        self.awaiting_code
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// True when a stored session exists and is worth trying.
    pub fn has_stored_session(&self) -> bool {
        self.session.is_resumable()
    }

    // ---- persistence ----------------------------------------------------------------

    fn capture(&mut self, response: &Response) {
        for (header, field) in CAPTURED_HEADERS {
            if let Some(v) = response
                .headers()
                .get(*header)
                .and_then(|v| v.to_str().ok())
            {
                self.session.set(*field, v.to_string());
            }
        }
    }

    /// Keep the session and its cookies where the next start finds them — each only when it
    /// differs from what was last written.
    ///
    /// Every heartbeat, so every sync and every return to the window, wrote the session to the
    /// keyring: several D-Bus writes, deletes of old names and a read-back, all while holding
    /// the sign-in lock, for a session that had not changed. Only a write that went through is
    /// remembered, so one that failed is made again next time.
    fn persist(&self) -> Result<()> {
        let session = serde_json::to_vec(&self.session)
            .map_err(|e| Error::Protocol(format!("couldn't save the session: {e}")))?;
        let mut cookies = Vec::new();
        {
            let jar = self.jar.lock().expect("cookie jar mutex poisoned");
            cookie_store::serde::json::save(&jar, &mut cookies)
                .map_err(|e| Error::Protocol(format!("couldn't save cookies: {e}")))?;
        }
        let (session, cookies) = (hash_of(&session), (hash_of(&cookies), cookies));
        let mut last = self
            .persisted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if last.session != Some(session) {
            self.store.save(&self.session)?;
            last.session = Some(session);
        }
        if last.cookies != Some(cookies.0) {
            self.write_cookies(&cookies.1)?;
            last.cookies = Some(cookies.0);
        }
        Ok(())
    }

    /// The cookie jar, as `cookie_store` wrote it, to its file.
    fn write_cookies(&self, cookies: &[u8]) -> Result<()> {
        use std::io::Write;
        // The session's cookies are a credential: readable by this user alone, whatever folder
        // they end up in (a backup, a copy), not only by the private data folder around them.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(self.store.cookie_path())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A file made before this kept its old mode: set it, too.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let mut w = std::io::BufWriter::new(file);
        w.write_all(cookies)?;
        w.flush()?;
        Ok(())
    }

    /// Forget this account entirely.
    pub fn sign_out(&mut self) -> Result<()> {
        self.session = Session::default();
        if let Ok(mut jar) = self.jar.lock() {
            jar.clear();
        }
        // Nothing is stored any more: the next session is written whatever it is.
        *self
            .persisted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Persisted::default();
        self.store.clear()
    }

    // ---- headers --------------------------------------------------------------------

    fn auth_headers(&mut self) -> HeaderMap {
        let client_id = self.session.client_id();
        base_oauth_headers(
            &client_id,
            &self.endpoints.home,
            self.session.scnt.as_deref(),
            self.session.session_id.as_deref(),
        )
    }

    /// Headers for `/signin/init` and `/signin/complete` only.
    ///
    /// Those two are served from idmsa and reject the icloud.com origin. Every other auth
    /// endpoint — verify, trust, phone — wants the icloud.com origin the client defaults to.
    fn srp_headers(&mut self) -> HeaderMap {
        let h = self.auth_headers();
        with_srp_origin(h, &self.endpoints.auth_root)
    }

    /// Headers for `/signin/init` specifically.
    ///
    /// Deliberately omits `scnt` and the session id: the flow has not issued them yet, and
    /// sending leftovers from an abandoned challenge is a conflict.
    fn srp_init_headers(&mut self) -> HeaderMap {
        let client_id = self.session.client_id();
        let h = base_oauth_headers(&client_id, &self.endpoints.home, None, None);
        with_srp_origin(h, &self.endpoints.auth_root)
    }

    /// Drop transient challenge state, keeping what identifies this install.
    ///
    /// `trust_token` and `client_id` survive — they are what lets Apple skip 2FA next time.
    /// Everything else belongs to a single sign-in attempt and must not leak into the next.
    fn clear_challenge_state(&mut self) {
        self.session.scnt = None;
        self.session.session_id = None;
        self.session.session_token = None;
    }

    // ---- the restart test -----------------------------------------------------------

    /// Try to resume a stored session without a password.
    ///
    /// Returns `Ok(true)` when the stored session still works — the behaviour the whole
    /// product promises. `Ok(false)` means Apple rejected it and a full sign-in is needed;
    /// that is an expected outcome (roughly every 60 days), not a failure.
    pub async fn resume(&mut self) -> Result<bool> {
        // A keyring that would not answer at startup is asked again; still locked, that is
        // what is said — never "signed out", which led to a sign-in replacing what it holds.
        if self.keyring_error.is_some() {
            match self.store.try_load() {
                Ok(session) => {
                    self.session = session;
                    self.keyring_error = None;
                }
                Err(e) => return Err(e),
            }
        }
        if !self.session.is_resumable() {
            return Ok(false);
        }
        // Cheap path: the cookies are still good, so nothing needs re-establishing.
        let validated = match self.validate().await {
            Ok(data) => {
                self.absorb(&data);
                self.persist()?;
                return Ok(true);
            }
            Err(e) => e,
        };

        // The cookies are missing or stale but the tokens are not. Apple's service endpoints
        // authenticate by cookie — without X-APPLE-WEBAUTH-USER they answer 421 — so trade
        // the stored tokens for a fresh set. No password, no code.
        //
        // This is what makes staying signed in survive a lost cookie file, a copied install,
        // or cookies expiring before the trust token does.
        let logged_in = match self.account_login().await {
            Ok(data) => {
                self.absorb(&data);
                self.persist()?;
                return Ok(true);
            }
            Err(e) => e,
        };
        // Unreachable is not rejected. Offline — no network yet after login, or a resume
        // from sleep mid-reconnect — the session is left exactly as it is and the caller
        // told so; signing in again here cleared the session token, failed on the network,
        // and saved the cleared state, so every launch after showed the sign-in screen.
        // Offline, or Apple down for a while: the session is not dead, only unreachable — it
        // is kept and tried again, rather than a sign-in that ends in "Apple signed you out".
        let away = |e: &Error| matches!(e, Error::Network(_) | Error::Unavailable(_));
        if away(&validated) && away(&logged_in) {
            return Err(logged_in);
        }

        // The session token has lapsed too — Apple keeps one for days, not months. With the
        // password kept, sign in again: this computer is still trusted, so Apple asks for no
        // code, and nobody sees a sign-in screen. Only when the trust itself has run out
        // (about two months) does Apple want a code, and then the code screen is what shows.
        if let (Some(apple_id), Some(password)) =
            (self.session.apple_id.clone(), self.session.password.clone())
        {
            let before = self.session.clone();
            match self.sign_in(&apple_id, &password).await {
                Ok(SignInOutcome::Complete) => return Ok(true),
                // Lost the network part way: what was kept is put back as it was.
                Err(Error::Network(e)) => {
                    self.session = before;
                    let _ = self.persist();
                    return Err(Error::Network(e));
                }
                Ok(SignInOutcome::NeedsTwoFactor) => {
                    self.awaiting_code = true;
                    return Ok(false);
                }
                // The password was changed elsewhere: the kept one is worthless now.
                Err(Error::BadCredentials) => {
                    self.session.password = None;
                    self.persist()?;
                }
                Err(_) => {}
            }
        }

        // Genuinely dead. Drop the challenge state so it cannot contaminate the sign-in that
        // follows.
        self.clear_challenge_state();
        Ok(false)
    }

    /// `POST /validate` — cheap liveness check. Doubles as the session heartbeat.
    async fn validate(&mut self) -> Result<Value> {
        let r = self
            .http
            .post(format!("{}/validate", self.endpoints.setup))
            .header("content-type", "application/json")
            .body("null")
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(transient(status)
                .unwrap_or_else(|| Error::Protocol(format!("validate returned {status}"))));
        }
        Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    /// Keep the session warm.
    ///
    /// Apple resets auth cookies on long idle gaps; icloudpd's field experience is that
    /// roughly hourly contact keeps them alive. Cheap enough to run on a timer.
    pub async fn heartbeat(&mut self) -> Result<bool> {
        match self.validate().await {
            Ok(_) => {
                self.persist()?;
                Ok(true)
            }
            // Cookies lapse after an idle spell long before the session does. Giving up here
            // left a running window signed out for good — every refresh failing quietly —
            // when the stored tokens, or the kept password, would have carried on.
            Err(_) => self.resume().await,
        }
    }

    // ---- sign-in --------------------------------------------------------------------

    /// Sign in with an Apple ID and password, over SRP.
    ///
    /// The password is never sent to Apple — SRP proves knowledge of it without transmitting
    /// it. Apple retired the old cleartext endpoint; it now answers 503.
    pub async fn sign_in(&mut self, apple_id: &str, password: &str) -> Result<SignInOutcome> {
        // What this account already owes, from beside the cookies rather than from the
        // keyring — so a machine with no keyring still remembers, which is where forgetting
        // would matter most. Per account, so signing in as somebody else inherits nothing.
        let owed = self.store.cooldown_for(apple_id);
        self.session.consecutive_failures = owed.consecutive_failures;
        self.session.retry_after = owed.retry_after;

        // Kept only once Apple has accepted it all the way: see `absorb`.
        self.pending = Some((apple_id.trim().to_string(), password.to_string()));
        self.awaiting_code = false;
        let outcome = self.sign_in_inner(apple_id, password).await;
        if outcome.is_err() {
            self.pending = None;
        }
        match &outcome {
            // Reaching the two-factor step means the password was accepted: not a failure.
            Ok(_) => {
                self.session.consecutive_failures = 0;
                self.session.retry_after = None;
            }
            // Only Apple turning the attempt down counts. Not reaching Apple, or this computer
            // failing afterwards (a keyring that would not take the session), is no wrong
            // password: counted, it held a person back from trying again once it was fixed.
            Err(
                Error::CoolingDown { .. }
                | Error::Network(_)
                | Error::Keyring(_)
                | Error::Io(_)
                | Error::Json(_),
            ) => {}
            Err(e) => {
                self.session.consecutive_failures =
                    self.session.consecutive_failures.saturating_add(1);
                // Apple saying "slow down" outranks our own schedule.
                let wait = if matches!(e, Error::RateLimited) {
                    backoff_seconds(u32::MAX)
                } else {
                    backoff_seconds(self.session.consecutive_failures)
                };
                self.session.retry_after = (wait > 0).then(|| unix_now() + wait);
            }
        }
        // Written on its own and first. `persist` needs the keyring and is allowed to fail;
        // the wait must be remembered either way, or a machine without a keyring forgets it
        // every time the window closes and somebody can go on hammering Apple until the
        // account they are trying to reach is locked.
        let _ = self.store.save_cooldown(&crate::session::Cooldown {
            account: Some(crate::session::account_fingerprint(apple_id)),
            consecutive_failures: self.session.consecutive_failures,
            retry_after: self.session.retry_after,
        });
        let _ = self.persist();
        outcome
    }

    async fn sign_in_inner(&mut self, apple_id: &str, password: &str) -> Result<SignInOutcome> {
        // Refuse before touching Apple. Hammering sign-in is what gets an account
        // temporarily locked, and an impatient user clicking twice should not risk that.
        if let Some(until) = self.session.retry_after {
            let now = unix_now();
            if now < until {
                return Err(Error::CoolingDown {
                    seconds: until - now,
                });
            }
        }

        // Start clean. An interrupted previous attempt would otherwise announce a
        // half-finished session and Apple would reject this one as a conflict.
        self.clear_challenge_state();

        let client = srp::Client::new(apple_id);

        // --- /signin/init: send A, receive salt, B, protocol, iteration count ---
        let headers = self.srp_init_headers();
        let r = self
            .http
            .post(format!("{}/signin/init", self.endpoints.auth))
            .headers(headers)
            .body(
                json!({
                    "a": B64.encode(client.a_public_bytes()),
                    "accountName": apple_id,
                    "protocols": ["s2k", "s2k_fo"],
                })
                .to_string(),
            )
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(Error::BadCredentials);
        }
        if !status.is_success() {
            return Err(apple_error("starting sign-in", status, &text));
        }
        let init: Value = serde_json::from_str(&text)?;

        let decode = |key: &str| -> Result<Vec<u8>> {
            let raw = init[key]
                .as_str()
                .ok_or_else(|| Error::Protocol(format!("sign-in response had no `{key}`")))?;
            B64.decode(raw)
                .map_err(|_| Error::Protocol(format!("`{key}` wasn't valid base64")))
        };
        let salt = decode("salt")?;
        let b_pub = decode("b")?;
        let c = init["c"]
            .as_str()
            .ok_or_else(|| Error::Protocol("sign-in response had no `c`".into()))?
            .to_string();
        let iterations = {
            let raw = init["iteration"]
                .as_u64()
                .ok_or_else(|| Error::Protocol("sign-in response had no `iteration`".into()))?;
            u32::try_from(raw)
                .ok()
                .filter(|n| (MIN_PBKDF2_ITERATIONS..=MAX_PBKDF2_ITERATIONS).contains(n))
                .ok_or_else(|| {
                    Error::Protocol(format!(
                        "Apple asked for an implausible {raw} key-derivation rounds"
                    ))
                })?
        };
        let protocol_name = init["protocol"].as_str().unwrap_or("s2k");
        let protocol = srp::Protocol::parse(protocol_name)
            .ok_or_else(|| Error::UnsupportedAccount(format!("SRP protocol {protocol_name}")))?;

        // --- /signin/complete: submit the proof ---
        let proof = client
            .process_challenge(password, protocol, &salt, &b_pub, iterations)
            .ok_or_else(|| {
                Error::Protocol("Apple's sign-in challenge failed its safety check".into())
            })?;

        let mut body = json!({
            "accountName": apple_id,
            "c": c,
            "m1": B64.encode(proof.m1),
            "m2": B64.encode(proof.m2),
            "rememberMe": true,
            "trustTokens": [],
        });
        if let Some(t) = &self.session.trust_token {
            body["trustTokens"] = json!([t]);
        }

        let headers = self.srp_headers();
        let r = self
            .http
            .post(format!("{}/signin/complete", self.endpoints.auth))
            .query(&[("isRememberMeEnabled", "true")])
            .headers(headers)
            .body(body.to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();

        match status {
            StatusCode::CONFLICT => {
                self.persist()?;
                // Ask for the code. Apple used to push it to trusted devices on this 409 by
                // itself; since iOS 26.4 it waits to be asked (PUT verify/trusteddevice/
                // securitycode), and until asked no "Allow sign-in?" ever reaches a phone —
                // which is why the only way a code ever arrived here was Airplane Mode and
                // Settings, or pressing "Send again". Not fatal if it fails: the code screen
                // still offers sending it again, and a text.
                if let Err(e) = self.resend_to_devices().await {
                    quiet!("asking Apple to send the code: {e}");
                }
                Ok(SignInOutcome::NeedsTwoFactor)
            }
            s if s.is_success() => {
                let data = self.account_login().await?;
                self.absorb(&data);
                self.persist()?;
                Ok(SignInOutcome::Complete)
            }
            StatusCode::UNAUTHORIZED => Err(Error::BadCredentials),
            StatusCode::PRECONDITION_FAILED => Err(Error::UnsupportedAccount(
                "two-factor auth isn't switched on".into(),
            )),
            s => Err(apple_error("finishing sign-in", s, &text)),
        }
    }

    /// Where Apple can send a code, and whether it already pushed one.
    ///
    /// Worth reading whenever a code doesn't arrive: a `trusted_device_count` of zero means
    /// no push was ever sent and SMS is the only route.
    pub async fn two_factor_options(&mut self) -> Result<TwoFactorOptions> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .get(&self.endpoints.auth)
            .headers(headers)
            .send()
            .await?;
        self.capture(&r);
        let text = r.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

        let phones = v["trustedPhoneNumbers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        Some(TrustedPhone {
                            id: p["id"].as_u64()?,
                            masked_number: p["numberWithDialCode"]
                                .as_str()
                                .or_else(|| p["obfuscatedNumber"].as_str())
                                .unwrap_or("(hidden)")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(TwoFactorOptions {
            trusted_device_count: v["trustedDeviceCount"].as_u64().unwrap_or(0),
            phones,
            too_many_codes_sent: v["securityCode"]["tooManyCodesSent"]
                .as_bool()
                .unwrap_or(false),
        })
    }

    /// Ask Apple to push the code to trusted devices again.
    pub async fn resend_to_devices(&mut self) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .put(format!(
                "{}/verify/trusteddevice/securitycode",
                self.endpoints.auth
            ))
            .headers(headers)
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            return Err(Error::Protocol(format!(
                "couldn't resend the code ({status})"
            )));
        }
        Ok(())
    }

    /// Text the code to one of the trusted phone numbers.
    pub async fn send_sms(&mut self, phone_id: u64) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .put(format!("{}/verify/phone", self.endpoints.auth))
            .headers(headers)
            .body(json!({ "phoneNumber": { "id": phone_id }, "mode": "sms" }).to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            return Err(Error::Protocol(format!(
                "couldn't send the text ({status})"
            )));
        }
        Ok(())
    }

    /// Submit a code that arrived by SMS. Trusted-device codes use `submit_two_factor`.
    pub async fn submit_sms_code(&mut self, phone_id: u64, code: &str) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .post(format!("{}/verify/phone/securitycode", self.endpoints.auth))
            .headers(headers)
            .body(
                json!({
                    "phoneNumber": { "id": phone_id },
                    "securityCode": { "code": code },
                    "mode": "sms",
                })
                .to_string(),
            )
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let err = apple_error("checking that texted code", status, &body);
            if matches!(err, Error::BadCode) {
                return Err(err);
            }
            if self.trust().await.is_ok() {
                let data = self.account_login().await?;
                self.absorb(&data);
                self.persist()?;
                return Ok(());
            }
            return Err(err);
        }
        self.trust().await?;
        let data = self.account_login().await?;
        self.absorb(&data);
        self.persist()?;
        Ok(())
    }

    /// Submit the six-digit code, then ask Apple to trust this install.
    pub async fn submit_two_factor(&mut self, code: &str) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .post(format!(
                "{}/verify/trusteddevice/securitycode",
                self.endpoints.auth
            ))
            .headers(headers)
            .body(json!({ "securityCode": { "code": code } }).to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        // Since iOS 26.4 a correct code is answered 409 with `securityCode.valid: true`.
        let accepted = status == StatusCode::CONFLICT
            && serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/securityCode/valid").and_then(Value::as_bool))
                == Some(true);
        if !status.is_success() && status != StatusCode::NO_CONTENT && !accepted {
            let err = apple_error("checking that code", status, &body);
            // A genuinely wrong code is final; don't burn an attempt probing further.
            if matches!(err, Error::BadCode) {
                return Err(err);
            }
            // Apple answers 409 on this endpoint in some flows even though the code WAS
            // accepted. The status alone is not evidence, so ask whether the session became
            // usable before reporting a failure.
            if self.trust().await.is_ok() {
                let data = self.account_login().await?;
                self.absorb(&data);
                self.persist()?;
                return Ok(());
            }
            return Err(err);
        }

        self.trust().await?;
        let data = self.account_login().await?;
        self.absorb(&data);
        self.persist()?;
        Ok(())
    }

    /// `GET /2sv/trust` — yields the trust token. Without it, sign-in wouldn't outlive the process.
    async fn trust(&mut self) -> Result<()> {
        let headers = self.auth_headers();
        let r = self
            .http
            .get(format!("{}/2sv/trust", self.endpoints.auth))
            .headers(headers)
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            return Err(apple_error("trusting this device", status, &body));
        }
        if self.session.trust_token.is_none() {
            return Err(Error::NoTrustToken);
        }
        Ok(())
    }

    /// Exchange the session token for Apple's per-account service URL map.
    async fn account_login(&mut self) -> Result<Value> {
        let body = json!({
            "accountCountryCode": self.session.account_country.clone().unwrap_or_default(),
            "dsWebAuthToken": self.session.session_token.clone().unwrap_or_default(),
            "extended_login": true,
            "trustToken": self.session.trust_token.clone().unwrap_or_default(),
        });
        let r = self
            .http
            .post(format!("{}/accountLogin", self.endpoints.setup))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(transient(status).unwrap_or_else(|| {
                Error::Protocol(format!("couldn't open the account ({status})"))
            }));
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn absorb(&mut self, data: &Value) {
        // A completed sign-in — with or without a code — is when a password has proved
        // itself, so that is when it is kept.
        if let Some((apple_id, password)) = self.pending.take() {
            self.session.apple_id = Some(apple_id);
            self.session.password = Some(password);
            self.awaiting_code = false;
        }
        if let Some(ws) = data.get("webservices") {
            self.session.webservices = Some(ws.clone());
        }
        if let Some(d) = data.pointer("/dsInfo/dsid").and_then(|v| v.as_str()) {
            self.session.dsid = Some(d.to_string());
        }
        let text = |path: &str| {
            data.pointer(path)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        if let Some(name) = text("/dsInfo/fullName").or_else(|| {
            let first = text("/dsInfo/firstName")?;
            Some(match text("/dsInfo/lastName") {
                Some(last) => format!("{first} {last}"),
                None => first,
            })
        }) {
            self.session.account_name = Some(name);
        }
        if let Some(email) = text("/dsInfo/primaryEmail").or_else(|| text("/dsInfo/appleId")) {
            self.session.account_email = Some(email);
        }
        if let Some(allowed) = data
            .pointer("/dsInfo/isWebAccessAllowed")
            .and_then(Value::as_bool)
        {
            self.session.web_access_allowed = Some(allowed);
        }
        if let Some(on) = data
            .get("iCDPEnabled")
            .or_else(|| data.pointer("/dsInfo/iCDPEnabled"))
            .and_then(Value::as_bool)
        {
            self.session.data_protection = Some(on);
        }
    }

    /// Query parameters every iCloud service endpoint expects alongside the session cookies.
    #[must_use]
    pub fn service_params(&self) -> Vec<(String, String)> {
        let mut params = vec![
            ("clientBuildNumber".to_string(), CLIENT_BUILD.to_string()),
            (
                "clientMasteringNumber".to_string(),
                CLIENT_MASTERING.to_string(),
            ),
            (
                "clientId".to_string(),
                self.session.client_id.clone().unwrap_or_default(),
            ),
        ];
        if let Some(dsid) = &self.session.dsid {
            params.push(("dsid".to_string(), dsid.clone()));
        }
        params
    }

    /// The account service's root (`…/setup/ws/1`), where storage use is asked for.
    #[must_use]
    pub fn setup_url(&self) -> &str {
        &self.endpoints.setup
    }

    /// Shared HTTP client, for the service crates that ride this session.
    pub fn http(&self) -> &HttpClient {
        &self.http
    }
}

/// The domains Apple's client can reach, each with every host under it: Apple's sign-in,
/// iCloud's services and their files, and push. What Settings
/// lists as what Skykeep talks to, read from here so the list is the lock itself.
pub const APPLE_DOMAINS: [&str; 3] = ["apple.com", "icloud.com", "icloud-content.com"];

/// Whether this computer counts as Apple's: in this crate's tests, and in a build made with the
/// `loopback` feature (the other crates' tests, whose mock servers run here) — never in the
/// app. A shipped client that reached `localhost` could be pointed, by an address in Apple's
/// data (a contact's photo), at whatever else runs on the person's computer.
const LOOPBACK: bool = cfg!(any(test, feature = "loopback"));

/// Whether `host` is Apple's: under one of [`APPLE_DOMAINS`] — every host Skykeep has any
/// reason to reach. Loopback too, which is this computer, only in tests (see `LOOPBACK`).
#[must_use]
pub fn is_apple_host(host: &str) -> bool {
    apple_host(host, LOOPBACK)
}

fn apple_host(host: &str, loopback: bool) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (loopback && host == "localhost")
        || APPLE_DOMAINS
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Every HTTP client Skykeep makes for iCloud starts here: Apple's servers, and nothing else,
/// can be reached at all (see [`AppleOnly`]). The two other clients are just as narrow:
/// [`listed_only`] for subscribed calendars and [`releases::releases_only`] for updates.
pub fn apple_only() -> reqwest::ClientBuilder {
    let builder = HttpClient::builder()
        .dns_resolver(Arc::new(AppleOnly))
        // The resolver alone is not enough: a URL with an address in it (`https://1.2.3.4/`)
        // is never looked up, and through a proxy the proxy looks the name up, not us. So
        // every request's own URL is checked first, a redirect's included; one that is not
        // Apple's is sent to a "proxy" that cannot be found — by the resolver above, so the
        // refusal reads the same — and never leaves the machine.
        .proxy(reqwest::Proxy::custom(|url| {
            (!url_is_apples(url)).then(|| REFUSED.to_string())
        }));
    // Setting a proxy turns off the system's own (`HTTPS_PROXY` and the rest), which Apple's
    // hosts still go through when one is set, as they did before.
    with_system_proxies(builder)
}

/// The proxy the system names for `scheme` ("https" or "http"), from `HTTPS_PROXY` and the
/// rest, if any.
pub(crate) fn env_proxy(scheme: &str) -> Option<String> {
    let names = if scheme == "https" {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    names
        .iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.trim().is_empty()))
}

/// The system's own proxies (`HTTPS_PROXY` and the rest), put back after a refusing proxy has
/// turned them off.
fn with_system_proxies(mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    for scheme in ["https", "http"] {
        let Some(via) = env_proxy(scheme) else {
            continue;
        };
        let proxy =
            reqwest::Proxy::custom(move |url| (url.scheme() == scheme).then(|| via.clone()))
                .no_proxy(reqwest::NoProxy::from_env());
        builder = builder.proxy(proxy);
    }
    builder
}

/// The client for calendars the person subscribed to by address — and for nothing else. It
/// reaches exactly the hosts in `hosts` (the ones those addresses name), by the same two locks
/// as [`apple_only`]: a resolver that looks up no other name, and a check of every request's
/// own URL, a redirect's included, so an address or a proxy cannot carry it anywhere else.
/// Apple's own client is not widened by it; nothing else in Skykeep uses this one.
pub fn listed_only(hosts: &[String]) -> reqwest::ClientBuilder {
    let listed: Arc<Vec<String>> = Arc::new(hosts.iter().map(|h| normal_host(h)).collect());
    let check = Arc::clone(&listed);
    let builder = HttpClient::builder()
        .dns_resolver(Arc::new(ListedOnly(Arc::clone(&listed))))
        .redirect(reqwest::redirect::Policy::limited(5))
        .proxy(reqwest::Proxy::custom(move |url| {
            (!url_is_listed(url, &check)).then(|| REFUSED.to_string())
        }));
    with_system_proxies(builder)
}

fn normal_host(host: &str) -> String {
    host.trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

/// Whether a request's URL names one of the listed hosts exactly — and, where it names an
/// address, one out on the internet.
fn url_is_listed(url: &reqwest::Url, listed: &[String]) -> bool {
    url.host_str().is_some_and(|h| {
        let h = normal_host(h);
        listed.contains(&h) && h.parse::<std::net::IpAddr>().ok().is_none_or(is_public_ip)
    })
}

/// Whether an address is out on the internet: not this computer, not the local network (a
/// router's page, a printer, `192.168.…`, `10.…`, `fd…`), not link-local (where cloud machines
/// keep their secrets, `169.254.169.254`), nor any other range no public calendar is on. A
/// subscription is an address anyone can hand the person — and its name can be pointed at any
/// address its owner likes — so one that leads inside is never fetched.
#[must_use]
pub fn is_public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || a >= 240
                // Shared address space (carrier NAT, tailnets) and benchmarking.
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (18..20).contains(&b))
                // IETF protocol assignments.
                || (a == 192 && b == 0 && v4.octets()[2] == 0))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            // NAT64 (`64:ff9b::/96`) carries an IPv4 address inside.
            if v6.segments()[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., a, b, c, d] = v6.octets();
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                || first == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

/// The subscribed-calendar client's resolver: a listed host is looked up as the system looks
/// up any name; any other is never looked up at all.
struct ListedOnly(Arc<Vec<String>>);

impl reqwest::dns::Resolve for ListedOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = normal_host(name.as_str());
        let allowed = self.0.contains(&host);
        Box::pin(async move {
            if !allowed {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not a calendar you subscribed to").into();
                return Err(refused);
            }
            // Whatever the name says, only addresses out on the internet are dialled: a feed
            // whose name leads to this computer or the local network reaches neither.
            let found: Vec<_> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| is_public_ip(a.ip()))
                .collect();
            if found.is_empty() {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not on the internet").into();
                return Err(refused);
            }
            let addrs: reqwest::dns::Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// Where a request that is not Apple's is pointed: a name the resolver refuses.
const REFUSED: &str = "http://refused.skykeep.invalid";

/// Whether a request's URL may be sent: an Apple host by name, or this computer in tests.
fn url_is_apples(url: &reqwest::Url) -> bool {
    url_apples(url, LOOPBACK)
}

fn url_apples(url: &reqwest::Url, loopback: bool) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    match host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
    {
        Ok(ip) => loopback && ip.is_loopback(),
        Err(_) => apple_host(host, loopback),
    }
}

/// The one way Apple's client finds a server: a host that is not Apple's is never looked up,
/// so no request of Apple's client can reach one — how the code works, not a promise about it.
/// Apple's are looked up as the system looks up any name.
struct AppleOnly;

impl reqwest::dns::Resolve for AppleOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            if !is_apple_host(&host) {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not Apple's, and Apple's client reaches only Apple").into();
                return Err(refused);
            }
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let addrs: reqwest::dns::Addrs = Box::new(found.collect::<Vec<_>>().into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    /// What the shipped client allows: Apple's hosts, and never this computer — by name or
    /// by address.
    #[test]
    fn a_release_client_never_reaches_this_computer() {
        for host in ["localhost", "LOCALHOST.", "127.0.0.1", "[::1]"] {
            assert!(!apple_host(host, false), "{host}");
        }
        for url in [
            "http://localhost:8080/",
            "http://127.0.0.1:631/",
            "http://127.1.2.3/",
            "http://[::1]:9/",
            "https://localhost./",
        ] {
            let u = reqwest::Url::parse(url).unwrap();
            assert!(!url_apples(&u, false), "{url}");
            assert!(url_apples(&u, true), "{url}");
        }
        assert!(url_apples(
            &reqwest::Url::parse("https://p01-contacts.icloud.com/x").unwrap(),
            false
        ));
    }
}

#[cfg(test)]
mod listed_tests {
    use super::*;

    /// A subscribed calendar's client reaches its own hosts and refuses every other — by
    /// name, by address, and Apple's too — before anything leaves the machine.
    #[tokio::test]
    async fn the_subscription_client_refuses_a_host_nobody_subscribed_to() {
        let http = listed_only(&["calendar.example.org".into()])
            .build()
            .unwrap();
        for url in [
            "https://elsewhere.example.net/feed.ics",
            "https://93.184.216.34/feed.ics",
            "https://www.icloud.com/",
        ] {
            let err = http.get(url).send().await.expect_err(url);
            let text = format!("{err:?}");
            assert!(
                text.contains("not a calendar you subscribed to"),
                "{url} was not refused by the list: {text}"
            );
        }
    }

    #[test]
    fn a_listed_host_is_matched_exactly_and_without_case() {
        let listed = vec![normal_host("Calendar.Example.org.")];
        let ok = |u: &str| url_is_listed(&reqwest::Url::parse(u).unwrap(), &listed);
        assert!(ok("https://calendar.example.org/a.ics"));
        assert!(ok("http://CALENDAR.example.org:8080/a.ics"));
        assert!(!ok("https://evil.calendar.example.org/a.ics"));
        assert!(!ok("https://example.org/a.ics"));
        assert!(!ok("https://calendar.example.org.evil.net/a.ics"));
    }

    /// A subscription never reaches this computer or the local network: not by an address in
    /// it, not by a name that leads there.
    #[tokio::test]
    async fn a_subscription_never_reaches_inside() {
        use std::net::IpAddr;
        for inside in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.4",
            "192.168.1.1",
            "169.254.169.254",
            "100.100.100.100",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
            "64:ff9b::a9fe:a9fe",
        ] {
            assert!(!is_public_ip(inside.parse::<IpAddr>().unwrap()), "{inside}");
        }
        for out in ["93.184.215.14", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_ip(out.parse::<IpAddr>().unwrap()), "{out}");
        }
        // Listed, and still refused: an address inside, or a name that leads inside.
        let by_address = vec![normal_host("192.168.1.1"), normal_host("[::1]")];
        for u in ["http://192.168.1.1/a.ics", "http://[::1]:8080/a.ics"] {
            assert!(
                !url_is_listed(&reqwest::Url::parse(u).unwrap(), &by_address),
                "{u}"
            );
        }
        let http = listed_only(&["localhost".into()]).build().unwrap();
        let err = http
            .get("http://localhost:9/feed.ics")
            .send()
            .await
            .expect_err("a name that leads to this computer");
        assert!(
            format!("{err:?}").contains("not on the internet"),
            "{err:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME_T: &str = "https://www.icloud.com";
    const ROOT_T: &str = "https://idmsa.apple.com";

    /// Apple answers 409 when the idmsa origin is sent to an endpoint that expects the
    /// icloud.com one. Only the two SRP calls may carry it.
    #[test]
    fn srp_origin_is_scoped_to_sign_in() {
        let base = base_oauth_headers("auth-test", HOME_T, None, None);
        assert!(
            base.get("origin").is_none(),
            "base headers must not pin an Origin; the client default (icloud.com) applies"
        );

        let srp = with_srp_origin(base_oauth_headers("auth-test", HOME_T, None, None), ROOT_T);
        assert_eq!(srp.get("origin").unwrap(), ROOT_T);
        assert_eq!(srp.get("referer").unwrap(), &format!("{ROOT_T}/")[..]);
    }

    #[test]
    fn session_headers_are_forwarded_when_present() {
        let h = base_oauth_headers(
            "auth-test",
            HOME_T,
            Some("scnt-value"),
            Some("session-value"),
        );
        assert_eq!(h.get("scnt").unwrap(), "scnt-value");
        assert_eq!(h.get("x-apple-id-session-id").unwrap(), "session-value");
    }

    /// Sign-in must not advertise a session that does not exist yet.
    #[test]
    fn init_headers_omit_challenge_state() {
        let stale = base_oauth_headers(
            "auth-test",
            HOME_T,
            Some("stale-scnt"),
            Some("stale-session"),
        );
        assert!(
            stale.get("scnt").is_some(),
            "sanity: the helper does forward them"
        );

        let init = with_srp_origin(base_oauth_headers("auth-test", HOME_T, None, None), ROOT_T);
        assert!(
            init.get("scnt").is_none(),
            "/signin/init must not send scnt"
        );
        assert!(
            init.get("x-apple-id-session-id").is_none(),
            "/signin/init must not send a session id"
        );
        assert_eq!(init.get("origin").unwrap(), ROOT_T);
    }

    /// The first retry should be nearly free; repeated failures must back off hard, because
    /// the penalty for hammering Apple is a temporary account lock.
    #[test]
    fn backoff_grows_and_is_capped() {
        use crate::session::backoff_seconds;
        assert_eq!(backoff_seconds(0), 0, "a first attempt must not be delayed");
        let steps: Vec<u64> = (1..=6).map(backoff_seconds).collect();
        for pair in steps.windows(2) {
            assert!(pair[1] >= pair[0], "backoff must never shrink: {steps:?}");
        }
        assert_eq!(
            backoff_seconds(u32::MAX),
            backoff_seconds(5),
            "and it must be bounded"
        );
    }

    #[test]
    fn apple_errors_are_classified() {
        let wrong =
            r#"{"serviceErrors":[{"code":"-21669","message":"Incorrect verification code."}]}"#;
        assert!(matches!(
            apple_error("x", StatusCode::CONFLICT, wrong),
            Error::BadCode
        ));

        let throttled = r#"{"serviceErrors":[{"code":"-22222","message":"Too many codes sent."}]}"#;
        assert!(matches!(
            apple_error("x", StatusCode::CONFLICT, throttled),
            Error::RateLimited
        ));

        assert!(matches!(
            apple_error("x", StatusCode::TOO_MANY_REQUESTS, ""),
            Error::RateLimited
        ));

        // Find My answers 450 on a resumed session; that is "sign in again", not a failure.
        assert!(matches!(
            apple_error("x", StatusCode::from_u16(450).unwrap(), ""),
            Error::ReauthRequired
        ));

        // Anything unrecognised must keep Apple's own words rather than flatten to a status.
        let other = r#"{"serviceErrors":[{"code":"-20101","message":"Account locked."}]}"#;
        match apple_error("signing in", StatusCode::CONFLICT, other) {
            Error::Protocol(m) => {
                assert!(m.contains("Account locked."), "got: {m}");
                assert!(m.contains("-20101"), "got: {m}");
            }
            e => panic!("expected Protocol, got {e:?}"),
        }
    }
}
