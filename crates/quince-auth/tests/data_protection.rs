//! Advanced Data Protection (issue #4) against a mock Apple: every account shape the approval
//! flow meets, so what it asks of Apple — and what it never asks — is pinned in CI rather than
//! found on someone's account.
//!
//! The conversation is icloud.com's own, as foxt/icloud.js#4, rclone's iclouddrive backend,
//! icloudpy#214 and timlaing/pyicloud record it: `requestWebAccessState`, then
//! `enableDeviceConsentForPCS` while no approval is given, then `requestPCS` per service.

use quince_auth::pcs::{follow, DataAccess};
use quince_auth::{Client, Endpoints, Error, Store};
use serde_json::{json, Value};
use std::time::Duration;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const STATE: &str = "/setup/ws/1/requestWebAccessState";
const CONSENT: &str = "/setup/ws/1/enableDeviceConsentForPCS";
const PCS: &str = "/setup/ws/1/requestPCS";

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as E, Engine};
    E.encode(bytes)
}

/// The account as `accountLogin` describes it.
#[derive(Clone, Copy)]
struct Shape {
    /// `iCDPEnabled`.
    adp: Option<bool>,
    /// `dsInfo.isWebAccessAllowed`.
    web_access: Option<bool>,
    /// `pcsRequired` on the encrypted services.
    pcs_required: bool,
    /// Apple offers iCloud Drive to the account at all.
    drive: bool,
}

const ADP: Shape = Shape {
    adp: Some(true),
    web_access: Some(true),
    pcs_required: true,
    drive: true,
};
const STANDARD: Shape = Shape {
    adp: None,
    web_access: None,
    pcs_required: false,
    drive: true,
};

fn login_body(shape: Shape) -> Value {
    let mut ws = json!({
        "calendar": { "url": "https://p00-calendarws.test.invalid:443", "status": "active" },
        "contacts": { "url": "https://p00-contactsws.test.invalid:443", "status": "active" },
        "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active",
                          "pcsRequired": shape.pcs_required },
    });
    if shape.drive {
        ws["drivews"] = json!({ "url": "https://p00-drivews.test.invalid:443", "status": "active",
                                "pcsRequired": shape.pcs_required });
    }
    let mut ds = json!({ "dsid": "1234567890" });
    if let Some(w) = shape.web_access {
        ds["isWebAccessAllowed"] = json!(w);
    }
    let mut body = json!({ "dsInfo": ds, "webservices": ws });
    if let Some(a) = shape.adp {
        body["iCDPEnabled"] = json!(a);
    }
    body
}

/// Signed in to an account of `shape`, with a code from a trusted device — or, `sms`, with
/// one texted to a phone: how the code came makes no difference to what follows.
async fn signed_in(server: &MockServer, dir: &std::path::Path, shape: Shape, sms: bool) -> Client {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "salt": b64(&[0x77; 16]), "b": b64(&[0x5e; 256]), "c": "challenge-c",
            "iteration": 20173, "protocol": "s2k",
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("X-Apple-Session-Token", "session-token-abc")
                .insert_header("X-Apple-ID-Account-Country", "IND")
                .set_body_json(json!({})),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/phone/securitycode"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server)
        .await;
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
        .respond_with(
            ResponseTemplate::new(200)
                .append_header(
                    "set-cookie",
                    "X-APPLE-WEBAUTH-TOKEN=web-token; Path=/; Max-Age=86400",
                )
                .set_body_json(login_body(shape)),
        )
        .mount(server)
        .await;

    let base = server.uri();
    let endpoints = Endpoints {
        auth_root: base.clone(),
        home: base.clone(),
        auth: format!("{base}/appleauth/auth"),
        setup: format!("{base}/setup/ws/1"),
    };
    let mut client = Client::with(dir, endpoints, Store::file(dir).unwrap()).unwrap();
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    if sms {
        client.submit_sms_code(1, "123456").await.unwrap();
    } else {
        client.submit_two_factor("123456").await.unwrap();
    }
    client
}

