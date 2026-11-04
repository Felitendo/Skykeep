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
    reqs.iter()
        .filter(|r| r.url.path().ends_with(suffix))
        .collect()
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

    let outcome = client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
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
    assert!(
        reloaded.has_stored_session(),
        "the trust token must outlive the process"
    );
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
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    assert_eq!(client.session().scnt.as_deref(), Some("scnt-from-init"));

    // Second attempt from the same session must start clean.
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();

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
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_two_factor("123456").await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let origin_of = |r: &Request| {
        r.headers
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<none>")
            .to_string()
    };

    for suffix in ["/signin/init", "/signin/complete"] {
        let found = requests_to(&reqs, suffix);
        assert!(!found.is_empty(), "no request reached {suffix}");
        for r in found {
            assert_eq!(
                origin_of(r),
                SRP_ORIGIN,
                "{suffix} must use the idmsa origin"
            );
        }
    }

    for suffix in [
        "/verify/trusteddevice/securitycode",
        "/2sv/trust",
        "/accountLogin",
    ] {
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
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();

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
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();

    client
        .submit_two_factor("123456")
        .await
        .expect("the session became usable, so the code was accepted");
    assert_eq!(
        client.session().trust_token.as_deref(),
        Some("trust-token-xyz")
    );
}

#[tokio::test]
async fn throttling_is_named_so_the_fix_is_to_wait() {
    let server = MockServer::start().await;
    mount_init(&server, 429).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());

    let err = client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap_err();
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
        seed.sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap();
    }

    let mut client = client_for(&server, dir.path());
    assert!(client.has_stored_session());
    assert!(
        !client.resume().await.unwrap(),
        "a 421 means the session is dead"
    );
    assert!(
        client.session().scnt.is_none(),
        "a dead session must not keep its challenge state"
    );

    // And signing in again still works.
    let outcome = client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    assert_eq!(outcome, SignInOutcome::NeedsTwoFactor);
}

/// A cooldown that resets on restart is no cooldown. After a failure the next attempt must
/// be refused locally, without another request reaching Apple.
#[tokio::test]
async fn a_cooldown_survives_a_restart_and_spares_apple() {
    let server = MockServer::start().await;
    mount_init(&server, 429).await; // Apple says slow down

    let dir = tempfile::tempdir().unwrap();
    {
        let mut client = client_for(&server, dir.path());
        let err = client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap_err();
        assert!(matches!(err, quince_auth::Error::RateLimited));
    }

    let before = server.received_requests().await.unwrap().len();

    // A brand-new client, as if the app had been restarted.
    let mut restarted = client_for(&server, dir.path());
    let err = restarted
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap_err();
    match err {
        quince_auth::Error::CoolingDown { seconds } => assert!(seconds > 0),
        other => panic!("expected a cooldown after a restart, got {other}"),
    }

    let after = server.received_requests().await.unwrap().len();
    assert_eq!(
        before, after,
        "the cooldown must be enforced without contacting Apple"
    );
}

/// Reaching the two-factor step means the password was accepted, so it must clear any
/// backoff rather than count as another failure.
#[tokio::test]
async fn a_successful_sign_in_clears_the_backoff() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();

    assert_eq!(client.session().consecutive_failures, 0);
    assert!(client.session().retry_after.is_none());
}

/// Signing out must leave nothing behind — a later client should find no session at all.
#[tokio::test]
async fn signing_out_forgets_everything() {
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
    {
        let mut client = client_for(&server, dir.path());
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap();
        client.submit_two_factor("123456").await.unwrap();
        assert!(client.session().trust_token.is_some());

        client.sign_out().unwrap();
        assert!(client.session().trust_token.is_none());
        assert!(!client.has_stored_session());
    }

    // And nothing survives into a fresh client over the same directory.
    let reopened = client_for(&server, dir.path());
    assert!(
        !reopened.has_stored_session(),
        "sign-out must clear the stored session"
    );
    assert!(reopened.session().trust_token.is_none());
    assert!(reopened.session().webservices.is_none());
}

/// Signing out must also clear a cooldown; otherwise a user who hit a backoff could be
/// locked out of their own app with no way to reset it.
#[tokio::test]
async fn signing_out_clears_a_cooldown() {
    let server = MockServer::start().await;
    mount_init(&server, 429).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap_err();
    assert!(client.session().retry_after.is_some());

    client.sign_out().unwrap();
    assert!(
        client.session().retry_after.is_none(),
        "a fresh start must not inherit the previous account's backoff"
    );
}

/// Apple's iteration count drives PBKDF2, arrives over the network, and is therefore
/// untrusted input controlling how much work we do. An implausible value must be refused
/// rather than truncated into a cast or left to burn CPU.
#[tokio::test]
async fn an_implausible_work_factor_is_refused() {
    for absurd in [u64::from(u32::MAX) + 1, 5_000_000_000, 3] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/appleauth/auth/signin/init"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "salt": b64(&[0x77; 16]),
                "b": b64(&[0x5e; 256]),
                "c": "challenge-c",
                "iteration": absurd,
                "protocol": "s2k",
            })))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let mut client = client_for(&server, dir.path());
        let err = client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap_err();
        assert!(
            matches!(err, quince_auth::Error::Protocol(_)),
            "iteration={absurd} should be rejected, got {err}"
        );
    }
}

