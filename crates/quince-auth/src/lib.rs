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
/// Injectable so the sign-in flow can be exercised against a mock server, and so Apple's
/// China domains are a constructor rather than a fork.
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
    /// Accounts hosted in mainland China.
    pub fn china() -> Self {
        Self {
            auth_root: "https://idmsa.apple.com.cn".into(),
            auth: "https://idmsa.apple.com.cn/appleauth/auth".into(),
            setup: "https://setup.icloud.com.cn/setup/ws/1".into(),
            home: "https://www.icloud.com.cn".into(),
        }
    }

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
    #[error("that Apple ID or password wasn't accepted")]
    BadCredentials,
    #[error("that code didn't match. Check your iPhone and try again")]
    BadCode,
    #[error("Apple didn't return a trust token, so this session wouldn't survive a restart")]
    NoTrustToken,
    #[error("this account signs in a way Quince can't handle yet ({0})")]
    UnsupportedAccount(String),
    #[error("couldn't reach Apple: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Apple sent back something unexpected: {0}")]
    Protocol(String),
    #[error("too many attempts just now — try again in {seconds}s")]
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
}

impl Client {
    pub fn new(state_dir: &Path) -> Result<Self> {
        Self::with(state_dir, Endpoints::default(), Store::keyring(state_dir)?)
    }

    /// Build a client against specific endpoints and a specific secret store.
    pub fn with(state_dir: &Path, endpoints: Endpoints, store: Store) -> Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let session = store.load();

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

        let http = HttpClient::builder()
            .cookie_provider(Arc::clone(&jar))
            .user_agent(UA)
            .default_headers(defaults)
            .build()?;

        Ok(Self {
            http,
            jar,
            store,
            session,
            endpoints,
        })
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

    fn persist(&self) -> Result<()> {
        self.store.save(&self.session)?;
        let mut w = std::io::BufWriter::new(std::fs::File::create(self.store.cookie_path())?);
        let jar = self.jar.lock().expect("cookie jar mutex poisoned");
        cookie_store::serde::json::save(&jar, &mut w)
            .map_err(|e| Error::Protocol(format!("couldn't save cookies: {e}")))
    }

    /// Forget this account entirely.
    pub fn sign_out(&mut self) -> Result<()> {
        self.session = Session::default();
        if let Ok(mut jar) = self.jar.lock() {
            jar.clear();
        }
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
        if !self.session.is_resumable() {
            return Ok(false);
        }
        // Cheap path: the cookies are still good, so nothing needs re-establishing.
        if let Ok(data) = self.validate().await {
            self.absorb(&data);
            self.persist()?;
            return Ok(true);
        }

        // The cookies are missing or stale but the tokens are not. Apple's service endpoints
        // authenticate by cookie — without X-APPLE-WEBAUTH-USER they answer 421 — so trade
        // the stored tokens for a fresh set. No password, no code.
        //
        // This is what makes staying signed in survive a lost cookie file, a copied install,
        // or cookies expiring before the trust token does.
        match self.account_login().await {
            Ok(data) => {
                self.absorb(&data);
                self.persist()?;
                Ok(true)
            }
            Err(_) => {
                // Genuinely dead. Drop the challenge state so it cannot contaminate the
                // sign-in that follows.
                self.clear_challenge_state();
                Ok(false)
            }
        }
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
            return Err(Error::Protocol(format!("validate returned {status}")));
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
            Err(_) => Ok(false),
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

        let outcome = self.sign_in_inner(apple_id, password).await;
        match &outcome {
            // Reaching the two-factor step means the password was accepted: not a failure.
            Ok(_) => {
                self.session.consecutive_failures = 0;
                self.session.retry_after = None;
            }
            Err(Error::CoolingDown { .. }) => {}
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
        if !status.is_success() && status != StatusCode::NO_CONTENT {
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
            return Err(Error::Protocol(format!(
                "couldn't open the account ({status})"
            )));
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn absorb(&mut self, data: &Value) {
        if let Some(ws) = data.get("webservices") {
            self.session.webservices = Some(ws.clone());
        }
        if let Some(d) = data.pointer("/dsInfo/dsid").and_then(|v| v.as_str()) {
            self.session.dsid = Some(d.to_string());
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

    /// Shared HTTP client, for the service crates that ride this session.
    pub fn http(&self) -> &HttpClient {
        &self.http
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