fn state(consented: bool, expiry_ms: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "dsid": 1234567890, "isWebAccessAllowed": true, "isDeviceConsentedForPCS": consented,
        "isICDRSDisabled": true, "deviceConsentForPCSExpiry": expiry_ms,
    }))
}

fn in_an_hour_ms() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    u64::try_from(now).unwrap() + 3_600_000
}

/// `requestPCS` attaching a service's keys, with the cookies icloud.com is given for it.
async fn mount_keys(server: &MockServer, max_age: u32) {
    let attached = |cookies: &[&str]| {
        let mut r = ResponseTemplate::new(200).set_body_json(json!({
            "isWebAccessAllowed": true, "isDeviceConsentedForPCS": true, "isICDRSDisabled": true,
            "message": "Cookies attached.", "status": "success",
        }));
        for c in cookies {
            r = r.append_header(
                "set-cookie",
                format!("{c}=key; Path=/; Max-Age={max_age}").as_str(),
            );
        }
        r
    };
    for (app, cookies) in [
        (
            "photos",
            vec!["X-APPLE-WEBAUTH-PCS-Photos", "X-APPLE-WEBAUTH-PCS-Sharing"],
        ),
        ("iclouddrive", vec!["X-APPLE-WEBAUTH-PCS-Documents"]),
        ("notes3", vec!["X-APPLE-WEBAUTH-PCS-Notes"]),
        ("reminders", vec![]),
    ] {
        Mock::given(method("POST"))
            .and(path(PCS))
            .and(body_partial_json(json!({ "appName": app })))
            .respond_with(attached(&cookies))
            .mount(server)
            .await;
    }
}

fn to<'a>(reqs: &'a [Request], p: &str) -> Vec<&'a Request> {
    reqs.iter().filter(|r| r.url.path() == p).collect()
}

async fn asked(server: &MockServer, p: &str) -> usize {
    to(&server.received_requests().await.unwrap(), p).len()
}

fn app_names(reqs: &[&Request]) -> Vec<String> {
    reqs.iter()
        .map(|r| {
            let v: Value = serde_json::from_slice(&r.body).unwrap();
            v["appName"].as_str().unwrap().to_string()
        })
        .collect()
}

/// Waits of nothing, a few times: `follow` without the clock.
fn quick(n: u32) -> Option<Duration> {
    (n < 8).then_some(Duration::ZERO)
}

/// Without Advanced Data Protection nothing changes: no approval, and not one request more.
#[tokio::test]
async fn an_account_without_advanced_data_protection_is_never_asked_anything() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), STANDARD, false).await;
    assert!(client.data_access_missing().is_empty());
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Open);
    for p in [STATE, CONSENT, PCS] {
        assert_eq!(asked(&server, p).await, 0, "{p} is never asked");
    }
}

/// `pcsRequired` alone does not prove Advanced Data Protection (icloudpy#214): Apple is asked
/// once, says recovery is on, and is not asked again every pass.
#[tokio::test]
async fn pcs_required_with_recovery_on_asks_once_and_needs_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let shape = Shape { adp: None, ..ADP };
    let mut client = signed_in(&server, dir.path(), shape, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsid": 1234567890, "isWebAccessAllowed": true
        })))
        .mount(&server)
        .await;
    // `pcsRequired` alone is no evidence: nothing held back, nothing asked.
    assert!(client.data_access_missing().is_empty());
    assert_eq!(client.data_access_standing(), DataAccess::Open);
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Open);
    assert_eq!(asked(&server, STATE).await, 0, "nothing asked of Apple");

    // iCDPEnabled, and Apple says recovery is on: asked once, then remembered, a restart too.
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsid": 1234567890, "isWebAccessAllowed": true, "isICDRSDisabled": false
        })))
        .mount(&server)
        .await;
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Open);
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Open);
    assert_eq!(
        asked(&server, STATE).await,
        1,
        "asked once, then remembered"
    );
    assert_eq!(
        asked(&server, CONSENT).await,
        0,
        "no prompt on anyone's phone"
    );
    assert!(client.data_access_missing().is_empty());
    drop(client);
    let again = Client::with(
        dir.path(),
        Endpoints::at(&server.uri()),
        Store::file(dir.path()).unwrap(),
    )
    .unwrap();
    assert!(
        again.data_access_missing().is_empty(),
        "and after a restart"
    );
}