/// Apple's service endpoints authenticate by cookie. If the cookies are lost but the tokens
/// survive — a copied install, a cleared cache, cookies expiring before the trust token —
/// resuming must re-establish them rather than leaving the app looking signed in while every
/// service answers 421.
#[tokio::test]
async fn resume_rebuilds_a_session_from_tokens_alone() {
    let server = MockServer::start().await;
    // No valid cookies, so Apple rejects the cheap check.
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421).set_body_json(json!({
            "reason": "Missing X-APPLE-WEBAUTH-USER cookie", "error": 1
        })))
        .mount(&server)
        .await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;

    let dir = tempfile::tempdir().unwrap();
    {
        let mut first = client_for(&server, dir.path());
        first
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap();
        first.submit_two_factor("123456").await.unwrap();
    }
    // Throw the cookies away, keeping the tokens.
    let cookies = dir.path().join("cookies.json");
    if cookies.exists() {
        std::fs::remove_file(&cookies).unwrap();
    }

    let mut resumed = client_for(&server, dir.path());
    assert!(resumed.has_stored_session());
    assert!(
        resumed.resume().await.unwrap(),
        "tokens alone must be enough to re-establish a session"
    );
    assert!(
        resumed.session().webservices.is_some(),
        "and the service map must come back with it"
    );
}

/// Offline is not signed out. With the network down — the app opened before the network
/// came up, or a resume from sleep mid-reconnect — resuming says so and leaves the session
/// exactly as it was; the next launch, online, is still signed in, with no cooldown.
#[tokio::test]
async fn resuming_offline_keeps_the_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421).set_body_json(json!({ "error": 1 })))
        .mount(&server)
        .await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    let dir = tempfile::tempdir().unwrap();
    {
        let mut first = client_for(&server, dir.path());
        first
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap();
        first.submit_two_factor("123456").await.unwrap();
    }
    // Nothing listening: every request fails to connect.
    let nowhere = Endpoints {
        auth_root: SRP_ORIGIN.into(),
        home: WEB_ORIGIN.into(),
        auth: "http://127.0.0.1:9/appleauth/auth".into(),
        setup: "http://127.0.0.1:9/setup/ws/1".into(),
    };
    {
        let mut offline =
            Client::with(dir.path(), nowhere, Store::file(dir.path()).unwrap()).unwrap();
        assert!(offline.has_stored_session());
        let err = offline
            .resume()
            .await
            .expect_err("offline is said, not hidden");
        assert!(matches!(err, quince_auth::Error::Network(_)), "{err}");
    }
    let back = client_for(&server, dir.path());
    assert!(
        back.has_stored_session(),
        "still signed in after a launch with no network"
    );
    assert!(
        back.session().session_token.is_some(),
        "the session token was kept"
    );
    assert_eq!(
        back.session().consecutive_failures,
        0,
        "and being offline is no failed attempt"
    );
}

/// A cooldown that a restart forgets is not a cooldown.
///
/// It used to live only in the keyring, so on a machine with none — or one that is locked —
/// closing the window forgot it, and somebody mistyping a password could go on doing it as
/// fast as they could reopen the app. Hammering sign-in is what gets an account temporarily
/// locked, and with other people using this it is their account at stake, not ours.
#[tokio::test]
async fn a_new_client_still_owes_the_wait_the_old_one_earned() {
    let server = MockServer::start().await;
    mount_init(&server, 429).await;

    let dir = tempfile::tempdir().unwrap();
    {
        let mut client = client_for(&server, dir.path());
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap_err();
        assert!(client.session().retry_after.is_some());
    }

    // A second client over the same directory: the app, reopened.
    let mut again = client_for(&server, dir.path());
    let err = again
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap_err();
    assert!(
        matches!(err, quince_auth::Error::CoolingDown { .. }),
        "reopening the app must not clear the wait, got: {err}"
    );
}

/// And the other half: it is owed by the account, not by the machine. Someone signing in with
/// a different Apple ID inherits nothing.
#[tokio::test]
async fn a_different_account_does_not_inherit_the_wait() {
    let server = MockServer::start().await;
    mount_init(&server, 429).await;

    let dir = tempfile::tempdir().unwrap();
    {
        let mut client = client_for(&server, dir.path());
        client
            .sign_in("first@example.invalid", "hunter2")
            .await
            .unwrap_err();
    }

    let mut other = client_for(&server, dir.path());
    let err = other
        .sign_in("second@example.invalid", "hunter2")
        .await
        .unwrap_err();
    assert!(
        !matches!(err, quince_auth::Error::CoolingDown { .. }),
        "a different account must start clean, got: {err}"
    );
}

