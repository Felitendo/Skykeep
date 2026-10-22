//! Apple's push "web courier": the live channel icloud.com uses today to hear that something
//! changed on another device.
//!
//! The older path — a token from `ckdeviceservice …/tokens/create`, long-polled on its
//! `webcourierURL` — is retired: Apple answers it `403 ACCESS_DENIED, "This endpoint is
//! deprecated."`, and the `push` service's `getToken` that the shell's own long-poll fallback
//! used no longer resolves at all. What icloud.com's shell (`push-service-sharedworker`, build
//! 2638Build27, read 2026-10-22) does instead, and this module does the same:
//!
//! 1. Make a P-256 key. Sign a nonce — `[0, unix-ms as 8 bytes big-endian, 8 random bytes]` —
//!    with ECDSA-SHA256; the signature goes as `[1, 3]` followed by its DER.
//! 2. Open `wss://websocket.push.apple.com/v2/<hex of a WebClientConnectionMessage>` carrying
//!    the public key, nonce, signature and `newConnectionRequest { tokenExpirationSeconds }`
//!    (or `connectWithPushToken` to keep a token across a reconnect).
//! 3. The first frame back is a `connectionResponseMessage`: a base64 push token, or
//!    `INVALID_NONCE` with the server's clock to sign against instead.
//! 4. Send a `webFilterMessage` with the topics wanted — `com.apple.icloud-container.<id>` —
//!    and a `jwtRequestMessage`; the socket answers with a short-lived JWT.
//! 5. Register the token for those topics with the push registry ("PDS"):
//!    `POST https://gatewayws.icloud.com/pds/WebObjects/pushregistry.woa/webBatchRegister/batchRegisterToken`,
//!    a protobuf `BatchRegisterReq` signed with the same key (nonce kind 2), sent with the
//!    session's cookies. Status 0 is success.
//! 6. Pushes then arrive on the socket as `pushMessage`s — topic (or its SHA-1), id and a JSON
//!    payload like `{"ck":{"cid":"com.apple.notes", "met":{…}}}` — and each is acknowledged
//!    with its id and topic.
//!
//! Nothing is created or changed in iCloud by any of this: the account's own devices keep the
//! CloudKit subscriptions that fire, as before.

use crate::Error;
use futures_util::{SinkExt, StreamExt};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use rand_core::{OsRng, RngCore};
use serde_json::Value;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};

/// The production push server icloud.com's shell connects to.
pub const SERVER: &str = "websocket.push.apple.com";

/// The push registry the token is registered with.
pub const REGISTRY: &str =
    "https://gatewayws.icloud.com/pds/WebObjects/pushregistry.woa/webBatchRegister/batchRegisterToken";

/// How long a token asked for lives — the shell's own `86400`.
const TOKEN_SECONDS: u32 = 86_400;

/// How long a registration with the registry lasts — the shell's `43200`.
pub const REGISTRATION_SECONDS: u64 = 43_200;

/// The topic a CloudKit container's pushes come on.
#[must_use]
pub fn topic_of(container: &str) -> String {
    format!("com.apple.icloud-container.{container}")
}