/// Advanced Data Protection with web access off: the existing page, and nothing asked.
#[tokio::test]
async fn web_access_off_at_sign_in_is_said_without_asking() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let shape = Shape {
        web_access: Some(false),
        ..ADP
    };
    let mut client = signed_in(&server, dir.path(), shape, false).await;
    assert_eq!(
        client.data_access(true).await.unwrap(),
        DataAccess::WebAccessOff
    );
    assert_eq!(asked(&server, STATE).await, 0);
}

/// Turned off since sign-in: Apple's state says so, and the session remembers it.
#[tokio::test]
async fn web_access_turned_off_since_is_read_from_apple() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isWebAccessAllowed": false, "isICDRSDisabled": true
        })))
        .mount(&server)
        .await;
    assert_eq!(
        client.data_access(true).await.unwrap(),
        DataAccess::WebAccessOff
    );
    assert_eq!(client.session().gate.web_access_allowed(), Some(false));
    assert_eq!(asked(&server, CONSENT).await, 0);
}

/// Approved already (within the hour): each service's key is asked for by icloud.com's name
/// for it, once, and held; nothing is put on the person's phone.
#[tokio::test]
async fn an_approval_already_given_attaches_every_key_silently() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    assert_eq!(
        client.data_access_missing(),
        ["notes", "reminders", "photos", "drive"],
        "every covered service waits on its key"
    );
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    mount_keys(&server, 3600).await;

    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    let reqs = server.received_requests().await.unwrap();
    let pcs = to(&reqs, PCS);
    assert_eq!(
        app_names(&pcs),
        ["notes3", "reminders", "photos", "iclouddrive"]
    );
    for r in &pcs {
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(
            v["derivedFromUserAction"],
            json!(true),
            "the first ask for each says so"
        );
        assert!(
            r.url
                .query_pairs()
                .any(|(k, v)| k == "dsid" && v == "1234567890"),
            "with the account's dsid, as icloud.com sends it"
        );
    }
    assert_eq!(to(&reqs, CONSENT).len(), 0, "already approved: no prompt");
    assert!(client.data_access_missing().is_empty(), "every key is held");

    // Held: the next pass asks Apple nothing.
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Open);
    assert_eq!(asked(&server, STATE).await, 1);

    // And kept for the next start, sealed with the cookies and the session.
    drop(client);
    let base = server.uri();
    let again = Client::with(
        dir.path(),
        Endpoints::at(&base),
        Store::file(dir.path()).unwrap(),
    )
    .unwrap();
    assert!(
        again.data_access_missing().is_empty(),
        "the keys outlive a restart"
    );
}

