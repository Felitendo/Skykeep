//! Quince Phase 0 probe — exercises the real `quince-auth` crate against a live account.
//!
//! Answers the two questions Phase 1 depends on:
//!   1. Does a signed-in session survive a process restart? (the headline claim)
//!   2. Is every iCloud service reachable from that ONE sign-in, with no app-specific password?
//!
//! Never prints secrets; tokens are redacted and the password is never written to disk.

use anyhow::Result;
use quince_auth::{Client, SignInOutcome};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn redact(v: Option<&String>) -> String {
    match v {
        None => "—".into(),
        Some(s) if s.len() <= 8 => format!("<{} chars>", s.len()),
        Some(s) => format!("{}…{} <{} chars>", &s[..4], &s[s.len() - 4..], s.len()),
    }
}

struct Outcome {
    label: &'static str,
    ok: bool,
    status: u16,
    detail: String,
}

impl Outcome {
    fn line(&self) -> String {
        format!(
            "{} {:<30} {:>4}  {}",
            if self.ok { "\x1b[32mPASS\x1b[0m" } else { "\x1b[31mFAIL\x1b[0m" },
            self.label,
            if self.status == 0 { "—".into() } else { self.status.to_string() },
            self.detail
        )
    }
}

/// Describe a response without dumping anyone's personal data.
fn summarise(v: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["Collections", "collections", "Reminders", "contacts", "Events", "events",
                "records", "zones", "items", "devices"] {
        if let Some(a) = v.get(key).and_then(|x| x.as_array()) {
            parts.push(format!("{key}={}", a.len()));
        }
    }
    if parts.is_empty() {
        if let Some(o) = v.as_object() {
            let mut keys: Vec<&str> = o.keys().map(String::as_str).take(6).collect();
            keys.sort_unstable();
            parts.push(format!("keys: {}", keys.join(", ")));
        }
    }
    parts.join("  ")
}

async fn probe_get(
    client: &Client,
    label: &'static str,
    url: String,
    extra: &[(&str, &str)],
) -> Outcome {
    let session = client.session();
    let mut q: Vec<(String, String)> = vec![
        ("clientBuildNumber".into(), "2523Project28".into()),
        ("clientMasteringNumber".into(), "2523B28".into()),
        ("clientId".into(), session.client_id.clone().unwrap_or_default()),
    ];
    if let Some(d) = &session.dsid {
        q.push(("dsid".into(), d.clone()));
    }
    for (k, v) in extra {
        q.push((k.to_string(), v.to_string()));
    }

    match client.http().get(&url).query(&q).send().await {
        Err(e) => Outcome { label, ok: false, status: 0, detail: format!("transport: {e}") },
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            let parsed: Option<Value> = serde_json::from_str(&body).ok();
            let detail = match (&parsed, status.is_success()) {
                (Some(v), true) => summarise(v),
                (_, true) => format!("{} bytes, non-JSON", body.len()),
                _ => body.chars().take(160).collect::<String>().replace('\n', " "),
            };
            Outcome { label, ok: status.is_success(), status: status.as_u16(), detail }
        }
    }
}