/// The container a topic is for, if it is a CloudKit one.
#[must_use]
pub fn container_of_topic(topic: &str) -> Option<&str> {
    topic.strip_prefix("com.apple.icloud-container.")
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// --- protobuf, the few messages this needs ------------------------------------------------

mod pb {
    /// A protobuf writer: just the wire types these messages use.
    #[derive(Default)]
    pub struct W(pub Vec<u8>);

    impl W {
        fn varint(&mut self, mut v: u64) {
            while v >= 0x80 {
                self.0.push((v as u8) | 0x80);
                v >>= 7;
            }
            self.0.push(v as u8);
        }
        pub fn uint(&mut self, field: u32, v: u64) -> &mut Self {
            self.varint(u64::from(field) << 3);
            self.varint(v);
            self
        }
        pub fn bytes(&mut self, field: u32, v: &[u8]) -> &mut Self {
            self.varint((u64::from(field) << 3) | 2);
            self.varint(v.len() as u64);
            self.0.extend_from_slice(v);
            self
        }
        pub fn string(&mut self, field: u32, v: &str) -> &mut Self {
            self.bytes(field, v.as_bytes())
        }
        pub fn message(&mut self, field: u32, inner: &W) -> &mut Self {
            self.bytes(field, &inner.0)
        }
    }

    /// One field as read: its number and either a varint or a length-delimited slice.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum V<'a> {
        Int(u64),
        Bytes(&'a [u8]),
    }

    fn varint(buf: &[u8], at: &mut usize) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *buf.get(*at)?;
            *at += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Some(v);
            }
        }
        None
    }

    /// Every field of a message, in order. `None` for a malformed one.
    pub fn fields(buf: &[u8]) -> Option<Vec<(u32, V<'_>)>> {
        let mut at = 0;
        let mut out = Vec::new();
        while at < buf.len() {
            let key = varint(buf, &mut at)?;
            let field = u32::try_from(key >> 3).ok()?;
            match key & 7 {
                0 => out.push((field, V::Int(varint(buf, &mut at)?))),
                2 => {
                    let len = usize::try_from(varint(buf, &mut at)?).ok()?;
                    let end = at.checked_add(len).filter(|e| *e <= buf.len())?;
                    out.push((field, V::Bytes(&buf[at..end])));
                    at = end;
                }
                1 => at = at.checked_add(8).filter(|e| *e <= buf.len())?,
                5 => at = at.checked_add(4).filter(|e| *e <= buf.len())?,
                _ => return None,
            }
        }
        Some(out)
    }

    pub fn int(fields: &[(u32, V<'_>)], n: u32) -> Option<u64> {
        fields.iter().find_map(|(f, v)| match v {
            V::Int(i) if *f == n => Some(*i),
            _ => None,
        })
    }

    pub fn bytes<'a>(fields: &[(u32, V<'a>)], n: u32) -> Option<&'a [u8]> {
        fields.iter().find_map(|(f, v)| match v {
            V::Bytes(b) if *f == n => Some(*b),
            _ => None,
        })
    }
}

// --- signing --------------------------------------------------------------------------------

/// A nonce, signed: what both the socket and the registry are shown.
#[derive(Debug, Clone)]
pub struct Signed {
    pub public_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub signature: Vec<u8>,
}

/// The nonce the shell signs. `kind` is the registry's purpose byte (`pds` = 2), absent for
/// the socket's own connect message.
fn nonce(unix_ms: u64, kind: Option<u8>) -> Vec<u8> {
    let mut random = [0u8; 8];
    OsRng.fill_bytes(&mut random);
    let mut n = Vec::with_capacity(18);
    match kind {
        Some(k) => {
            n.push(1);
            n.extend_from_slice(&unix_ms.to_be_bytes());
            n.push(k);
        }
        None => {
            n.push(0);
            n.extend_from_slice(&unix_ms.to_be_bytes());
        }
    }
    n.extend_from_slice(&random);
    n
}

/// The signature as the shell lays it out: `[1, 3]`, then a DER `SEQUENCE { INTEGER r,
/// INTEGER s }` whose integers keep all 32 bytes, with a zero in front when the top bit is set.
fn shell_signature(sig: &Signature) -> Vec<u8> {
    let raw = sig.to_bytes();
    let (r, s) = raw.split_at(32);
    let int = |v: &[u8]| {
        let mut out = vec![2u8];
        let pad = v[0] > 127;
        out.push(32 + u8::from(pad));
        if pad {
            out.push(0);
        }
        out.extend_from_slice(v);
        out
    };
    let (r, s) = (int(r), int(s));
    let mut out = vec![1u8, 3, 48, (r.len() + s.len()) as u8];
    out.extend(r);
    out.extend(s);
    out
}

/// The key a connection is made with and the token Apple gave it. Kept across reconnects, so
/// a dropped socket comes back on the same token and its registrations still stand.
pub struct Identity {
    key: SigningKey,
    token: Option<Vec<u8>>,
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}

