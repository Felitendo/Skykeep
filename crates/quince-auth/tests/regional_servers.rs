//! Accounts Apple keeps on its regional servers (issue #10): Apple answers `accountLogin` at
//! iCloud.com with `302 {"domainToUse": "iCloud.com.cn"}`, and the account is reached only at
//! Apple's servers there. Followed by itself — after a code, with a new code where Apple asks
//! for one — remembered, and undone at sign-out; any other domain is refused. Its own file, run
//! one test at a time: which domains the lock opens is the process's.

use quince_auth::{is_apple_host, reachable_domains, web_origin, Client, Endpoints, Error, Store};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const HOME_ORIGIN: &str = "https://web.test.invalid";
const REGIONAL_ORIGIN: &str = "https://web-regional.test.invalid";

fn endpoints_for(server: &MockServer, home: &str) -> Endpoints {
    let base = server.uri();
    Endpoints {
        auth_root: "https://idmsa.test.invalid".into(),
        home: home.into(),
        auth: format!("{base}/appleauth/auth"),
        setup: format!("{base}/setup/ws/1"),
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as E, Engine};
    E.encode(bytes)
}

/// Apple's sign-in up to its code: a challenge, then 409 for the code, which is taken.
async fn mount_sign_in(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "salt": b64(&[0x77; 16]),
            "b": b64(&[0x5e; 256]),
            "c": "challenge-c",
            "iteration": 20173,
            "protocol": "s2k",
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("X-Apple-Session-Token", "session-token")
                .set_body_json(json!({})),
        )
        .mount(server)
        .await;
    for p in ["/appleauth/auth/verify/trusteddevice/securitycode"] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(204))
            .mount(server)
            .await;
    }
    // Asking for the code to be sent, as a sign-in someone watches does.
    Mock::given(method("PUT"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(202))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/appleauth/auth/2sv/trust"))
        .respond_with(
            ResponseTemplate::new(204)
                .insert_header("X-Apple-TwoSV-Trust-Token", "trust-token")
                .insert_header("X-Apple-Session-Token", "session-token-2"),
        )
        .mount(server)
        .await;
}

/// iCloud.com, for an account kept elsewhere: everything as usual until `accountLogin`.
async fn mount_moved(server: &MockServer, to: &str) {
    mount_sign_in(server).await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(302).set_body_json(json!({ "domainToUse": to })))
        .mount(server)
        .await;
}

async fn mount_opened(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "2026110700" },
            "webservices": {
                "ckdatabasews": { "url": "https://p01-ckdatabasews.icloud.com.cn:443", "status": "active" }
            }
        })))
        .mount(server)
        .await;
}

fn hits<'a>(reqs: &'a [Request], suffix: &str) -> Vec<&'a Request> {
    reqs.iter()
        .filter(|r| r.url.path().ends_with(suffix))
        .collect()
}

fn origin(r: &Request) -> &str {
    r.headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// The issue's own path: the code is taken at iCloud.com, Apple then names its regional
/// servers, and the sign-in made at iCloud.com isn't taken there. Signed in again there, the
/// person is asked for the new code Apple sent, and is in.
#[tokio::test]
async fn an_account_kept_regionally_signs_in_there_after_a_new_code() {
    let _one = ONE_AT_A_TIME.lock().await;
    let home = MockServer::start().await;
    let regional = MockServer::start().await;
    mount_moved(&home, "iCloud.com.cn").await;
    mount_sign_in(&regional).await;
    // The iCloud.com sign-in, once, refused there; the one made there, taken.
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(421).set_body_json(json!({ "error": 1 })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&regional)
        .await;
    mount_opened(&regional).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::with(
        dir.path(),
        endpoints_for(&home, HOME_ORIGIN),
        Store::file(dir.path()).unwrap(),
    )
    .unwrap();
    client.regional_endpoints_at(endpoints_for(&regional, REGIONAL_ORIGIN));
    assert!(
        !is_apple_host("setup.icloud.com.cn"),
        "closed before any account needs it"
    );

    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    let again = client.submit_two_factor("111111").await.unwrap_err();
    assert!(matches!(again, Error::CodeAgain), "{again}");
    assert!(
        !again.to_string().to_lowercase().contains("china"),
        "{again}"
    );
    client.submit_two_factor("222222").await.unwrap();

    let session = client.session();
    assert_eq!(session.domain.as_deref(), Some("icloud.com.cn"));
    assert_eq!(session.dsid.as_deref(), Some("2026110700"));
    let seen = regional.received_requests().await.unwrap();
    assert_eq!(
        hits(&seen, "/signin/init").len(),
        1,
        "signed in again there, once"
    );
    let logins = hits(&seen, "/accountLogin");
    assert_eq!(logins.len(), 2);
    assert!(
        logins.iter().all(|r| origin(r) == REGIONAL_ORIGIN),
        "its servers' origin"
    );
    // Its servers are reachable, and said to be, only now; the web origin follows.
    assert!(is_apple_host("setup.icloud.com.cn"));
    assert!(is_apple_host("cvws.icloud-content.com.cn"));
    assert!(reachable_domains().contains(&"icloud.com.cn"));
    assert_eq!(web_origin(), "https://www.icloud.com.cn");

    // Kept: the next launch starts there.
    let reloaded = Client::with(
        dir.path(),
        endpoints_for(&home, HOME_ORIGIN),
        Store::file(dir.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(reloaded.session().domain.as_deref(), Some("icloud.com.cn"));

    // Signing out: the next account starts at iCloud.com, and the regional domains close.
    client.sign_out().unwrap();
    assert!(!is_apple_host("setup.icloud.com.cn"));
    assert!(!reachable_domains().contains(&"icloud.com.cn"));
    assert_eq!(web_origin(), "https://www.icloud.com");
}

/// Where Apple's regional servers take the sign-in already made: in, with no second code.
#[tokio::test]
async fn an_account_kept_regionally_needs_no_second_code_when_apple_takes_the_first() {
    let _one = ONE_AT_A_TIME.lock().await;
    let home = MockServer::start().await;
    let regional = MockServer::start().await;
    mount_moved(&home, "iCloud.com.cn").await;
    mount_opened(&regional).await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::with(
        dir.path(),
        endpoints_for(&home, HOME_ORIGIN),
        Store::file(dir.path()).unwrap(),
    )
    .unwrap();
    client.regional_endpoints_at(endpoints_for(&regional, REGIONAL_ORIGIN));
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_two_factor("111111").await.unwrap();

    assert_eq!(client.session().domain.as_deref(), Some("icloud.com.cn"));
    let seen = regional.received_requests().await.unwrap();
    assert!(hits(&seen, "/signin/init").is_empty(), "no second sign-in");
    client.sign_out().unwrap();
}

/// A domain Apple names that isn't one Skykeep knows is said plainly, and opens nothing.
#[tokio::test]
async fn an_unknown_domain_opens_nothing() {
    let _one = ONE_AT_A_TIME.lock().await;
    let home = MockServer::start().await;
    mount_moved(&home, "icloud.example").await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::with(
        dir.path(),
        endpoints_for(&home, HOME_ORIGIN),
        Store::file(dir.path()).unwrap(),
    )
    .unwrap();
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    let err = client.submit_two_factor("111111").await.unwrap_err();
    assert!(err.to_string().contains("can't open"), "{err}");
    assert_eq!(client.session().domain, None);
    assert!(!is_apple_host("setup.icloud.com.cn"));
    assert!(!is_apple_host("icloud.example"));
}