/// Sign in once, all the way through the code, into `dir`.
async fn signed_in(dir: &std::path::Path) {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    let mut client = client_for(&server, dir);
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_two_factor("123456").await.unwrap();
}

/// Apple, days later: the cookies and the session token have both lapsed.
async fn lapsed(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(421))
        .up_to_n_times(1)
        .mount(server)
        .await;
    mount_init(server, 200).await;
}

/// "I was told I wouldn't need to sign in again": a session token lapses in days, and
/// without the password the only way back was the sign-in screen. Kept, it signs this
/// still-trusted computer back in — no screen, no code.
#[tokio::test]
async fn a_lapsed_session_signs_itself_back_in_without_a_code() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    lapsed(&server).await;
    mount_complete(&server, 200).await; // trusted: no code asked for
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    assert!(client.resume().await.unwrap(), "back in, by itself");
    assert!(!client.awaiting_code());
    let reqs = server.received_requests().await.unwrap();
    let complete = requests_to(&reqs, "/signin/complete");
    assert_eq!(complete.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&complete[0].body).unwrap();
    assert_eq!(
        body["trustTokens"][0], "trust-token-xyz",
        "this computer's trust went with it"
    );
    assert!(
        requests_to(&reqs, "/securitycode").is_empty(),
        "and no code was asked for"
    );
}

/// Apple busy or down just as a lapsed session signs itself back in: that is "not now", not
/// "signed out". Read as signed out, every sync during an outage told the person Apple had
/// signed them out, and the session token was dropped.
#[tokio::test]
async fn an_outage_while_signing_back_in_is_not_a_sign_out() {
    for (init, complete) in [(503, 200), (200, 502)] {
        let dir = tempfile::tempdir().unwrap();
        signed_in(dir.path()).await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/setup/ws/1/validate"))
            .respond_with(ResponseTemplate::new(421))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/setup/ws/1/accountLogin"))
            .respond_with(ResponseTemplate::new(421))
            .mount(&server)
            .await;
        mount_init(&server, init).await;
        Mock::given(method("POST"))
            .and(path("/appleauth/auth/signin/complete"))
            .respond_with(
                ResponseTemplate::new(complete).set_body_string("<html>Bad Gateway</html>"),
            )
            .mount(&server)
            .await;

        let mut client = client_for(&server, dir.path());
        let err = client
            .resume()
            .await
            .expect_err("an outage is said, not read as signed out");
        assert!(
            matches!(
                err,
                quince_auth::Error::Unavailable(_) | quince_auth::Error::RateLimited
            ),
            "{init}/{complete}: {err:?}"
        );
        assert!(!err.to_string().contains('<'), "no error page: {err}");
        assert!(
            client.session().session_token.is_some(),
            "the session is kept"
        );

        let back = client_for(&server, dir.path());
        assert!(
            back.has_stored_session(),
            "and still kept on the next launch"
        );
        assert!(back.session().session_token.is_some());
    }
}

/// When the trust itself has run out, Apple wants a code: the code screen is next, not the
/// sign-in screen, and the code finishes it.
#[tokio::test]
async fn when_the_trust_has_run_out_the_code_is_all_that_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    lapsed(&server).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    assert!(!client.resume().await.unwrap());
    assert!(client.awaiting_code(), "the code screen is next");
    client.submit_two_factor("123456").await.unwrap();
    assert!(!client.awaiting_code());
    assert_eq!(
        client.session().password.as_deref(),
        Some("hunter2"),
        "and it stays kept"
    );
}

/// A password is kept only once Apple has accepted it all the way, and signing out erases it.
#[tokio::test]
async fn a_password_is_kept_only_once_it_has_worked_and_forgotten_on_sign_out() {
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
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    assert!(client.session().password.is_none(), "not before the code");
    client.submit_two_factor("123456").await.unwrap();
    assert_eq!(client.session().password.as_deref(), Some("hunter2"));
    client.sign_out().unwrap();
    assert!(
        client_for(&server, dir.path()).session().password.is_none(),
        "gone on sign-out"
    );
}

/// "I never receive the verification code." Since iOS 26.4 Apple no longer pushes it on the
/// 409 by itself — the client has to ask, with a PUT to verify/trusteddevice/securitycode —
/// so a sign-in that needs a code asks straight away, from the icloud.com origin.
#[tokio::test]
async fn a_sign_in_that_needs_a_code_asks_apple_to_send_it() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    let outcome = client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    assert_eq!(outcome, SignInOutcome::NeedsTwoFactor);

    let reqs = server.received_requests().await.unwrap();
    let asked: Vec<_> = requests_to(&reqs, "/verify/trusteddevice/securitycode")
        .into_iter()
        .filter(|r| r.method.as_str() == "PUT")
        .collect();
    assert_eq!(asked.len(), 1, "the code was asked for");
    let origin = asked[0].headers.get("origin").and_then(|v| v.to_str().ok());
    assert_ne!(
        origin,
        Some(SRP_ORIGIN),
        "from the icloud.com origin, not idmsa's"
    );
}