impl Identity {
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SigningKey::random(&mut OsRng),
            token: None,
        }
    }

    /// Whether Apple has given this identity a token yet.
    #[must_use]
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// Start over with no token: the next connection asks for a new one.
    pub fn forget_token(&mut self) {
        self.token = None;
    }

    #[must_use]
    pub fn sign(&self, unix_ms: u64, kind: Option<u8>) -> Signed {
        let nonce = nonce(unix_ms, kind);
        let sig: Signature = self.key.sign(&nonce);
        Signed {
            public_key: self
                .key
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec(),
            nonce,
            signature: shell_signature(&sig),
        }
    }

    /// The `WebClientConnectionMessage`, hex-encoded, that goes in the socket's address.
    fn connect_message(&self, unix_ms: u64) -> String {
        let s = self.sign(unix_ms, None);
        let mut v1 = pb::W::default();
        v1.bytes(1, &s.public_key)
            .bytes(2, &s.nonce)
            .bytes(3, &s.signature);
        match &self.token {
            Some(token) => {
                let mut with = pb::W::default();
                with.string(
                    1,
                    &base64::Engine::encode(&base64::engine::general_purpose::STANDARD, token),
                );
                v1.message(4, &with);
            }
            None => {
                let mut new = pb::W::default();
                new.uint(1, u64::from(TOKEN_SECONDS));
                v1.message(5, &new);
            }
        }
        let mut outer = pb::W::default();
        outer.message(1, &v1);
        hex(&outer.0)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// --- the socket -----------------------------------------------------------------------------

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// What the socket said, as far as the app cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A change in this CloudKit container.
    Changed(String),
    /// A push for something else, or one whose payload named no container.
    Other,
}

/// Why the channel could not be opened or was lost.
#[derive(Debug)]
pub enum Failure {
    /// Apple refused outright — a bad request, failed authentication, a retired endpoint. Asking
    /// again soon will be refused again.
    Refused(String),
    /// Apple asked to be left alone for a while.
    RateLimited(Duration),
    /// The session needs a fresh sign-in.
    Reauth,
    /// The network, a dropped socket, a timeout: worth trying again shortly.
    Transient(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(why) => write!(f, "refused: {why}"),
            Self::RateLimited(d) => write!(f, "rate limited for {}s", d.as_secs()),
            Self::Reauth => write!(f, "needs a fresh sign-in"),
            Self::Transient(why) => write!(f, "{why}"),
        }
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        match e {
            Error::ReauthRequired => Self::Reauth,
            Error::Refused(why) => Self::Refused(why),
            other => Self::Transient(other.to_string()),
        }
    }
}

/// A live connection to the push server, subscribed to some CloudKit containers.
pub struct Courier {
    ws: Ws,
    topics: Vec<String>,
    /// SHA-1 of each topic, as a push may name its topic that way.
    hashed: Vec<([u8; 20], String)>,
}

fn sha1(s: &str) -> [u8; 20] {
    use sha1::Digest;
    sha1::Sha1::digest(s.as_bytes()).into()
}

/// The connection-response status codes (`WebConnectionResponseMessage.Status`).
const OK: u64 = 0;
const INVALID_NONCE: u64 = 2;
const RATE_LIMITED: u64 = 4;
const EXPIRED_PUSH_TOKEN: u64 = 6;

/// How long the server is given to answer the connect, and a JWT request.
const ANSWER: Duration = Duration::from_secs(15);