/// The usual first run: not approved yet. The approval is put on the person's devices once,
/// waited for without asking again, and each key fetched once it is given — through the
/// device's "still uploading" answer.
#[tokio::test]
async fn an_approval_pending_then_given_is_followed_to_the_end() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let shape = Shape {
        drive: false,
        ..ADP
    };
    let mut client = signed_in(&server, dir.path(), shape, true).await;
    // Not approved for the first two looks, then approved.
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(false, 0))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(CONSENT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isDeviceConsentNotificationSent": true, "isWebAccessAllowed": true,
            "isDeviceConsentedForPCS": false, "isICDRSDisabled": true, "deviceConsentForPCSExpiry": 0
        })))
        .mount(&server)
        .await;
    // The device is still sending the Photos key the first time it is asked for.
    Mock::given(method("POST"))
        .and(path(PCS))
        .and(body_partial_json(json!({ "appName": "photos" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isWebAccessAllowed": true, "isDeviceConsentedForPCS": true, "isICDRSDisabled": true,
            "message": "Requested the device to upload cookies.", "status": "failure"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_keys(&server, 3600).await;

    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Waiting);
    assert_eq!(
        asked(&server, CONSENT).await,
        1,
        "the approval is put on the devices"
    );

    let landed = {
        let client = std::sync::Arc::new(tokio::sync::Mutex::new(client));
        let stepping = std::sync::Arc::clone(&client);
        let landed = follow(quick, || {
            let c = std::sync::Arc::clone(&stepping);
            async move { c.lock().await.data_access(false).await }
        })
        .await
        .unwrap();
        assert!(client.lock().await.data_access_missing().is_empty());
        landed
    };
    assert_eq!(landed, DataAccess::Open);

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        to(&reqs, CONSENT).len(),
        1,
        "waiting never puts a second prompt on the phone"
    );
    let photos: Vec<Value> = to(&reqs, PCS)
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap())
        .filter(|v| v["appName"] == "photos")
        .collect();
    assert_eq!(photos.len(), 2, "asked for until it arrives");
    assert_eq!(photos[0]["derivedFromUserAction"], json!(true));
    assert_eq!(
        photos[1]["derivedFromUserAction"],
        json!(false),
        "and only the first ask notifies again"
    );
    assert!(
        !app_names(&to(&reqs, PCS)).contains(&"iclouddrive".to_string()),
        "an account without iCloud Drive is never asked for its key"
    );
}

/// A background pass never puts a prompt on the person's phone: it says one is needed.
#[tokio::test]
async fn a_background_pass_says_an_approval_is_needed_without_asking() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(false, 0))
        .mount(&server)
        .await;
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Needed);
    assert_eq!(asked(&server, CONSENT).await, 0);
}

/// Never answered: the wait is bounded, and lands on "needed" — Try Again, not a spinner for
/// ever, and never a second prompt while it waits.
#[tokio::test]
async fn an_approval_never_given_runs_out_and_asks_for_try_again() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(false, 0))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(CONSENT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isDeviceConsentNotificationSent": true
        })))
        .mount(&server)
        .await;
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Waiting);
    let client = std::sync::Arc::new(tokio::sync::Mutex::new(client));
    let landed = follow(quick, || {
        let c = std::sync::Arc::clone(&client);
        async move { c.lock().await.data_access(false).await }
    })
    .await
    .unwrap();
    assert_eq!(landed, DataAccess::Needed);
    assert_eq!(
        asked(&server, STATE).await,
        9,
        "one look, then one per wait: bounded"
    );
    assert_eq!(asked(&server, CONSENT).await, 1);
    // The person presses Try Again straight away: the request already on their phone stands.
    assert_eq!(
        client.lock().await.data_access(true).await.unwrap(),
        DataAccess::Waiting
    );
    assert_eq!(
        asked(&server, CONSENT).await,
        1,
        "not pushed twice within two minutes"
    );
}

/// Apple could not reach any of the person's devices.
#[tokio::test]
async fn an_approval_apple_could_not_send_is_said() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(false, 0))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(CONSENT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isDeviceConsentNotificationSent": false, "isWebAccessAllowed": true
        })))
        .mount(&server)
        .await;
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::NotSent);
}

/// A key Apple refused even though one was held (it let it go early): asked for again, and
/// "Cookies already present." is taken for what it says.
#[tokio::test]
async fn a_refused_key_is_asked_for_again_and_already_present_is_fine() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    mount_keys(&server, 3600).await;
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    let before = asked(&server, PCS).await;

    server.reset().await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(PCS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isWebAccessAllowed": true, "isDeviceConsentedForPCS": true, "isICDRSDisabled": true,
            "message": "Cookies already present.", "status": "success"
        })))
        .mount(&server)
        .await;
    assert_eq!(before, 4);
    // Drive answered 423 "Missing PCS cookies" (quince_icloud::Error::NeedsDeviceApproval).
    client.data_access_lapsed("drive");
    assert_eq!(client.data_access_missing(), ["drive"]);
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        app_names(&to(&reqs, PCS)),
        ["iclouddrive"],
        "only the refused one"
    );
    assert!(client.data_access_missing().is_empty());
}