/// A heartbeat that changes nothing writes nothing: the keyring is not rewritten on every sync
/// and every return to the window. One that brings something new is written as before.
#[tokio::test]
async fn a_heartbeat_writes_the_session_only_when_it_changed() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("scnt", "scnt-renewed")
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_two_factor("123456").await.unwrap();
    let stored = dir.path().join("session.json");
    assert!(stored.exists());

    // The first heartbeat of a run writes once; the next, with nothing new, does not.
    assert!(client.heartbeat().await.unwrap());
    std::fs::remove_file(&stored).unwrap();
    assert!(client.heartbeat().await.unwrap());
    assert!(!stored.exists(), "an unchanged session was written again");

    // Apple hands over something new: written.
    assert!(client.heartbeat().await.unwrap());
    let kept = std::fs::read_to_string(&stored).expect("a changed session is written");
    assert!(kept.contains("scnt-renewed"));
}

/// No Secret Service (a window manager without a keyring, a session over SSH): the store
/// refuses every write. The password went through and then the sign-in failed on that write,
/// before the code was even asked for, so such a machine could never sign in at all. Now it
/// signs in, holds the session in memory, says so, and writes it once the store takes it.
#[tokio::test]
async fn a_store_that_will_not_keep_the_session_never_fails_the_sign_in() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    // Where the session would be written is a folder: every save fails.
    let blocked = dir.path().join("session.json");
    std::fs::create_dir(&blocked).unwrap();
    let mut client = client_for(&server, dir.path());

    let outcome = client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .expect("the password was accepted: the code is asked for");
    assert_eq!(outcome, SignInOutcome::NeedsTwoFactor);
    client
        .submit_two_factor("123456")
        .await
        .expect("signed in, if only for as long as it runs");
    assert!(client.session().webservices.is_some());
    assert!(client.unsaved().is_some(), "said that it is not kept");

    // A keyring comes up: the next heartbeat writes it, and nothing is said any more.
    std::fs::remove_dir(&blocked).unwrap();
    assert!(client.heartbeat().await.unwrap());
    assert!(client.unsaved().is_none());
    assert!(client_for(&server, dir.path()).has_stored_session());
}

/// A locked account is also a 403 from `/signin/complete`, with Apple's code -20209. It must
/// say the account is locked and where to unlock it, never that the password was wrong.
#[tokio::test]
async fn a_locked_account_says_so_not_wrong_password() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "serviceErrors": [{
                "code": "-20209",
                "message": "This Apple Account has been locked for security reasons."
            }]
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    let err = client
        .sign_in("probe@example.invalid", "pw")
        .await
        .unwrap_err();
    assert!(
        matches!(err, quince_auth::Error::AccountLocked),
        "got {err:?}"
    );
    assert!(err.to_string().contains("iforgot.apple.com"), "{err}");
}

/// Asking for the code again too often is answered 429. That reads as "wait", the same as
/// throttling anywhere else in sign-in, never as raw HTTP text, by either way of sending it.
#[tokio::test]
async fn asking_for_the_code_again_too_often_says_to_wait() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/phone"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();

    let err = client.resend_to_devices().await.unwrap_err();
    assert!(matches!(err, quince_auth::Error::RateLimited), "{err:?}");
    let err = client.send_sms(1).await.unwrap_err();
    assert!(matches!(err, quince_auth::Error::RateLimited), "{err:?}");
}

/// Apple can refuse a locked account before the password is even tried: a 403 from
/// `/signin/init`, or a 401 from `/signin/complete`, each carrying code -20209
/// (timlaing/pyicloud#368, icloud_photos_downloader#1010). Read as a wrong password, a lapsed
/// session signing itself back in threw away the password it keeps.
#[tokio::test]
async fn a_locked_account_refused_at_any_step_keeps_the_password() {
    let locked = json!({
        "serviceErrors": [{
            "code": "-20209",
            "message": "This Apple Account has been locked for security reasons."
        }]
    });
    for (step, status) in [("init", 403), ("init", 401), ("complete", 401)] {
        let dir = tempfile::tempdir().unwrap();
        signed_in(dir.path()).await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/setup/ws/1/validate"))
            .respond_with(ResponseTemplate::new(421))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/setup/ws/1/accountLogin"))
            .respond_with(ResponseTemplate::new(421))
            .mount(&server)
            .await;
        if step == "init" {
            Mock::given(method("POST"))
                .and(path("/appleauth/auth/signin/init"))
                .respond_with(ResponseTemplate::new(status).set_body_json(locked.clone()))
                .mount(&server)
                .await;
        } else {
            mount_init(&server, 200).await;
            Mock::given(method("POST"))
                .and(path("/appleauth/auth/signin/complete"))
                .respond_with(ResponseTemplate::new(status).set_body_json(locked.clone()))
                .mount(&server)
                .await;
        }

        let mut client = client_for(&server, dir.path());
        assert!(
            !client.resume().await.unwrap(),
            "{step} {status}: not back in"
        );
        assert_eq!(
            client.session().password.as_deref(),
            Some("hunter2"),
            "{step} {status}: a locked account is no wrong password; the password stays kept"
        );
        // Signed in by hand (on another install: this one now owes a wait), it says why.
        let fresh = tempfile::tempdir().unwrap();
        let err = client_for(&server, fresh.path())
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap_err();
        assert!(
            matches!(err, quince_auth::Error::AccountLocked),
            "{step} {status}: {err:?}"
        );
    }
}

