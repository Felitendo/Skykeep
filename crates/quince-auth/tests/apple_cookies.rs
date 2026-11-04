//! Apple's own Set-Cookie headers, with every attribute it sends, through the same jar the
//! client uses (issue #5: every service answered "Missing X-APPLE-WEBAUTH-TOKEN cookie" /
//! "no auth method found"). Each cookie `accountLogin` sets on setup.icloud.com must be kept and
//! sent to every service host Apple's map names, and to nothing outside icloud.com.

use reqwest::cookie::CookieStore as _;
use reqwest::header::HeaderValue;
use reqwest_cookie_store::CookieStoreMutex;

fn url(s: &str) -> reqwest::Url {
    reqwest::Url::parse(s).unwrap()
}

/// What `accountLogin` sets, shaped as Apple sends it: quoted values with `=` and `:`, both
/// `Expires` date forms browsers see from Apple, `Max-Age`, `SameSite=None`, newer attributes
/// (`Partitioned`, `Priority`), a domain with and without its leading dot, an empty "delete"
/// tombstone, and a token of the size Apple's are.
fn account_login_cookies() -> Vec<String> {
    let token = format!("\"v=2:t={}\"", "A".repeat(3500));
    vec![
        format!("X-APPLE-WEBAUTH-TOKEN={token}; Path=/; Domain=.icloud.com; Secure; HttpOnly"),
        "X-APPLE-WEBAUTH-USER=\"v=1:s=0:d=12345678901\"; Expires=Sat, 05-Dec-2026 10:00:00 GMT; Path=/; Domain=.icloud.com; Secure; HttpOnly".into(),
        "X-APPLE-DS-WEB-SESSION-TOKEN=\"AQEAAAAAAAAAAA==\"; Path=/; Domain=.icloud.com; Secure; HttpOnly; SameSite=None".into(),
        "X-APPLE-WEBAUTH-VALIDATE=\"v=1:t=AQAAAAAAAAAA\"; Path=/; Domain=icloud.com; Secure".into(),
        "X-APPLE-WEBAUTH-HSA-TRUST=\"8a7c0e3f2b1d\"; Max-Age=2592000; Expires=Thu, 03 Dec 2026 10:00:00 GMT; Path=/; Domain=.icloud.com; Secure; HttpOnly; Priority=High".into(),
        "X-APPLE-UNIQUE-CLIENT-ID=\"Ab12\"; Path=/; Domain=.icloud.com; Secure; Partitioned; SameSite=None".into(),
        "X-APPLE-WEBAUTH-HSA-LOGIN=; Expires=Thu, 01-Jan-1970 00:00:01 GMT; Path=/; Domain=.icloud.com; Secure; HttpOnly".into(),
    ]
}

const KEPT: [&str; 6] = [
    "X-APPLE-WEBAUTH-TOKEN",
    "X-APPLE-WEBAUTH-USER",
    "X-APPLE-DS-WEB-SESSION-TOKEN",
    "X-APPLE-WEBAUTH-VALIDATE",
    "X-APPLE-WEBAUTH-HSA-TRUST",
    "X-APPLE-UNIQUE-CLIENT-ID",
];

fn names_sent(jar: &CookieStoreMutex, to: &str) -> Vec<String> {
    jar.cookies(&url(to))
        .map(|h| {
            h.to_str()
                .unwrap()
                .split("; ")
                .filter_map(|c| c.split('=').next())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn every_account_login_cookie_reaches_every_service_host() {
    let jar = CookieStoreMutex::default();
    let headers: Vec<HeaderValue> = account_login_cookies()
        .iter()
        .map(|c| HeaderValue::from_str(c).unwrap())
        .collect();
    jar.set_cookies(
        &mut headers.iter(),
        &url("https://setup.icloud.com/setup/ws/1/accountLogin?clientBuildNumber=1"),
    );
    for host in [
        "https://p123-contactsws.icloud.com/co/startup",
        "https://p45-ckdatabasews.icloud.com/database/1/com.apple.notes/production/private/changes/zone",
        "https://p45-calendarws.icloud.com/ca/startup",
        "https://p45-drivews.icloud.com/retrieveItemDetailsInFolders",
        "https://setup.icloud.com/setup/ws/1/validate",
    ] {
        let sent = names_sent(&jar, host);
        for name in KEPT {
            assert!(sent.iter().any(|n| n == name), "{name} not sent to {host}: {sent:?}");
        }
        assert!(
            !sent.iter().any(|n| n == "X-APPLE-WEBAUTH-HSA-LOGIN"),
            "a deleted cookie is not sent"
        );
    }
    assert!(
        names_sent(&jar, "https://example.com/").is_empty(),
        "never outside icloud.com"
    );
    assert!(
        names_sent(&jar, "http://p123-contactsws.icloud.com/co/startup").is_empty(),
        "never over plain HTTP"
    );
}

/// Several cookies in one header line (folded by a proxy) are not lost silently: each line Apple
/// sends is one cookie, and that is what this keeps.
#[test]
fn one_set_cookie_header_per_cookie_is_what_is_kept() {
    let jar = CookieStoreMutex::default();
    let headers = [
        HeaderValue::from_static(
            "X-APPLE-WEBAUTH-USER=\"v=1:s=0\"; Path=/; Domain=.icloud.com; Secure",
        ),
        HeaderValue::from_static(
            "X-APPLE-WEBAUTH-TOKEN=\"v=2:t=x\"; Path=/; Domain=.icloud.com; Secure",
        ),
    ];
    jar.set_cookies(
        &mut headers.iter(),
        &url("https://setup.icloud.com/setup/ws/1/accountLogin"),
    );
    let sent = names_sent(&jar, "https://p1-contactsws.icloud.com/co/startup");
    assert_eq!(sent.len(), 2, "{sent:?}");
}