/// Keys run out (Apple's hour): asked for again by themselves while the approval holds; once
/// it has run out too, a background pass says an approval is needed.
#[tokio::test]
async fn keys_that_run_out_are_asked_for_again() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(false, 0))
        .mount(&server)
        .await;
    mount_keys(&server, 1).await;
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        client.data_access_missing(),
        ["notes", "reminders", "photos", "drive"],
        "every key ran out with its cookies"
    );
    // The approval still holds: attached again, silently.
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    assert_eq!(asked(&server, CONSENT).await, 0);
    assert_eq!(asked(&server, PCS).await, 8);
    // Then the approval runs out as well.
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Needed);
}

/// Apple busy or asking for fewer requests: "not now", typed, so the caller waits — never read
/// as a refusal, and never hammered.
#[tokio::test]
async fn throttling_and_outages_are_not_refusals() {
    for status in [429u16, 503] {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let mut client = signed_in(&server, dir.path(), ADP, false).await;
        Mock::given(method("POST"))
            .and(path(STATE))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let got = client.data_access(true).await;
        assert!(
            matches!(got, Err(Error::Unavailable(s)) if s == status),
            "{status}: {got:?}"
        );
        assert_eq!(
            asked(&server, STATE).await,
            1,
            "one request, no retry storm"
        );
    }
}

/// A session Apple wants signed back in first: typed as that, for the heartbeat to mend.
#[tokio::test]
async fn a_session_apple_will_not_release_keys_to_needs_a_sign_in() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(PCS))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "message": "Missing X-APPLE-WEBAUTH-TOKEN cookie", "status": "failure"
        })))
        .mount(&server)
        .await;
    assert!(matches!(
        client.data_access(false).await,
        Err(Error::ReauthRequired)
    ));

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(ResponseTemplate::new(421))
        .mount(&server)
        .await;
    assert!(matches!(
        client.data_access(true).await,
        Err(Error::ReauthRequired)
    ));
}

/// Accounts Apple keeps from the web in its own way — a child's, a Managed Apple Account — or
/// answers it does not explain: a typed error in words, never a crash, never Apple's JSON.
#[tokio::test]
async fn an_account_apple_refuses_is_classified_not_crashed_on() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": "ACCESS_DENIED", "reason": "managed account: data@example.invalid"
        })))
        .mount(&server)
        .await;
    match client.data_access(true).await {
        Err(Error::Protocol(words)) => {
            assert!(
                !words.contains("example.invalid"),
                "Apple's words stay out: {words}"
            );
        }
        other => panic!("{other:?}"),
    }

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(PCS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "Something new.", "status": "failure"
        })))
        .mount(&server)
        .await;
    assert!(matches!(
        client.data_access(false).await,
        Err(Error::Protocol(_))
    ));

    // Garbage, or nothing at all.
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .mount(&server)
        .await;
    assert!(
        matches!(client.data_access(true).await, Err(Error::Protocol(_))),
        "an account that said it has Advanced Data Protection is not read as having none"
    );
    assert_eq!(asked(&server, CONSENT).await, 0);
}

/// Apple's "User has not disabled iCloud DRS": recovery is on after all, nothing to approve.
#[tokio::test]
async fn recovery_on_after_all_needs_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(PCS))
        .respond_with(ResponseTemplate::new(412).set_body_json(json!({
            "message": "User has not disabled iCloud DRS", "status": "failure"
        })))
        .mount(&server)
        .await;
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    assert!(client.data_access_missing().is_empty());
}