/// A plain 401 or 403 with no code of Apple's is still the wrong password.
#[tokio::test]
async fn a_bare_refusal_is_still_a_wrong_password() {
    for status in [401, 403] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/appleauth/auth/signin/init"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let mut client = client_for(&server, dir.path());
        let err = client
            .sign_in("someone@example.invalid", "nope")
            .await
            .unwrap_err();
        assert!(
            matches!(err, quince_auth::Error::BadCredentials),
            "{status}: {err:?}"
        );
    }
}

/// Since its 2026 sign-in Apple can keep the trusted numbers under
/// `direct.twoSV.bridgeInitiateData.phoneNumberVerification` (icloud_photos_downloader#1335),
/// or send its sign-in page with them in a `boot_args` script (timlaing/pyicloud#322). Read
/// only at the top, an account reached only by text showed "nowhere to send a code".
#[tokio::test]
async fn trusted_numbers_are_found_wherever_apple_keeps_them() {
    let nested = json!({
        "direct": {
            "hasTrustedDevices": false,
            "twoSV": {
                "bridgeInitiateData": {
                    "phoneNumberVerification": {
                        "trustedPhoneNumbers": [
                            { "id": 1, "numberWithDialCode": "+91 ••••• ••318", "pushMode": "sms" },
                            { "id": 2, "obfuscatedNumber": "(•••) •••-••12", "pushMode": "voice", "nonFTEU": true }
                        ],
                        "trustedPhoneNumber": { "id": 1, "numberWithDialCode": "+91 ••••• ••318", "pushMode": "sms" },
                        "securityCode": { "tooManyCodesSent": true }
                    }
                }
            }
        }
    });
    let page = format!(
        "<html><head><script type=\"application/json\" class=\"boot_args\">\n{nested}\n</script></head></html>"
    );
    for (label, reply) in [
        (
            "json",
            ResponseTemplate::new(200).set_body_json(nested.clone()),
        ),
        (
            "page",
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(page.clone()),
        ),
    ] {
        let server = MockServer::start().await;
        mount_init(&server, 200).await;
        mount_complete(&server, 409).await;
        Mock::given(method("GET"))
            .and(path("/appleauth/auth"))
            .respond_with(reply)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/appleauth/auth/verify/phone"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/appleauth/auth/verify/phone/securitycode"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        mount_trust_and_login(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let mut client = client_for(&server, dir.path());
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap();

        let options = client.two_factor_options().await.unwrap();
        let ids: Vec<u64> = options.phones.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2], "{label}: each number once");
        assert_eq!(options.phones[0].masked_number, "+91 ••••• ••318");
        assert_eq!(options.phones[1].mode, "voice", "{label}");
        assert_eq!(options.trusted_device_count, 0, "{label}");
        assert!(options.too_many_codes_sent, "{label}");

        // Sent and checked the way Apple reaches that number: by a call, with its flag.
        client.send_sms(2).await.unwrap();
        client.submit_sms_code(2, "123456").await.unwrap();
        let reqs = server.received_requests().await.unwrap();
        for sent in requests_to(&reqs, "/verify/phone")
            .into_iter()
            .chain(requests_to(&reqs, "/verify/phone/securitycode"))
        {
            let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
            assert_eq!(body["mode"], "voice", "{label}: {body}");
            assert_eq!(body["phoneNumber"]["id"], 2, "{label}: {body}");
            assert_eq!(body["phoneNumber"]["nonFTEU"], true, "{label}: {body}");
        }
    }
}

/// The reply as it has always been, with the numbers at the top: read as before, texted.
#[tokio::test]
async fn trusted_numbers_at_the_top_are_still_texted() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("GET"))
        .and(path("/appleauth/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "trustedDeviceCount": 2,
            "trustedPhoneNumbers": [{ "id": 7, "numberWithDialCode": "+1 (•••) •••-••01" }],
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/phone"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    let options = client.two_factor_options().await.unwrap();
    assert_eq!(options.trusted_device_count, 2);
    assert_eq!(options.phones.len(), 1);
    assert_eq!(options.phones[0].mode, "sms");
    client.send_sms(7).await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let sent = requests_to(&reqs, "/verify/phone");
    let body: serde_json::Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body, json!({ "phoneNumber": { "id": 7 }, "mode": "sms" }));
}

