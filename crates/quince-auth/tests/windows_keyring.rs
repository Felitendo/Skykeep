//! On Windows, against the real Credential Manager: a signed-in session is larger than one
//! credential can hold (2,560 bytes), and it must still be kept, read back whole, and erased
//! by signing out. This is the failure that stopped sign-in on Windows outright.
#![cfg(windows)]

use quince_auth::session::Session;
use quince_auth::Store;

#[test]
fn a_full_session_survives_windows_credential_manager() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::keyring(dir.path()).unwrap();
    let session = Session {
        client_id: Some("auth-00000000-0000-0000-0000-000000000000".into()),
        session_token: Some("T".repeat(1104)),
        trust_token: Some("R".repeat(200)),
        session_id: Some("S".repeat(80)),
        scnt: Some("C".repeat(300)),
        dsid: Some("12345".into()),
        webservices: Some(serde_json::json!({
            "ckdatabasews": { "url": "https://p177-ckdatabasews.icloud.com:443", "status": "active" },
            "calendar": { "url": "https://p177-calendarws.icloud.com:443", "status": "active" },
            "contacts": { "url": "https://p177-contactsws.icloud.com:443", "status": "active" },
            "drivews": { "url": "https://p177-drivews.icloud.com:443", "status": "active" },
            "docws": { "url": "https://p177-docws.icloud.com:443", "status": "active" },
        })),
        ..Session::default()
    };
    let size = serde_json::to_string(&session).unwrap().len();
    assert!(
        size > 1280,
        "the test session must be past one credential's limit ({size})"
    );
    store.save(&session).expect("kept, in parts");
    let back = store.try_load().expect("read back");
    assert_eq!(back.session_token, session.session_token);
    assert_eq!(back.trust_token, session.trust_token);
    assert_eq!(back.webservices, session.webservices);
    store.clear().expect("signed out");
    let gone = store.try_load().expect("read after sign-out");
    assert!(gone.session_token.is_none() && gone.trust_token.is_none());
}
