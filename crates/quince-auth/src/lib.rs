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
use session::{Session, Store, CAPTURED_HEADERS};
use std::path::Path;
use std::sync::Arc;

const AUTH_ROOT: &str = "https://idmsa.apple.com";
const AUTH: &str = "https://idmsa.apple.com/appleauth/auth";
const SETUP: &str = "https://setup.icloud.com/setup/ws/1";
const HOME: &str = "https://www.icloud.com";

/// iCloud web's first-party OAuth widget key. Not a secret — it is public in the web client.
const WIDGET_KEY: &str = "d39ba9916b7251055b22c7f910e2ea796ee65e98b2ddecea8f5dde8d9d1a815d";
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
    #[error("couldn't use the system keyring: {0}")]
    Keyring(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// What `sign_in` decided.
#[derive(Debug, PartialEq, Eq)]
pub enum SignInOutcome {
    /// Signed in. Apple already trusted this install.
    Complete,
    /// Apple wants a six-digit code. Call [`Client::submit_two_factor`] next.
    NeedsTwoFactor,
}

pub struct Client {
    http: HttpClient,
    jar: Arc<CookieStoreMutex>,
    store: Store,
    session: Session,
}

impl Client {
    pub fn new(state_dir: &Path) -> Result<Self> {
        let store = Store::new(state_dir)?;
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

        // Default origin is icloud.com — what setup.icloud.com and the service endpoints expect.
        // The idmsa SRP endpoints want their own origin, and override it per request.
        let mut defaults = HeaderMap::new();
        defaults.insert("origin", HeaderValue::from_static(HOME));
        defaults.insert("referer", HeaderValue::from_static("https://www.icloud.com/"));
        defaults.insert("accept-language", HeaderValue::from_static("en-US,en;q=0.9"));

        let http = HttpClient::builder()
            .cookie_provider(Arc::clone(&jar))
            .user_agent(UA)
            .default_headers(defaults)
            .build()?;

        Ok(Self { http, jar, store, session })
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
            if let Some(v) = response.headers().get(*header).and_then(|v| v.to_str().ok()) {
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
        put("x-apple-oauth-redirect-uri", HOME);
        put("x-apple-oauth-require-grant-code", "true");
        put("x-apple-oauth-response-mode", "web_message");
        put("x-apple-oauth-response-type", "code");
        put("x-apple-oauth-state", &client_id);
        put("x-apple-widget-key", WIDGET_KEY);
        // SRP is served from idmsa, not the icloud.com default.
        put("origin", AUTH_ROOT);
        put("referer", &format!("{AUTH_ROOT}/"));
        if let Some(s) = self.session.scnt.clone() {
            put("scnt", &s);
        }
        if let Some(s) = self.session.session_id.clone() {
            put("x-apple-id-session-id", &s);
        }
        h
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
        match self.validate().await {
            Ok(data) => {
                self.absorb(&data);
                self.persist()?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// `POST /validate` — cheap liveness check. Doubles as the session heartbeat.
    async fn validate(&mut self) -> Result<Value> {
        let r = self
            .http
            .post(format!("{SETUP}/validate"))
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
        let client = srp::Client::new(apple_id);

        // --- /signin/init: send A, receive salt, B, protocol, iteration count ---
        let headers = self.auth_headers();
        let r = self
            .http
            .post(format!("{AUTH}/signin/init"))
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
            return Err(Error::Protocol(format!("sign-in start failed ({status})")));
        }
        let init: Value = serde_json::from_str(&text)?;

        let decode = |key: &str| -> Result<Vec<u8>> {
            let raw = init[key]
                .as_str()
                .ok_or_else(|| Error::Protocol(format!("sign-in response had no `{key}`")))?;
            B64.decode(raw).map_err(|_| Error::Protocol(format!("`{key}` wasn't valid base64")))
        };
        let salt = decode("salt")?;
        let b_pub = decode("b")?;
        let c = init["c"]
            .as_str()
            .ok_or_else(|| Error::Protocol("sign-in response had no `c`".into()))?
            .to_string();
        let iterations = init["iteration"]
            .as_u64()
            .ok_or_else(|| Error::Protocol("sign-in response had no `iteration`".into()))?
            as u32;
        let protocol_name = init["protocol"].as_str().unwrap_or("s2k");
        let protocol = srp::Protocol::parse(protocol_name)
            .ok_or_else(|| Error::UnsupportedAccount(format!("SRP protocol {protocol_name}")))?;

        // --- /signin/complete: submit the proof ---
        let proof = client
            .process_challenge(password, protocol, &salt, &b_pub, iterations)
            .ok_or_else(|| Error::Protocol("Apple's sign-in challenge failed its safety check".into()))?;

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

        let headers = self.auth_headers();
        let r = self
            .http
            .post(format!("{AUTH}/signin/complete"))
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
            StatusCode::PRECONDITION_FAILED => {
                Err(Error::UnsupportedAccount("two-factor auth isn't switched on".into()))
            }
            s => Err(Error::Protocol(format!("sign-in finish failed ({s})"))),
        }
    }

    /// Submit the six-digit code, then ask Apple to trust this install.
    pub async fn submit_two_factor(&mut self, code: &str) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .post(format!("{AUTH}/verify/trusteddevice/securitycode"))
            .headers(headers)
            .body(json!({ "securityCode": { "code": code } }).to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            // Apple uses 400/412 for a wrong code; anything else is a real protocol problem.
            return Err(match status {
                StatusCode::BAD_REQUEST | StatusCode::PRECONDITION_FAILED => Error::BadCode,
                s => Error::Protocol(format!("code check failed ({s})")),
            });
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
        let r = self.http.get(format!("{AUTH}/2sv/trust")).headers(headers).send().await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            return Err(Error::Protocol(format!("couldn't trust this device ({status})")));
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
            .post(format!("{SETUP}/accountLogin"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Error::Protocol(format!("couldn't open the account ({status})")));
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

    /// Shared HTTP client, for the service crates that ride this session.
    pub fn http(&self) -> &HttpClient {
        &self.http
    }
}
