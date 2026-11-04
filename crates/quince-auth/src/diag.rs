//! Diagnostics the person turns on, for a problem that only shows on their account.
//!
//! Off unless they ask (the app turns it on with `SKYKEEP_LOG=1`; see its `diagnostics`). On,
//! it says what Skykeep asked Apple and what came back — and nothing of the person's:
//!
//! * where: the host, and the path with every id-like part (record ids, GUIDs, numbers) put
//!   as `…`; never the query, which carries the account's id and this install's;
//! * the status, and Apple's own error codes and reasons, each cut short, with anything that
//!   could be an address or an id taken out;
//! * which cookies went with a request, and which an answer set — by name, with their domain,
//!   path and whether they last — and whether the jar kept each one, and if not why. Never a
//!   value: those are the session itself.
//!
//! No password, token, name, address, note or record is ever written. The log stays on this
//! computer; Skykeep sends nothing anywhere.

use reqwest::header::HeaderValue;
use reqwest_cookie_store::CookieStoreMutex;
use std::sync::{Arc, OnceLock};

/// Where a line goes. Set once by the app when the person turned diagnostics on.
pub type Sink = fn(&str);
static SINK: OnceLock<Sink> = OnceLock::new();

/// Turn diagnostics on. Once per run; later calls change nothing.
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Whether diagnostics are on.
#[must_use]
pub fn on() -> bool {
    SINK.get().is_some()
}

/// Tell one line, when diagnostics are on.
pub fn note(line: &str) {
    if let Some(sink) = SINK.get() {
        sink(line);
    }
}

/// A part of a path that names something of the person's rather than an endpoint: long, or
/// mostly digits, or shaped like an id.
fn id_like(part: &str) -> bool {
    let digits = part.chars().filter(char::is_ascii_digit).count();
    part.len() > 24
        || (digits >= 4 && digits * 2 >= part.len())
        || part.contains("::")
        || part.contains('@')
        || part.contains('%')
}

/// Host and path, the person's parts put as `…`, no query.
#[must_use]
pub fn place(url: &reqwest::Url) -> String {
    let path: Vec<&str> = url
        .path()
        .split('/')
        .map(|p| if id_like(p) { "…" } else { p })
        .collect();
    format!("{}{}", url.host_str().unwrap_or("?"), path.join("/"))
}

/// Apple's error codes and reasons out of an answer, safe to write: only the fields Apple uses
/// for them, each cut to 100 characters, and nothing that could be an address or an id.
#[must_use]
pub fn codes(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return if body.trim().is_empty() {
            "(no body)".into()
        } else {
            "(not JSON)".into()
        };
    };
    let mut found = Vec::new();
    collect(&v, &mut found, 0);
    if found.is_empty() {
        "(no error codes)".into()
    } else {
        found.join(", ")
    }
}

const CODE_KEYS: [&str; 9] = [
    "serverErrorCode",
    "errorCode",
    "errorReason",
    "reason",
    "error",
    "code",
    "message",
    "status",
    "success",
];

