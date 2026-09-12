//! The sign-in flow against a mock Apple, so regressions surface in CI rather than on a
//! real account. Two shipped bugs motivated this file:
//!
//!   * the idmsa Origin was sent to endpoints that expect icloud.com, earning a 409;
//!   * an abandoned two-factor attempt left `scnt` behind, and replaying it into the next
//!     `/signin/init` made Apple reject a sign-in that should have succeeded.
//!
//! Both are asserted here. The SRP maths itself is covered by vectors in `srp.rs`; this file
//! is about the HTTP conversation.

use quince_auth::{Client, Endpoints, SignInOutcome, Store};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Distinct from the mock's own URL so an Origin assertion means something.
const SRP_ORIGIN: &str = "https://idmsa.test.invalid";
const WEB_ORIGIN: &str = "https://web.test.invalid";

fn endpoints_for(server: &MockServer) -> Endpoints {
    let base = server.uri();
    Endpoints {
        // Origins are header values only; pointing them elsewhere lets us prove which
        // origin each endpoint received.
        auth_root: SRP_ORIGIN.into(),
        home: WEB_ORIGIN.into(),
        auth: format!("{base}/appleauth/auth"),
        setup: format!("{base}/setup/ws/1"),
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as E, Engine};
    E.encode(bytes)
}

/// `/signin/init` answering with a well-formed SRP challenge.
async fn mount_init(server: &MockServer, status: u16) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("scnt", "scnt-from-init")
                .insert_header("X-Apple-ID-Session-Id", "session-from-init")
                .set_body_json(json!({
                    "salt": b64(&[0x77; 16]),
                    "b": b64(&[0x5e; 256]),
                    "c": "challenge-c",
                    "iteration": 20173,
                    "protocol": "s2k",
                })),
        )
        .mount(server)
        .await;
}

async fn mount_complete(server: &MockServer, status: u16) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("X-Apple-Session-Token", "session-token-abc")
                .insert_header("X-Apple-ID-Account-Country", "IND")
                .set_body_json(json!({})),
        )
        .mount(server)
        .await;
}

async fn mount_trust_and_login(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/appleauth/auth/2sv/trust"))
        .respond_with(
            ResponseTemplate::new(204)
                .insert_header("X-Apple-TwoSV-Trust-Token", "trust-token-xyz")
                .insert_header("X-Apple-Session-Token", "session-token-def"),
        )
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890" },
            "webservices": {
                "calendar": { "url": "https://p00-calendarws.test.invalid:443", "status": "active" },
                "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
            }
        })))
        .mount(server)
        .await;
}

fn client_for(server: &MockServer, dir: &std::path::Path) -> Client {
    Client::with(dir, endpoints_for(server), Store::file(dir).unwrap()).unwrap()
}

fn requests_to<'a>(reqs: &'a [Request], suffix: &str) -> Vec<&'a Request> {
    reqs.iter().filter(|r| r.url.path().ends_with(suffix)).collect()
}

#[tokio::test]
async fn two_factor_sign_in_stores_the_trust_token() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await; // 409 => two-factor required
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());

    let outcome = client.sign_in("someone@example.invalid", "hunter2").await.unwrap();
    assert_eq!(outcome, SignInOutcome::NeedsTwoFactor);

    client.submit_two_factor("123456").await.unwrap();

    let session = client.session();
    assert_eq!(session.trust_token.as_deref(), Some("trust-token-xyz"));
    assert_eq!(session.dsid.as_deref(), Some("1234567890"));
    assert_eq!(
        session.service_url("calendar").as_deref(),
        Some("https://p00-calendarws.test.invalid:443"),
        "the service map should be usable without a trailing slash"
    );

    // A fresh client over the same store must resume without a password.
    let reloaded = client_for(&server, dir.path());
    assert!(reloaded.has_stored_session(), "the trust token must outlive the process");
}

