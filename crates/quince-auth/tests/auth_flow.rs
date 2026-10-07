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