fn collect(v: &serde_json::Value, out: &mut Vec<String>, depth: usize) {
    if depth > 4 || out.len() >= 12 {
        return;
    }
    match v {
        serde_json::Value::Object(map) => {
            for key in CODE_KEYS {
                if let Some(x) = map.get(key) {
                    let text = match x {
                        serde_json::Value::String(s) => clean(s),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    out.push(format!("{key}={text}"));
                }
            }
            for x in map.values() {
                if x.is_object() || x.is_array() {
                    collect(x, out, depth + 1);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for x in items.iter().take(4) {
                collect(x, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// A reason Apple gave, cut short, with any word that could be an address or an id taken out.
fn clean(s: &str) -> String {
    let words: Vec<&str> = s
        .split_whitespace()
        .map(|w| if id_like(w) { "…" } else { w })
        .collect();
    let mut text = words.join(" ");
    if text.chars().count() > 100 {
        text = text.chars().take(100).collect::<String>() + "…";
    }
    format!("{text:?}")
}

/// One answer, told: where it came from and its status.
pub fn answer(url: &reqwest::Url, status: reqwest::StatusCode) {
    if on() {
        note(&format!("← {} {}", status.as_u16(), place(url)));
    }
}

/// Facts about an account answer that explain a session that will not work, never its data.
pub fn account(what: &str, data: &serde_json::Value) {
    if !on() {
        return;
    }
    let flag = |k: &str| {
        data.get(k)
            .or_else(|| data.pointer(&format!("/dsInfo/{k}")))
            .and_then(serde_json::Value::as_bool)
            .map_or("absent".to_string(), |b| b.to_string())
    };
    let services: Vec<&str> = data
        .get("webservices")
        .and_then(serde_json::Value::as_object)
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    note(&format!(
        "{what}: termsUpdateNeeded={} isRepairNeeded={} hsaChallengeRequired={} hsaTrustedBrowser={} isWebAccessAllowed={} iCDPEnabled={} services={services:?}",
        flag("termsUpdateNeeded"),
        flag("isRepairNeeded"),
        flag("hsaChallengeRequired"),
        flag("hsaTrustedBrowser"),
        flag("isWebAccessAllowed"),
        flag("iCDPEnabled"),
    ));
}

/// The client's cookie jar, telling diagnostics which cookies go out and which come back — by
/// name only — and whether the jar kept each one it was handed.
pub struct Told(pub Arc<CookieStoreMutex>);

impl reqwest::cookie::CookieStore for Told {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &reqwest::Url) {
        if !on() {
            self.0.set_cookies(headers, url);
            return;
        }
        let headers: Vec<&HeaderValue> = headers.collect();
        let mut seen = Vec::new();
        for h in &headers {
            seen.push(describe(h, url));
        }
        note(&format!("  set by {}: {}", place(url), seen.join(" | ")));
        self.0.set_cookies(&mut headers.into_iter(), url);
    }

    fn cookies(&self, url: &reqwest::Url) -> Option<HeaderValue> {
        let sent = self.0.cookies(url);
        if on() {
            let names: Vec<String> = sent
                .as_ref()
                .and_then(|h| h.to_str().ok())
                .map(|s| {
                    s.split("; ")
                        .filter_map(|c| c.split('=').next())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            note(&format!("→ {} cookies {names:?}", place(url)));
        }
        sent
    }
}

/// One Set-Cookie, by name and attributes, and whether the jar would keep it — never its value.
fn describe(h: &HeaderValue, url: &reqwest::Url) -> String {
    let Ok(text) = h.to_str() else {
        return "(not text: dropped)".into();
    };
    let Ok(raw) = cookie_store::RawCookie::parse(text.to_string()) else {
        return "(unreadable: dropped)".into();
    };
    let lasts = if raw.max_age().is_some() || raw.expires().is_some_and(|e| e.is_datetime()) {
        "lasting"
    } else {
        "session"
    };
    let kept = match cookie_store::Cookie::try_from_raw_cookie(&raw, url) {
        Ok(c) if c.is_expired() => "deleted".to_string(),
        Ok(_) if raw.value().is_empty() => "empty".to_string(),
        Ok(_) => "kept".to_string(),
        Err(e) => format!("DROPPED: {e:?}"),
    };
    format!(
        "{} domain={} path={} {lasts} {kept}",
        raw.name(),
        raw.domain().unwrap_or("(host)"),
        raw.path().unwrap_or("(default)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_never_carry_the_query_or_an_id() {
        let u = reqwest::Url::parse(
            "https://p45-ckdatabasews.icloud.com/database/1/com.apple.notes/production/private/records/lookup?dsid=12345678901&clientId=auth-abc",
        )
        .unwrap();
        assert_eq!(
            place(&u),
            "p45-ckdatabasews.icloud.com/database/1/com.apple.notes/production/private/records/lookup"
        );
        let u = reqwest::Url::parse(
            "https://p45-calendarws.icloud.com/ca/events/home/4186A3B2-77C1-4E0A-9F00-5D0C1B2E3F44",
        )
        .unwrap();
        assert_eq!(place(&u), "p45-calendarws.icloud.com/ca/events/home/…");
    }

    #[test]
    fn only_apples_codes_are_written_never_its_data() {
        let body = r#"{ "uuid" : "f3321aa0-1111-2222-3333-444455556666", "serverErrorCode" : "AUTHENTICATION_FAILED", "reason" : "no auth method found" }"#;
        assert_eq!(
            codes(body),
            r#"serverErrorCode="AUTHENTICATION_FAILED", reason="no auth method found""#
        );
        assert_eq!(
            codes(r#"{"reason":"Missing X-APPLE-WEBAUTH-TOKEN cookie","error":1}"#),
            r#"reason="Missing X-APPLE-WEBAUTH-TOKEN cookie", error=1"#
        );
        // An address, an id, a name in a record: none of it.
        let personal = r#"{"reason":"no account for someone@example.com","records":[{"recordName":"ABCDEF123456","fields":{"title":{"value":"Mum's birthday"}}}]}"#;
        let said = codes(personal);
        assert!(
            !said.contains("example.com") && !said.contains("ABCDEF") && !said.contains("birthday"),
            "{said}"
        );
        assert_eq!(codes("<html>Bad Gateway</html>"), "(not JSON)");
    }

    #[test]
    fn a_cookie_is_described_by_name_never_by_value() {
        let u = reqwest::Url::parse("https://setup.icloud.com/setup/ws/1/accountLogin").unwrap();
        let h = HeaderValue::from_static(
            "X-APPLE-WEBAUTH-TOKEN=\"v=2:t=SECRETSECRET\"; Path=/; Domain=.icloud.com; Secure; HttpOnly",
        );
        let said = describe(&h, &u);
        assert!(
            said.starts_with("X-APPLE-WEBAUTH-TOKEN domain=icloud.com path=/ session kept"),
            "{said}"
        );
        assert!(!said.contains("SECRET"));
        let wrong = HeaderValue::from_static("X=1; Domain=apple.com; Path=/");
        assert!(describe(&wrong, &u).contains("DROPPED: DomainMismatch"));
    }
}