/// New iCloud terms not yet accepted: `accountLogin` answers 200 with `termsUpdateNeeded`
/// and opens the session only part-way (rclone/rclone#9919). Taken as signed in, every sync
/// failed after with nothing to say why. It says so, and costs no wait: no password was wrong.
#[tokio::test]
async fn new_terms_to_accept_are_said_not_taken_for_a_sign_in() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 200).await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "termsUpdateNeeded": true,
            "dsInfo": { "dsid": "1234567890" },
            "webservices": {}
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    for attempt in 0..2 {
        let err = client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap_err();
        assert!(
            matches!(err, quince_auth::Error::TermsPending),
            "attempt {attempt}: {err:?}"
        );
        assert!(err.to_string().contains("icloud.com"), "{err}");
    }
    assert!(client.session().webservices.is_none(), "not signed in");
}

/// The same for a session already kept: said, and kept, rather than signed in again with
/// the password into the same terms.
#[tokio::test]
async fn new_terms_on_a_kept_session_keep_it() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "termsUpdateNeeded": true })),
        )
        .mount(&server)
        .await;
    let mut client = client_for(&server, dir.path());
    let err = client.resume().await.unwrap_err();
    assert!(matches!(err, quince_auth::Error::TermsPending), "{err:?}");
    assert!(
        client.session().session_token.is_some(),
        "the session is kept"
    );
    assert_eq!(client.session().password.as_deref(), Some("hunter2"));
    let reqs = server.received_requests().await.unwrap();
    assert!(
        requests_to(&reqs, "/signin/init").is_empty(),
        "no sign-in into the same terms"
    );
}

/// The PUTs that ask Apple to send a code to the person's devices.
fn code_pushes(reqs: &[Request]) -> usize {
    requests_to(reqs, "/verify/trusteddevice/securitycode")
        .into_iter()
        .filter(|r| r.method.as_str() == "PUT")
        .count()
}

/// "My iPhone shows a code prompt out of nowhere." A lapse mended in the background — the
/// heartbeat, a sync, a launch at login — that Apple answers with "a code first" asks Apple to
/// send nothing: the person is nowhere near the code screen. The code is sent once they open
/// it, and only once.
#[tokio::test]
async fn signing_back_in_by_itself_never_pushes_a_code_to_the_phone() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    lapsed(&server).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let mut client = client_for(&server, dir.path());
    assert!(!client.resume().await.unwrap());
    assert!(client.awaiting_code(), "the code screen is what comes next");
    assert_eq!(
        code_pushes(&server.received_requests().await.unwrap()),
        0,
        "and nothing was sent to the phone meanwhile"
    );
    // A heartbeat later, still nothing.
    let _ = client.heartbeat().await;
    assert_eq!(code_pushes(&server.received_requests().await.unwrap()), 0);

    // The person opens the code screen: now it is sent — once, however often it opens.
    client.request_code().await.unwrap();
    client.request_code().await.unwrap();
    assert_eq!(code_pushes(&server.received_requests().await.unwrap()), 1);
}

/// Typing the password is asking for the code: sent straight away, as before — and the code
/// screen opening after it does not send a second.
#[tokio::test]
async fn a_sign_in_by_hand_sends_the_code_once() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::NeedsTwoFactor
    );
    client.request_code().await.unwrap();
    assert_eq!(code_pushes(&server.received_requests().await.unwrap()), 1);
    // Sending it again by hand always goes.
    client.resend_to_devices().await.unwrap();
    assert_eq!(code_pushes(&server.received_requests().await.unwrap()), 2);
}

/// Apple's sign-in leans on cookies it sets without an expiry (idmsa's own, some of
/// icloud.com's). `cookie_store`'s save dropped every one of them, so each start's sign-in came
/// back a stranger's and Apple asked for a code again — about daily. They are kept now, sealed
/// with the rest, and sent again after a restart; ones that ran out are not.
#[tokio::test]
async fn the_cookies_apple_sets_without_an_expiry_survive_a_restart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "aasp=idmsa-session; Path=/")
                .append_header("set-cookie", "dslang=GB-EN; Path=/")
                .set_body_json(json!({
                    "salt": b64(&[0x77; 16]), "b": b64(&[0x5e; 256]), "c": "challenge-c",
                    "iteration": 20173, "protocol": "s2k",
                })),
        )
        .mount(&server)
        .await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/appleauth/auth/2sv/trust"))
        .respond_with(
            ResponseTemplate::new(204)
                .insert_header("X-Apple-TwoSV-Trust-Token", "trust-token-xyz")
                .insert_header("X-Apple-Session-Token", "session-token-def"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "X-APPLE-WEBAUTH-HSA-LOGIN=login; Path=/")
                .append_header("set-cookie", "X-APPLE-WEBAUTH-USER=user; Path=/; Max-Age=2592000")
                .append_header("set-cookie", "gone=old; Path=/; Max-Age=0")
                .set_body_json(json!({
                    "dsInfo": { "dsid": "1234567890" },
                    "webservices": { "calendar": { "url": "https://p00-calendarws.test.invalid:443" } }
                })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let client_id = {
        let mut client = client_for(&server, dir.path());
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap();
        client.submit_two_factor("123456").await.unwrap();
        client.session().client_id.clone()
    };
    assert!(client_id.is_some());

    // The next start.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let mut again = client_for(&server, dir.path());
    assert_eq!(
        again.session().client_id,
        client_id,
        "the same client id: Apple ties the trust to it"
    );
    assert!(again.resume().await.unwrap());
    let reqs = server.received_requests().await.unwrap();
    let validate = requests_to(&reqs, "/setup/ws/1/validate");
    let sent: Vec<String> = validate[0]
        .headers
        .get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.split('=').next())
        .map(|n| n.trim().to_string())
        .collect();
    for name in [
        "aasp",
        "dslang",
        "X-APPLE-WEBAUTH-HSA-LOGIN",
        "X-APPLE-WEBAUTH-USER",
    ] {
        assert!(
            sent.iter().any(|n| n == name),
            "{name} came back after a restart: {sent:?}"
        );
    }
    assert!(
        !sent.iter().any(|n| n == "gone"),
        "one that ran out did not"
    );
}