/// The regression: an abandoned attempt left `scnt` behind, and replaying it into a new
/// `/signin/init` made Apple answer 409.
#[tokio::test]
async fn sign_in_never_replays_a_previous_challenge() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());

    // First attempt: abandoned after Apple asks for a code, leaving scnt/session id stored.
    client.sign_in("someone@example.invalid", "hunter2").await.unwrap();
    assert_eq!(client.session().scnt.as_deref(), Some("scnt-from-init"));

    // Second attempt from the same session must start clean.
    client.sign_in("someone@example.invalid", "hunter2").await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let inits = requests_to(&reqs, "/signin/init");
    assert_eq!(inits.len(), 2, "expected two sign-in attempts");
    for (i, req) in inits.iter().enumerate() {
        assert!(
            req.headers.get("scnt").is_none(),
            "/signin/init #{i} leaked scnt from an earlier challenge"
        );
        assert!(
            req.headers.get("x-apple-id-session-id").is_none(),
            "/signin/init #{i} leaked a session id from an earlier challenge"
        );
    }
}

/// The other regression: only the two SRP calls may carry the idmsa Origin.
#[tokio::test]
async fn idmsa_origin_is_used_only_for_sign_in() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client.sign_in("someone@example.invalid", "hunter2").await.unwrap();
    client.submit_two_factor("123456").await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let origin_of = |r: &Request| {
        r.headers.get("origin").and_then(|v| v.to_str().ok()).unwrap_or("<none>").to_string()
    };

    for suffix in ["/signin/init", "/signin/complete"] {
        let found = requests_to(&reqs, suffix);
        assert!(!found.is_empty(), "no request reached {suffix}");
        for r in found {
            assert_eq!(origin_of(r), SRP_ORIGIN, "{suffix} must use the idmsa origin");
        }
    }

    for suffix in ["/verify/trusteddevice/securitycode", "/2sv/trust", "/accountLogin"] {
        let found = requests_to(&reqs, suffix);
        assert!(!found.is_empty(), "no request reached {suffix}");
        for r in found {
            assert_eq!(
                origin_of(r),
                WEB_ORIGIN,
                "{suffix} must use the icloud.com origin — the idmsa one earns a 409"
            );
        }
    }
}

#[tokio::test]
async fn a_wrong_code_is_reported_as_a_wrong_code() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "serviceErrors": [{ "code": "-21669", "message": "Incorrect verification code." }]
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client.sign_in("someone@example.invalid", "hunter2").await.unwrap();

    let err = client.submit_two_factor("000000").await.unwrap_err();
    assert!(
        matches!(err, quince_auth::Error::BadCode),
        "a wrong code must not surface as a protocol failure: {err}"
    );
}

/// Apple answers 409 on this endpoint in flows where the code was in fact accepted. The
/// status alone is not evidence, so completion is what decides.
#[tokio::test]
async fn a_409_that_still_completes_is_treated_as_success() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "serviceErrors": [{ "code": "-20209", "message": "Unexpected." }]
        })))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client.sign_in("someone@example.invalid", "hunter2").await.unwrap();

    client
        .submit_two_factor("123456")
        .await
        .expect("the session became usable, so the code was accepted");
    assert_eq!(client.session().trust_token.as_deref(), Some("trust-token-xyz"));
}

#[tokio::test]
async fn throttling_is_named_so_the_fix_is_to_wait() {
    let server = MockServer::start().await;
    mount_init(&server, 429).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());

    let err = client.sign_in("someone@example.invalid", "hunter2").await.unwrap_err();
    assert!(
        matches!(err, quince_auth::Error::RateLimited),
        "throttling must be distinguishable from a real failure: {err}"
    );
}

/// A dead stored session must not block a fresh sign-in.
#[tokio::test]
async fn a_rejected_session_falls_back_cleanly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421))
        .mount(&server)
        .await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;

    let dir = tempfile::tempdir().unwrap();
    {
        // Seed a stored session that Apple will reject.
        let mut seed = client_for(&server, dir.path());
        seed.sign_in("someone@example.invalid", "hunter2").await.unwrap();
    }

    let mut client = client_for(&server, dir.path());
    assert!(client.has_stored_session());
    assert!(!client.resume().await.unwrap(), "a 421 means the session is dead");
    assert!(client.session().scnt.is_none(), "a dead session must not keep its challenge state");

    // And signing in again still works.
    let outcome = client.sign_in("someone@example.invalid", "hunter2").await.unwrap();
    assert_eq!(outcome, SignInOutcome::NeedsTwoFactor);
}
