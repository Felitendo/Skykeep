//! Issue #5: sign-in works, then every service — Calendar and Contacts too — answers "Missing
//! X-APPLE-WEBAUTH-TOKEN cookie" or "no auth method found". With new iCloud terms waiting,
//! Apple's `accountLogin` says `termsUpdateNeeded` and opens only part of a web session
//! (rclone#9658). Said as that, nothing more asked of Apple meanwhile, and cleared by the next
//! `accountLogin` once the person has agreed at icloud.com.
//!
//! And diagnostics: on, they name each request's place, status and cookies — never a value.

use quince_auth::pcs::DataAccess;
use quince_auth::{Client, Endpoints, Store};
use serde_json::json;
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as E, Engine};
    E.encode(bytes)
}

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn sink(line: &str) {
    LINES.lock().unwrap().push(line.to_string());
}

async fn mount_sign_in(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "aasp=SECRET-AASP; Path=/; Secure; HttpOnly")
                .set_body_json(json!({
                    "salt": b64(&[0x77; 16]), "b": b64(&[0x5e; 256]), "c": "challenge-c",
                    "iteration": 20173, "protocol": "s2k",
                })),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-Apple-Session-Token", "session-token-abc")
                .set_body_json(json!({})),
        )
        .mount(server)
        .await;
}

fn login(terms: bool) -> ResponseTemplate {
    let mut r = ResponseTemplate::new(200)
        .append_header(
            "set-cookie",
            "X-APPLE-WEBAUTH-USER=\"v=1:s=0:d=SECRET-USER\"; Path=/; Max-Age=2592000",
        )
        .append_header(
            "set-cookie",
            "X-APPLE-WEBAUTH-HSA-TRUST=\"SECRET-TRUST\"; Path=/; Max-Age=2592000",
        );
    if !terms {
        r = r.append_header(
            "set-cookie",
            "X-APPLE-WEBAUTH-TOKEN=\"v=2:t=SECRET-TOKEN\"; Path=/",
        );
    }
    r.set_body_json(json!({
        "dsInfo": { "dsid": "1234567890", "isWebAccessAllowed": true, "fullName": "Asha Rao",
                    "primaryEmail": "asha@example.invalid" },
        "iCDPEnabled": true,
        "termsUpdateNeeded": terms,
        "webservices": {
            "calendar": { "url": "https://p00-calendarws.test.invalid:443" },
            "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "pcsRequired": true },
            "drivews": { "url": "https://p00-drivews.test.invalid:443", "pcsRequired": true }
        }
    }))
}

fn client_for(server: &MockServer, dir: &std::path::Path) -> Client {
    Client::with(dir, Endpoints::at(&server.uri()), Store::file(dir).unwrap()).unwrap()
}

#[tokio::test]
async fn new_terms_are_said_waited_on_and_cleared_once_agreed() {
    quince_auth::diag::set_sink(sink);
    let server = MockServer::start().await;
    mount_sign_in(&server).await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(login(true))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(login(false))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let mut client = client_for(&server, dir.path());
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    assert!(
        client.session().gate.terms_update_needed(),
        "Apple's flag is kept"
    );
    assert!(client.waits_on_icloud_com());
    // Advanced Data Protection's keys are not asked for meanwhile: Apple would refuse them.
    assert!(client.data_access_missing().is_empty());
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Open);
    let reqs = server.received_requests().await.unwrap();
    assert!(
        !reqs
            .iter()
            .any(|r| r.url.path().contains("requestWebAccessState")),
        "nothing asked of Apple about keys while the terms wait"
    );

    // Agreed at icloud.com, then Try Again.
    client.refresh_account().await.unwrap();
    assert!(
        !client.session().gate.terms_update_needed(),
        "cleared by the next accountLogin"
    );
    assert!(!client.waits_on_icloud_com());
    assert_eq!(
        client.data_access_missing(),
        ["notes", "reminders", "photos", "drive"]
    );
    // And remembered across a restart either way.
    drop(client);
    assert!(!client_for(&server, dir.path())
        .session()
        .gate
        .terms_update_needed());

    // Diagnostics: places, statuses, flags and cookie names — no value, token, name or address.
    let lines = LINES.lock().unwrap().join("\n");
    assert!(lines.contains("termsUpdateNeeded=true"), "{lines}");
    assert!(lines.contains("termsUpdateNeeded=false"));
    assert!(
        lines.contains("← 200 127.0.0.1/setup/ws/1/accountLogin"),
        "{lines}"
    );
    assert!(
        lines.contains("X-APPLE-WEBAUTH-TOKEN domain=(host) path=/ session kept"),
        "{lines}"
    );
    assert!(
        lines.contains("→ 127.0.0.1/setup/ws/1/accountLogin cookies"),
        "{lines}"
    );
    for secret in [
        "SECRET",
        "hunter2",
        "session-token-abc",
        "Asha",
        "asha@example.invalid",
        "1234567890",
        "someone@example.invalid",
    ] {
        assert!(
            !lines.contains(secret),
            "{secret} written to diagnostics:\n{lines}"
        );
    }
}