/// The whole promise, end to end: a computer Apple already trusts, signing in again with its
/// trust token, gets no code — nothing pushed, no code screen.
#[tokio::test]
async fn a_trusted_computer_signing_in_again_is_asked_for_no_code() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    // Apple honours the trust token only when it is sent: 409 without it, 200 with it.
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "trustTokens": ["trust-token-xyz"] }),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Apple-Session-Token", "session-token-new")
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;
    mount_complete(&server, 409).await;
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::Complete
    );
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(code_pushes(&reqs), 0, "nothing sent to the phone");
    assert!(
        requests_to(&reqs, "/2sv/trust").is_empty(),
        "no code to trust"
    );
    assert_eq!(
        client.session().trust_token.as_deref(),
        Some("trust-token-xyz"),
        "and the trust is kept as it was"
    );
}

/// A cooldown set while the clock ran ahead — a dual-boot computer whose hardware clock keeps
/// local time, +5:30 — seemed to have hours left once the clock was put right, and sign-in
/// stayed refused all that time. No wait Skykeep sets is longer than ten minutes, so one that
/// seems longer is over; a real one still holds, and still spares Apple.
#[test]
fn a_cooldown_from_a_clock_that_ran_ahead_is_over() {
    use quince_auth::session::wait_left;
    let now = 1_800_000_000;
    assert_eq!(
        wait_left(Some(now + 30), now),
        Some(30),
        "a real wait holds"
    );
    assert_eq!(
        wait_left(Some(now + 600), now),
        Some(600),
        "the longest one Skykeep sets"
    );
    assert_eq!(
        wait_left(Some(now + 5 * 3600 + 30 * 60), now),
        None,
        "five and a half hours ahead: over"
    );
    assert_eq!(wait_left(Some(now - 1), now), None, "run out");
    assert_eq!(wait_left(Some(now), now), None);
    assert_eq!(wait_left(None, now), None);
}

#[tokio::test]
async fn sign_in_is_not_held_up_by_a_cooldown_from_a_clock_that_ran_ahead() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::file(dir.path()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    store
        .save_cooldown(&quince_auth::session::Cooldown {
            account: Some(quince_auth::session::account_fingerprint(
                "someone@example.invalid",
            )),
            consecutive_failures: 3,
            retry_after: Some(now + 5 * 3600 + 30 * 60),
        })
        .unwrap();
    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::NeedsTwoFactor,
        "the sign-in goes ahead"
    );
    assert_eq!(
        requests_to(&server.received_requests().await.unwrap(), "/signin/init").len(),
        1
    );
}

/// `/escrow/init` answering a well-formed challenge, and `/escrow/complete` a new session token.
async fn mount_escrow(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/escrow/init"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "salt": b64(&[0x31; 16]),
            "b": b64(&[0x6a; 256]),
            "c": "escrow-c",
            "iteration": 20173,
            "protocol": "s2k",
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/escrow/complete"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Apple-Session-Token", "session-token-escrow")
                .set_body_json(json!({})),
        )
        .mount(server)
        .await;
}

/// The daily code: a computer Apple already trusts signs in again, and Apple answers 409 with
/// `X-Apple-EDP` — "trust token accepted, now the escrow step" (rclone#10048,
/// icloud-photos-sync#1124). Read as "a code", every lapse asked for one. Now: the escrow step,
/// then the account — no code, nothing pushed to the phone.
#[tokio::test]
async fn a_trusted_sign_in_that_asks_for_escrow_needs_no_code() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("X-Apple-EDP", "1")
                .insert_header("X-Apple-Session-Token", "session-token-abc")
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;
    mount_escrow(&server).await;
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::Complete
    );
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(code_pushes(&reqs), 0, "no code asked for");
    let init = requests_to(&reqs, "/escrow/init");
    assert_eq!(init.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&init[0].body).unwrap();
    assert_eq!(
        body["accountName"], "",
        "the escrow proof is made with an empty account name"
    );
    assert_eq!(body["protocols"], json!(["s2k", "s2k_fo"]));
    let complete = requests_to(&reqs, "/escrow/complete");
    assert_eq!(complete.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&complete[0].body).unwrap();
    assert_eq!(body["c"], "escrow-c", "the challenge is echoed");
    for key in ["m1", "m2", "k"] {
        let v = body[key].as_str().unwrap_or_default();
        assert_eq!(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, v)
                .map(|b| b.len())
                .unwrap_or(0),
            32,
            "{key} is a 32-byte value"
        );
    }
    assert!(
        requests_to(&reqs, "/setup/ws/1/accountLogin").len() == 1,
        "and the account is opened"
    );
}