impl Courier {
    /// Connect to `server` (normally [`SERVER`]), get or keep a token, and filter the socket
    /// to the containers' topics. Not yet registered: see [`Courier::register`].
    pub async fn connect(
        server: &str,
        identity: &mut Identity,
        containers: &[&str],
    ) -> std::result::Result<Self, Failure> {
        // Only Apple's push server, ever: the socket does not ride the HTTP client's locks.
        let host = server
            .split("://")
            .last()
            .unwrap_or(server)
            .split(['/', ':'])
            .next()
            .unwrap_or_default();
        if !(host.ends_with(".apple.com") || host == "localhost" || host == "127.0.0.1") {
            return Err(Failure::Refused(format!("{host} is not Apple's")));
        }
        let scheme = if server.starts_with("ws://") {
            ""
        } else {
            "wss://"
        };
        let topics: Vec<String> = containers.iter().map(|c| topic_of(c)).collect();
        let mut clock: Option<u64> = None;
        for _ in 0..3 {
            let url = format!(
                "{scheme}{server}/v2/{}",
                identity.connect_message(clock.unwrap_or_else(now_ms))
            );
            let mut request = url
                .into_client_request()
                .map_err(|e| Failure::Transient(format!("bad push address: {e}")))?;
            request.headers_mut().insert(
                "origin",
                tungstenite::http::HeaderValue::from_static("https://www.icloud.com"),
            );
            let (mut ws, _) =
                tokio::time::timeout(ANSWER, tokio_tungstenite::connect_async(request))
                    .await
                    .map_err(|_| Failure::Transient("the push server did not answer".into()))?
                    .map_err(|e| match e {
                        tungstenite::Error::Http(r) => {
                            let s = r.status().as_u16();
                            if matches!(s, 400 | 401 | 403 | 404 | 410) {
                                Failure::Refused(format!("the push server answered {s}"))
                            } else {
                                Failure::Transient(format!("the push server answered {s}"))
                            }
                        }
                        other => Failure::Transient(format!("push socket: {}", words(&other))),
                    })?;
            let first = tokio::time::timeout(ANSWER, next_binary(&mut ws))
                .await
                .map_err(|_| Failure::Transient("no answer to the connect".into()))??;
            let outer = pb::fields(&first)
                .ok_or_else(|| Failure::Transient("an unreadable connect answer".into()))?;
            let Some(response) = pb::bytes(&outer, 1).and_then(pb::fields) else {
                return Err(Failure::Transient(
                    "the connect was answered with something else".into(),
                ));
            };
            let status = pb::int(&response, 2).unwrap_or(OK);
            match status {
                OK => {
                    let token = pb::bytes(&response, 1)
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .and_then(|t| {
                            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, t)
                                .ok()
                        })
                        .filter(|t| !t.is_empty())
                        .ok_or_else(|| Failure::Transient("connected without a token".into()))?;
                    identity.token = Some(token);
                    let mut courier = Self {
                        ws,
                        hashed: topics.iter().map(|t| (sha1(t), t.clone())).collect(),
                        topics,
                    };
                    let mut filter = pb::W::default();
                    for t in &courier.topics {
                        filter.string(1, t);
                    }
                    courier.send(3, &filter).await?;
                    return Ok(courier);
                }
                INVALID_NONCE => {
                    // Our clock is off from Apple's: sign against theirs, as the shell does.
                    clock = pb::int(&response, 3).map(|s| s.saturating_mul(1000));
                    if clock.is_none() {
                        return Err(Failure::Transient("nonce refused without a clock".into()));
                    }
                }
                EXPIRED_PUSH_TOKEN => identity.forget_token(),
                RATE_LIMITED => return Err(Failure::RateLimited(Duration::from_secs(15 * 60))),
                other => {
                    // A kept token that is refused is dropped; a fresh connection is refused
                    // for a reason asking again won't change.
                    if identity.token.take().is_none() {
                        return Err(Failure::Refused(format!("connect status {other}")));
                    }
                }
            }
        }
        Err(Failure::Transient(
            "the push server would not take the connect".into(),
        ))
    }

    async fn send(&mut self, field: u32, inner: &pb::W) -> std::result::Result<(), Failure> {
        let mut outer = pb::W::default();
        outer.message(field, inner);
        self.ws
            .send(Message::binary(outer.0))
            .await
            .map_err(|e| Failure::Transient(format!("push socket: {}", words(&e))))
    }

    /// Ask the socket for the short-lived JWT the registry wants.
    async fn jwt(&mut self) -> std::result::Result<String, Failure> {
        let mut ask = pb::W::default();
        ask.uint(1, 600);
        self.send(7, &ask).await?;
        let deadline = tokio::time::Instant::now() + ANSWER;
        loop {
            let frame = tokio::time::timeout_at(deadline, next_binary(&mut self.ws))
                .await
                .map_err(|_| Failure::Transient("no JWT came back".into()))??;
            let Some(outer) = pb::fields(&frame) else {
                continue;
            };
            if let Some(answer) = pb::bytes(&outer, 8).and_then(pb::fields) {
                return match pb::int(&answer, 2) {
                    Some(1) => pb::bytes(&answer, 1)
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .map(str::to_string)
                        .filter(|j| !j.is_empty())
                        .ok_or_else(|| Failure::Transient("an empty JWT".into())),
                    Some(3) => Err(Failure::RateLimited(Duration::from_secs(10 * 60))),
                    other => Err(Failure::Transient(format!("JWT status {other:?}"))),
                };
            }
            // A push that raced the answer is acknowledged and otherwise left to the next
            // full pass, which runs once the channel is up.
            self.acknowledge(&outer).await?;
        }
    }

    /// Register this connection's token for its topics with the push registry, using the
    /// session's cookies. Must be done after each new token and before a registration
    /// lapses ([`REGISTRATION_SECONDS`]).
    pub async fn register(
        &mut self,
        http: &reqwest::Client,
        registry: &str,
        identity: &Identity,
        dsid: &str,
    ) -> std::result::Result<(), Failure> {
        let jwt = self.jwt().await?;
        let token = identity
            .token
            .as_deref()
            .ok_or_else(|| Failure::Transient("no token to register".into()))?;
        let body = registration(identity, token, dsid, &jwt, &self.topics, now_ms());
        let response = http
            .post(registry)
            .header("x-protocol-version", "1")
            .header("x-pds-client-version", "2")
            .header("x-pds-client-type", "iCloudCom")
            .header("x-registration-reason", "1")
            .header("content-type", "application/protobuf")
            .body(body)
            .send()
            .await
            .map_err(|e| Failure::from(Error::from(e)))?;
        let status = response.status();
        if matches!(status.as_u16(), 401 | 421 | 450) {
            return Err(Failure::Reauth);
        }
        let bytes = response.bytes().await.unwrap_or_default();
        if !status.is_success() {
            return Err(if matches!(status.as_u16(), 400 | 403 | 404 | 410) {
                Failure::Refused(format!("the push registry answered {status}"))
            } else {
                Failure::Transient(format!("the push registry answered {status}"))
            });
        }
        let fields = pb::fields(&bytes)
            .ok_or_else(|| Failure::Transient("an unreadable registry answer".into()))?;
        match pb::int(&fields, 1).unwrap_or(0) {
            0 => Ok(()),
            // RETRY_WITH_DELAY and friends: try again after what it says, or a while.
            code @ (6002 | 6003 | 6007) => {
                let wait = pb::int(&fields, 5).filter(|s| *s > 0).unwrap_or(60);
                quiet!("push: registry status {code}, asked to wait {wait}s");
                Err(Failure::RateLimited(Duration::from_secs(wait.min(3600))))
            }
            6005 | 6008 | 6010 => Err(Failure::Reauth),
            code => Err(Failure::Refused(format!("registry status {code}"))),
        }
    }

    /// Acknowledge a push in `outer` if it is one, returning what it was about.
    async fn acknowledge(
        &mut self,
        outer: &[(u32, pb::V<'_>)],
    ) -> std::result::Result<Option<Event>, Failure> {
        let Some(push) = pb::bytes(outer, 2).and_then(pb::fields) else {
            return Ok(None);
        };
        let topic = pb::bytes(&push, 1).unwrap_or_default();
        if let Some(id) = pb::int(&push, 2) {
            let mut ack = pb::W::default();
            ack.bytes(1, topic).uint(2, id);
            self.send(2, &ack).await?;
        }
        let named = self.topic_name(topic);
        let payload = pb::bytes(&push, 4)
            .and_then(|p| serde_json::from_slice::<Value>(p).ok())
            .unwrap_or(Value::Null);
        let container = crate::push::container_of(&payload)
            .or_else(|| named.and_then(|t| container_of_topic(&t).map(str::to_string)));
        Ok(Some(container.map_or(Event::Other, Event::Changed)))
    }

    fn topic_name(&self, topic: &[u8]) -> Option<String> {
        if let Some((_, t)) = self.hashed.iter().find(|(h, _)| h.as_slice() == topic) {
            return Some(t.clone());
        }
        std::str::from_utf8(topic).ok().map(str::to_string)
    }

    /// Wait up to `longest` for the next push. `Ok(None)` is a quiet wait. The socket is pinged
    /// while quiet, so a connection that died without closing is found within `longest`.
    pub async fn next(&mut self, longest: Duration) -> std::result::Result<Option<Event>, Failure> {
        let deadline = tokio::time::Instant::now() + longest;
        let mut pinged: Option<tokio::time::Instant> = None;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            // A ping every minute of quiet; no answer at all within 30 s is a dead socket.
            let wake = match pinged {
                Some(at) => (at + Duration::from_secs(30)).min(deadline),
                None => (now + Duration::from_secs(60)).min(deadline),
            };
            match tokio::time::timeout_at(wake, self.ws.next()).await {
                Err(_) => match pinged {
                    Some(_) if tokio::time::Instant::now() < deadline => {
                        return Err(Failure::Transient(
                            "the push socket stopped answering".into(),
                        ))
                    }
                    Some(_) => return Ok(None),
                    None => {
                        self.ws
                            .send(Message::Ping(Vec::new().into()))
                            .await
                            .map_err(|e| {
                                Failure::Transient(format!("push socket: {}", words(&e)))
                            })?;
                        pinged = Some(tokio::time::Instant::now());
                    }
                },
                Ok(None) => return Err(Failure::Transient("the push socket closed".into())),
                Ok(Some(Err(e))) => {
                    return Err(Failure::Transient(format!("push socket: {}", words(&e))))
                }
                Ok(Some(Ok(frame))) => {
                    pinged = None;
                    match frame {
                        Message::Binary(b) => {
                            let Some(outer) = pb::fields(&b) else {
                                continue;
                            };
                            if let Some(event) = self.acknowledge(&outer).await? {
                                return Ok(Some(event));
                            }
                        }
                        Message::Close(c) => {
                            return Err(Failure::Transient(format!(
                                "the push server closed the socket ({})",
                                c.map_or(0, |c| u16::from(c.code))
                            )))
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Close politely.
    pub async fn close(mut self) {
        self.ws.close(None).await.ok();
    }
}

/// The next binary frame, answering pings on the way (tungstenite queues the pong).
async fn next_binary(ws: &mut Ws) -> std::result::Result<Vec<u8>, Failure> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(b))) => return Ok(b.to_vec()),
            Some(Ok(Message::Close(c))) => {
                return Err(Failure::Transient(format!(
                    "the push server closed the socket ({})",
                    c.map_or(0, |c| u16::from(c.code))
                )))
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(Failure::Transient(format!("push socket: {}", words(&e)))),
            None => return Err(Failure::Transient("the push socket closed".into())),
        }
    }
}

/// A socket error in words, with no address in it (the address carries a signed nonce).
fn words(e: &tungstenite::Error) -> String {
    match e {
        tungstenite::Error::Io(io) => format!("{}", io.kind()),
        tungstenite::Error::Tls(_) => "a TLS failure".into(),
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => "closed".into(),
        tungstenite::Error::Protocol(p) => format!("{p}"),
        tungstenite::Error::Http(r) => format!("HTTP {}", r.status()),
        tungstenite::Error::Url(_) => "a bad address".into(),
        _ => "a socket error".into(),
    }
}

/// The registry's `BatchRegisterReq` for one token and its topics.
fn registration(
    identity: &Identity,
    token: &[u8],
    dsid: &str,
    jwt: &str,
    topics: &[String],
    unix_ms: u64,
) -> Vec<u8> {
    let signed = identity.sign(unix_ms, Some(2));
    let mut web = pb::W::default();
    web.bytes(1, &signed.nonce)
        .bytes(2, &signed.public_key)
        .bytes(3, &signed.signature)
        .bytes(4, jwt.as_bytes());
    let mut token_auth = pb::W::default();
    token_auth.message(1, &web);
    let mut user_auth = pb::W::default();
    user_auth.message(2, &pb::W::default());
    let mut user_token = pb::W::default();
    user_token
        .string(1, dsid)
        .message(2, &user_auth)
        .string(3, &hex(token).to_uppercase())
        .message(4, &token_auth);
    let mut request = pb::W::default();
    request.message(1, &user_token);
    for t in topics {
        let qualifier = container_of_topic(t)
            .map(|c| format!("{c}_P"))
            .unwrap_or_default();
        let mut topic = pb::W::default();
        topic.string(1, t).string(2, &qualifier).uint(4, 5); // client: ICLOUDCOM
        request.message(4, &topic);
    }
    let mut batch = pb::W::default();
    batch.message(1, &request).uint(2, REGISTRATION_SECONDS);
    batch.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, VerifyingKey};

    #[test]
    fn a_container_is_its_topic_and_back() {
        assert_eq!(
            topic_of("com.apple.notes"),
            "com.apple.icloud-container.com.apple.notes"
        );
        assert_eq!(
            container_of_topic("com.apple.icloud-container.com.apple.reminders"),
            Some("com.apple.reminders")
        );
        assert_eq!(container_of_topic("com.apple.mail"), None);
    }

    #[test]
    fn the_nonce_is_laid_out_as_the_shell_lays_it() {
        let n = nonce(0x0102_0304_0506_0708, None);
        assert_eq!(n.len(), 17);
        assert_eq!(&n[..9], &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let n = nonce(0x0102_0304_0506_0708, Some(2));
        assert_eq!(n.len(), 18);
        assert_eq!(&n[..10], &[1, 1, 2, 3, 4, 5, 6, 7, 8, 2]);
    }

    #[test]
    fn the_signature_verifies_once_unwrapped() {
        let id = Identity::new();
        let s = id.sign(1_700_000_000_000, None);
        assert_eq!(s.public_key.len(), 65);
        assert_eq!(s.public_key[0], 4, "uncompressed point");
        assert_eq!(&s.signature[..3], &[1, 3, 48]);
        // Strip the shell's framing back to r and s, and check them against the key.
        let der = &s.signature[2..];
        let mut at = 2;
        let mut part = || {
            assert_eq!(der[at], 2);
            let len = der[at + 1] as usize;
            let v = &der[at + 2..at + 2 + len];
            at += 2 + len;
            v[v.len() - 32..].to_vec()
        };
        let (r, sv) = (part(), part());
        let sig = Signature::from_scalars(
            *p256::FieldBytes::from_slice(&r),
            *p256::FieldBytes::from_slice(&sv),
        )
        .unwrap();
        let key = VerifyingKey::from_sec1_bytes(&s.public_key).unwrap();
        key.verify(&s.nonce, &sig).unwrap();
    }

    #[test]
    fn the_connect_message_asks_for_a_new_token_then_keeps_it() {
        let mut id = Identity::new();
        let fresh = id.connect_message(1);
        let bytes: Vec<u8> = (0..fresh.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&fresh[i..i + 2], 16).unwrap())
            .collect();
        let outer = pb::fields(&bytes).unwrap();
        let v1 = pb::fields(pb::bytes(&outer, 1).unwrap()).unwrap();
        assert_eq!(pb::bytes(&v1, 1).unwrap().len(), 65);
        assert_eq!(pb::bytes(&v1, 2).unwrap().len(), 17);
        let new = pb::fields(pb::bytes(&v1, 5).unwrap()).unwrap();
        assert_eq!(pb::int(&new, 1), Some(86_400));
        assert!(pb::bytes(&v1, 4).is_none());

        id.token = Some(vec![0xAB, 0xCD]);
        let kept = id.connect_message(1);
        let bytes: Vec<u8> = (0..kept.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&kept[i..i + 2], 16).unwrap())
            .collect();
        let outer = pb::fields(&bytes).unwrap();
        let v1 = pb::fields(pb::bytes(&outer, 1).unwrap()).unwrap();
        let with = pb::fields(pb::bytes(&v1, 4).unwrap()).unwrap();
        assert_eq!(pb::bytes(&with, 1), Some(&b"q80="[..]));
        assert!(pb::bytes(&v1, 5).is_none());
    }

    #[test]
    fn the_registration_names_each_topic_for_icloud_com() {
        let id = Identity::new();
        let body = registration(
            &id,
            &[0xAB, 0x01],
            "123",
            "J.W.T",
            &[topic_of("com.apple.notes")],
            5,
        );
        let batch = pb::fields(&body).unwrap();
        assert_eq!(pb::int(&batch, 2), Some(43_200));
        let request = pb::fields(pb::bytes(&batch, 1).unwrap()).unwrap();
        let user = pb::fields(pb::bytes(&request, 1).unwrap()).unwrap();
        assert_eq!(pb::bytes(&user, 1), Some(&b"123"[..]));
        assert_eq!(pb::bytes(&user, 3), Some(&b"AB01"[..]), "upper-case hex");
        let auth = pb::fields(pb::bytes(&user, 2).unwrap()).unwrap();
        assert_eq!(pb::bytes(&auth, 2), Some(&b""[..]), "icloudAuth, empty");
        let web =
            pb::fields(pb::bytes(&pb::fields(pb::bytes(&user, 4).unwrap()).unwrap(), 1).unwrap())
                .unwrap();
        assert_eq!(pb::bytes(&web, 1).unwrap()[9], 2, "a registry nonce");
        assert_eq!(pb::bytes(&web, 4), Some(&b"J.W.T"[..]));
        let topic = pb::fields(pb::bytes(&request, 4).unwrap()).unwrap();
        assert_eq!(
            pb::bytes(&topic, 1),
            Some(&b"com.apple.icloud-container.com.apple.notes"[..])
        );
        assert_eq!(pb::bytes(&topic, 2), Some(&b"com.apple.notes_P"[..]));
        assert_eq!(pb::int(&topic, 4), Some(5));
    }

    /// The whole exchange against a stand-in push server and registry: the connect, the
    /// token, the filter, the JWT, the registration, then a push named by its topic's SHA-1,
    /// which comes out as its container and is acknowledged.
    #[tokio::test]
    #[allow(clippy::result_large_err)] // tungstenite's own callback signature
    async fn a_push_arrives_as_its_container_and_is_acknowledged() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut path = String::new();
            let mut ws =
                tokio_tungstenite::accept_hdr_async(tcp, |req: &Request, res: Response| {
                    path = req.uri().path().to_string();
                    Ok(res)
                })
                .await
                .unwrap();
            let hexed = path.strip_prefix("/v2/").expect("the v2 connect path");
            let bytes: Vec<u8> = (0..hexed.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hexed[i..i + 2], 16).unwrap())
                .collect();
            let outer = pb::fields(&bytes).unwrap();
            let v1 = pb::fields(pb::bytes(&outer, 1).unwrap()).unwrap();
            assert!(pb::bytes(&v1, 5).is_some(), "a new connection is asked for");
            let mut ok = pb::W::default();
            ok.string(1, "VE9L").uint(2, 0);
            let mut reply = pb::W::default();
            reply.message(1, &ok);
            ws.send(Message::binary(reply.0)).await.unwrap();
            let mut got_filter = false;
            loop {
                let Some(Ok(Message::Binary(b))) = ws.next().await else {
                    panic!("the client went away");
                };
                let m = pb::fields(&b).unwrap();
                if let Some(filter) = pb::bytes(&m, 3) {
                    let f = pb::fields(filter).unwrap();
                    assert_eq!(f.len(), 2, "both topics");
                    got_filter = true;
                } else if pb::bytes(&m, 7).is_some() {
                    assert!(got_filter, "filtered before anything else");
                    let mut jwt = pb::W::default();
                    jwt.string(1, "J.W.T").uint(2, 1);
                    let mut reply = pb::W::default();
                    reply.message(8, &jwt);
                    ws.send(Message::binary(reply.0)).await.unwrap();
                    break;
                }
            }
            // A push, named by the SHA-1 of its topic, once the client has registered.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let topic = sha1(&topic_of("com.apple.reminders"));
            let mut push = pb::W::default();
            push.bytes(1, &topic).uint(2, 7).bytes(
                4,
                br#"{"aps":{"content-available":1},"ck":{"cid":"com.apple.reminders"}}"#,
            );
            let mut reply = pb::W::default();
            reply.message(2, &push);
            ws.send(Message::binary(reply.0)).await.unwrap();
            let Some(Ok(Message::Binary(b))) = ws.next().await else {
                panic!("no acknowledgement");
            };
            let m = pb::fields(&b).unwrap();
            let ack = pb::fields(pb::bytes(&m, 2).expect("an ack")).unwrap();
            assert_eq!(pb::int(&ack, 2), Some(7));
            assert_eq!(pb::bytes(&ack, 1), Some(&topic[..]));
        });

        let registry = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::header("x-pds-client-type", "iCloudCom"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(vec![0x08, 0x00]))
            .expect(1)
            .mount(&registry)
            .await;

        let mut id = Identity::new();
        let mut courier = Courier::connect(
            &format!("ws://{addr}"),
            &mut id,
            &["com.apple.notes", "com.apple.reminders"],
        )
        .await
        .unwrap();
        assert!(id.has_token());
        courier
            .register(&reqwest::Client::new(), &registry.uri(), &id, "1")
            .await
            .unwrap();
        let event = courier.next(Duration::from_secs(5)).await.unwrap();
        assert_eq!(event, Some(Event::Changed("com.apple.reminders".into())));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn only_apple_is_ever_dialled() {
        let mut id = Identity::new();
        let got = Courier::connect("example.com", &mut id, &["com.apple.notes"]).await;
        assert!(matches!(got, Err(Failure::Refused(_))));
    }

    #[test]
    fn a_malformed_message_is_refused_not_panicked_on() {
        assert!(pb::fields(&[0x0a, 0x05, 1]).is_none());
        assert!(pb::fields(&[0xff]).is_none());
        assert_eq!(pb::fields(&[]).unwrap().len(), 0);
    }
}