/// Apple said the key was attached and set nothing — a wrong app name: said, not taken as held.
#[tokio::test]
async fn success_with_no_key_is_not_taken_for_one() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(PCS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "Cookies attached.", "status": "success"
        })))
        .mount(&server)
        .await;
    assert!(matches!(
        client.data_access(false).await,
        Err(Error::Protocol(_))
    ));
    assert!(client.data_access_missing().contains(&"photos"));
}

/// A refusal on an account that never said it had Advanced Data Protection: it does, and the
/// approval flow takes over.
#[tokio::test]
async fn a_refusal_on_an_account_that_never_said_turns_the_flow_on() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), STANDARD, false).await;
    assert!(client.data_access_missing().is_empty());
    client.data_access_lapsed("notes");
    assert_eq!(
        client.data_access_missing(),
        ["notes", "reminders", "photos", "drive"]
    );
    client.data_access_lapsed("calendar");
    assert!(
        !client.data_access_missing().contains(&"calendar"),
        "Calendar never needs one"
    );
}

/// Signing out forgets every key and approval.
#[tokio::test]
async fn signing_out_forgets_the_keys() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    mount_keys(&server, 3600).await;
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    client.sign_out().unwrap();
    assert!(client.session().pcs_until.is_empty());
    assert!(
        client.data_access_missing().is_empty(),
        "signed out: no services at all"
    );
}

/// What a background pass goes by: read from what is held here — no request to Apple, so no
/// prompt and no notice on anyone's phone, however often it runs.
#[tokio::test]
async fn where_access_stands_is_read_without_asking_apple() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let client = signed_in(&server, dir.path(), ADP, false).await;
    let before = server.received_requests().await.unwrap().len();
    for _ in 0..10 {
        assert_eq!(client.data_access_standing(), DataAccess::Needed);
    }
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        before,
        "nothing asked of Apple"
    );

    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let client = signed_in(&server, dir.path(), STANDARD, false).await;
    assert_eq!(
        client.data_access_standing(),
        DataAccess::Open,
        "no ADP: nothing waits"
    );
    let shape = Shape {
        web_access: Some(false),
        ..ADP
    };
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let client = signed_in(&server, dir.path(), shape, false).await;
    assert_eq!(client.data_access_standing(), DataAccess::WebAccessOff);
}

/// One approval, every protected service: a single consent request, then each service's key
/// asked for back to back in the same step.
#[tokio::test]
async fn one_approval_covers_every_protected_service_in_one_go() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(false, 0))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, in_an_hour_ms()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(CONSENT))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isDeviceConsentNotificationSent": true
        })))
        .mount(&server)
        .await;
    mount_keys(&server, 3600).await;
    assert_eq!(client.data_access(true).await.unwrap(), DataAccess::Waiting);
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(to(&reqs, CONSENT).len(), 1, "one approval");
    assert_eq!(
        app_names(&to(&reqs, PCS)),
        ["notes3", "reminders", "photos", "iclouddrive"],
        "every service, one after another, in the step after it"
    );
}

/// Keys are held only until the approval runs out, even when their cookies would last longer:
/// a pass stops asking for the service at that moment, never after Apple refuses it.
#[tokio::test]
async fn keys_are_let_go_when_the_approval_runs_out() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut client = signed_in(&server, dir.path(), ADP, false).await;
    let soon_ms = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        u64::try_from(now).unwrap() + 1500
    };
    Mock::given(method("POST"))
        .and(path(STATE))
        .respond_with(state(true, soon_ms))
        .mount(&server)
        .await;
    mount_keys(&server, 3600).await; // cookies good for an hour
    assert_eq!(client.data_access(false).await.unwrap(), DataAccess::Open);
    assert!(client.data_access_missing().is_empty());
    tokio::time::sleep(Duration::from_millis(2600)).await;
    assert_eq!(
        client.data_access_missing(),
        ["notes", "reminders", "photos", "drive"],
        "the approval's expiry, not the cookies', is when they go"
    );
    assert_eq!(client.data_access_standing(), DataAccess::Needed);
}