/// Without a trust token a 409 is a code, escrow header or not: a first sign-in never skips it.
#[tokio::test]
async fn a_first_sign_in_that_mentions_escrow_still_asks_for_a_code() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("X-Apple-EDP", "1")
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::NeedsTwoFactor
    );
    assert!(requests_to(&server.received_requests().await.unwrap(), "/escrow/init").is_empty());
}

/// An accepted code whose answer asks for escrow: the escrow step comes before the trust
/// token is asked for — without it Apple does not honour that token next time.
#[tokio::test]
async fn an_accepted_code_that_asks_for_escrow_completes_it_before_trust() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("X-Apple-PDP", "1")
                .insert_header("X-Apple-Session-Token", "session-token-code")
                .set_body_json(json!({ "securityCode": { "valid": true } })),
        )
        .mount(&server)
        .await;
    mount_escrow(&server).await;
    mount_trust_and_login(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_two_factor("123456").await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    let order: Vec<&str> = reqs
        .iter()
        .map(|r| r.url.path())
        .filter(|p| p.ends_with("/escrow/complete") || p.ends_with("/2sv/trust"))
        .collect();
    assert_eq!(
        order,
        [
            "/appleauth/auth/escrow/complete",
            "/appleauth/auth/2sv/trust"
        ],
        "escrow first, then trust"
    );
    assert_eq!(
        client.session().trust_token.as_deref(),
        Some("trust-token-xyz")
    );
}

/// `/signin/complete` answering 412: Apple's repair step, as icloudpd takes it, then the account
/// — not "two-factor auth isn't switched on".
#[tokio::test]
async fn a_412_at_sign_in_is_repaired_and_carries_on() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 412).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/repair/complete"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::Complete
    );
    let reqs = server.received_requests().await.unwrap();
    let repair = requests_to(&reqs, "/repair/complete");
    assert_eq!(repair.len(), 1);
    assert_eq!(repair[0].body, b"{}");
}

/// A repair Apple refuses is said as what to do — at account.apple.com — and is not counted as
/// a wrong password.
#[tokio::test]
async fn a_repair_apple_refuses_says_where_to_finish_it() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 412).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/repair/complete"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    let err = client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap_err();
    assert!(matches!(err, quince_auth::Error::RepairNeeded), "{err:?}");
    assert!(err.to_string().contains("account.apple.com"));
    assert_eq!(client.session().consecutive_failures, 0);
}

/// A texted code answered 412 with a repair session token was accepted (pyicloud#466): the
/// sign-in finishes, never "That code didn't match".
#[tokio::test]
async fn a_texted_code_answered_412_with_a_repair_token_is_accepted() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 409).await;
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/phone/securitycode"))
        .respond_with(
            ResponseTemplate::new(412)
                .insert_header("X-Apple-Repair-Session-Token", "repair-token")
                .insert_header("X-Apple-Session-Token", "session-token-sms")
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_sms_code(1, "123456").await.unwrap();
    assert_eq!(
        client.session().trust_token.as_deref(),
        Some("trust-token-xyz")
    );
}

/// An account that signs in with security keys is told so, not left at a code screen.
#[tokio::test]
async fn an_account_with_security_keys_is_recognised() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/appleauth/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "trustedDeviceCount": 0,
            "fsaChallenge": { "challenge": "Y2hhbGxlbmdl", "keyHandles": ["aGFuZGxl"], "rpId": "apple.com" },
            "keyNames": ["Key"]
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    assert!(client.two_factor_options().await.unwrap().security_key);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/appleauth/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "trustedDeviceCount": 1, "fsaChallenge": { "challenge": "", "keyHandles": [] }
        })))
        .mount(&server)
        .await;
    let mut client = client_for(&server, dir.path());
    assert!(
        !client.two_factor_options().await.unwrap().security_key,
        "an empty challenge is no security key"
    );
}

/// An account Apple answers with no services at all (seen with a beta of iOS on it,
/// icloudpd#1361): signed in, nothing to read, nothing crashes.
#[tokio::test]
async fn an_account_with_no_services_signs_in_and_offers_none() {
    let server = MockServer::start().await;
    mount_init(&server, 200).await;
    mount_complete(&server, 200).await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890", "hasUnreleasedOS": true },
            "webservices": {},
            "configBag": { "urls": {} }
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    assert_eq!(
        client
            .sign_in("someone@example.invalid", "hunter2")
            .await
            .unwrap(),
        SignInOutcome::Complete
    );
    assert!(client.session().service_url("ckdatabasews").is_none());
    assert!(client.data_access_missing().is_empty());
}