fn state_dir() -> PathBuf {
    std::env::var("QUINCE_STATE").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".local/share/quince-probe")
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let fresh = std::env::args().any(|a| a == "--fresh");
    let dir = state_dir();
    let mut client = Client::new(&dir)?;

    println!("\n\x1b[1mQuince Phase 0 probe\x1b[0m");
    println!("state: {} (secrets live in the system keyring)\n", dir.display());

    if fresh {
        client.sign_out().ok();
        println!("--fresh: cleared the stored session.\n");
    }

    // ---- 1. the restart test ----------------------------------------------------
    let mut resumed = false;
    if client.has_stored_session() {
        println!("Found a stored session. Testing whether it still works…");
        if client.resume().await? {
            println!("  \x1b[32mRESUMED — no password, no 2FA.\x1b[0m\n");
            resumed = true;
        } else {
            println!("  Apple rejected it; signing in from scratch.\n");
        }
    }

    // ---- 2. full sign-in --------------------------------------------------------
    if !resumed {
        let apple_id = match std::env::var("QUINCE_APPLE_ID") {
            Ok(v) if !v.is_empty() => v,
            _ => prompt("Apple ID: ")?,
        };
        let password = rpassword::prompt_password("Password (not echoed, never stored): ")?;

        println!("\nSigning in over SRP (your password is never sent to Apple)…");
        match client.sign_in(&apple_id, &password).await? {
            SignInOutcome::Complete => println!("  signed in; Apple already trusted this install"),
            SignInOutcome::NeedsTwoFactor => {
                println!("  Apple sent a six-digit code to your devices.");
                let code = prompt("  Code: ")?;
                client.submit_two_factor(&code).await?;
                println!("  \x1b[32mcode accepted, trust token stored in the keyring\x1b[0m");
            }
        }
    }

    // ---- 3. account facts -------------------------------------------------------
    let session = client.session();
    println!("\n\x1b[1m── Account ──\x1b[0m");
    println!("  dsid          : {}", session.dsid.clone().unwrap_or("—".into()));
    println!("  trust token   : {}", redact(session.trust_token.as_ref()));
    println!("  session token : {}", redact(session.session_token.as_ref()));

    println!("\n\x1b[1m── Services Apple advertises for this account ──\x1b[0m");
    if let Some(ws) = session.webservices.as_ref().and_then(|w| w.as_object()) {
        let mut keys: Vec<&String> = ws.keys().collect();
        keys.sort();
        for k in keys {
            let status = ws[k].get("status").and_then(|s| s.as_str()).unwrap_or("—");
            println!("  {k:<22} {status}");
        }
    } else {
        println!("  (none returned)");
    }

    // ---- 4. one session, every service ------------------------------------------
    println!("\n\x1b[1m── Reachability: one sign-in, every service ──\x1b[0m");
    let mut results = Vec::new();
    let cal_range: &[(&str, &str)] =
        &[("startDate", "2026-09-01"), ("endDate", "2026-09-30"), ("lang", "en-us"), ("usertz", "UTC")];

    if let Some(u) = session.service_url("calendar") {
        results.push(probe_get(&client, "Calendar  /ca/startup", format!("{u}/ca/startup"), cal_range).await);
        results.push(probe_get(&client, "Calendar  /ca/events", format!("{u}/ca/events"), cal_range).await);
    }
    if let Some(u) = session.service_url("contacts") {
        results.push(probe_get(&client, "Contacts  /co/startup", format!("{u}/co/startup"),
            &[("clientVersion", "2.1"), ("locale", "en_US"), ("order", "last,first")]).await);
    }
    if let Some(u) = session.service_url("reminders") {
        results.push(probe_get(&client, "Reminders /rd/startup", format!("{u}/rd/startup"),
            &[("clientVersion", "4.0"), ("lang", "en-us"), ("usertz", "UTC")]).await);
    }
    if let Some(u) = session.service_url("drivews") {
        results.push(probe_get(&client, "Drive     item details",
            format!("{u}/retrieveItemDetailsInFolders"), &[]).await);
    }
    if let Some(u) = session.service_url("findme") {
        results.push(probe_get(&client, "Find My   initClient",
            format!("{u}/fmipservice/client/web/initClient"), &[]).await);
    }
    // CloudKit zone lists double as the Advanced Data Protection tell: with ADP on,
    // these return nothing readable because Apple's servers hold no keys.
    if let Some(u) = session.service_url("ckdatabasews") {
        let ck: &[(&str, &str)] = &[("ckjsBuildVersion", "17DProjectDev77"), ("ckjsVersion", "2.6.1")];
        results.push(probe_get(&client, "Photos    CloudKit zones",
            format!("{u}/database/1/com.apple.photos.cloud/production/private/zones/list"), ck).await);
        results.push(probe_get(&client, "Notes     CloudKit zones",
            format!("{u}/database/1/com.apple.notes/production/private/zones/list"), ck).await);
    }

    for r in &results {
        println!("  {}", r.line());
    }

    // ---- 5. verdict -------------------------------------------------------------
    let passed = results.iter().filter(|r| r.ok).count();
    println!("\n\x1b[1m── Verdict ──\x1b[0m");
    println!("  services reachable from one sign-in : {passed}/{}", results.len());
    println!("  trust token stored                  : {}", session.trust_token.is_some());
    println!("  resumed with no password or 2FA     : {resumed}");
    if !resumed {
        println!(
            "\n  \x1b[33mRun this again. If it prints RESUMED, the headline claim holds\n  \
             and Phase 1 is unblocked.\x1b[0m"
        );
    }

    let mut md = String::from("# Quince Phase 0 — findings\n\n");
    md.push_str(&format!(
        "- trust token acquired: {}\n- resumed without re-auth: {resumed}\n- services reachable: {passed}/{}\n\n## Reachability\n\n",
        session.trust_token.is_some(), results.len()
    ));
    for r in &results {
        md.push_str(&format!("- `{}` — **{}** ({}) {}\n",
            r.label.trim(), if r.ok { "PASS" } else { "FAIL" }, r.status, r.detail));
    }
    std::fs::write(dir.join("findings.md"), md)?;
    println!("\n  findings: {}\n", dir.join("findings.md").display());
    Ok(())
}
