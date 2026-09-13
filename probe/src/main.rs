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
            if self.ok {
                "\x1b[32mPASS\x1b[0m"
            } else {
                "\x1b[31mFAIL\x1b[0m"
            },
            self.label,
            if self.status == 0 {
                "—".into()
            } else {
                self.status.to_string()
            },
            self.detail
        )
    }
}

/// Describe a response without dumping anyone's personal data.
fn summarise(v: &Value) -> String {
    let mut parts = Vec::new();
    for key in [
        "Collections",
        "collections",
        "Reminders",
        "contacts",
        "Events",
        "events",
        "records",
        "zones",
        "items",
        "devices",
    ] {
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

async fn probe_post(client: &Client, label: &'static str, url: String, body: Value) -> Outcome {
    let session = client.session();
    let q: Vec<(String, String)> = vec![
        ("clientBuildNumber".into(), "2523Project28".into()),
        ("clientMasteringNumber".into(), "2523B28".into()),
        (
            "clientId".into(),
            session.client_id.clone().unwrap_or_default(),
        ),
        ("dsid".into(), session.dsid.clone().unwrap_or_default()),
    ];
    match client.http().post(&url).query(&q).json(&body).send().await {
        Err(e) => Outcome {
            label,
            ok: false,
            status: 0,
            detail: format!("transport: {e}"),
        },
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            let parsed: Option<Value> = serde_json::from_str(&text).ok();
            let detail = match (&parsed, status.is_success()) {
                // Drive answers with an array of folders; summarise the first.
                (Some(Value::Array(items)), true) => {
                    let children = items
                        .first()
                        .and_then(|f| f.get("items"))
                        .and_then(|i| i.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    format!("folders={}  items={}", items.len(), children)
                }
                (Some(v), true) => summarise(v),
                (_, true) => format!("{} bytes", text.len()),
                _ => text
                    .chars()
                    .take(160)
                    .collect::<String>()
                    .replace('\n', " "),
            };
            Outcome {
                label,
                ok: status.is_success(),
                status: status.as_u16(),
                detail,
            }
        }
    }
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
        (
            "clientId".into(),
            session.client_id.clone().unwrap_or_default(),
        ),
    ];
    if let Some(d) = &session.dsid {
        q.push(("dsid".into(), d.clone()));
    }
    for (k, v) in extra {
        q.push((k.to_string(), v.to_string()));
    }

    match client.http().get(&url).query(&q).send().await {
        Err(e) => Outcome {
            label,
            ok: false,
            status: 0,
            detail: format!("transport: {e}"),
        },
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            let parsed: Option<Value> = serde_json::from_str(&body).ok();
            let detail = match (&parsed, status.is_success()) {
                (Some(v), true) => summarise(v),
                (_, true) => format!("{} bytes, non-JSON", body.len()),
                // 450 means Apple wants a fresh sign-in for this service specifically.
                _ if status.as_u16() == 450 => {
                    "needs a fresh sign-in (a resumed session isn't accepted here)".to_string()
                }
                _ => body
                    .chars()
                    .take(160)
                    .collect::<String>()
                    .replace('\n', " "),
            };
            Outcome {
                label,
                ok: status.is_success(),
                status: status.as_u16(),
                detail,
            }
        }
    }
}

fn state_dir() -> PathBuf {
    std::env::var("QUINCE_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join(".local/share/quince-probe")
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    let fresh = std::env::args().any(|a| a == "--fresh");
    let code_only = std::env::args().any(|a| a == "--code");
    let dir = state_dir();
    let mut client = Client::new(&dir)?;

    println!("\n\x1b[1mQuince Phase 0 probe\x1b[0m");
    println!(
        "state: {} (secrets live in the system keyring)\n",
        dir.display()
    );

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
    // `--code` resumes an in-flight 2FA challenge using the session already on disk, so a
    // retry doesn't cost another sign-in.
    if !resumed && code_only {
        println!("Resuming the pending two-factor challenge (no new sign-in)…\n");
        let opts = client.two_factor_options().await?;
        println!(
            "  trusted devices Apple pushed to : {}",
            opts.trusted_device_count
        );
        for (i, ph) in opts.phones.iter().enumerate() {
            println!("    [{}] {}", i + 1, ph.masked_number);
        }
        loop {
            let code = prompt("  Code: ")?;
            let code = code.trim().to_string();
            if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
                println!("  that isn't a 6-digit code");
                continue;
            }
            match client.submit_two_factor(&code).await {
                Ok(()) => {
                    println!("  \x1b[32maccepted\x1b[0m");
                    break;
                }
                Err(e) => println!("  {e}"),
            }
        }
    } else if !resumed {
        let apple_id = match std::env::var("QUINCE_APPLE_ID") {
            Ok(v) if !v.is_empty() => v,
            _ => prompt("Apple ID: ")?,
        };
        let password = rpassword::prompt_password("Password (not echoed, never stored): ")?;

        println!("\nSigning in over SRP (your password is never sent to Apple)…");
        match client.sign_in(&apple_id, &password).await? {
            SignInOutcome::Complete => println!("  signed in; Apple already trusted this install"),
            SignInOutcome::NeedsTwoFactor => {
                let opts = client.two_factor_options().await?;
                println!();
                if opts.trusted_device_count > 0 {
                    println!(
                        "  Apple pushed a code to {} trusted device(s).",
                        opts.trusted_device_count
                    );
                } else {
                    println!(
                        "  \x1b[33mApple reports NO trusted devices, so no code was pushed.\x1b[0m"
                    );
                }
                if opts.too_many_codes_sent {
                    println!(
                        "  \x1b[33mApple is rate-limiting new codes. Wait a few minutes.\x1b[0m"
                    );
                }
                for (i, p) in opts.phones.iter().enumerate() {
                    println!("    [{}] text it to {}", i + 1, p.masked_number);
                }
                println!("\n  Nothing arrived? Two things to know:");
                println!("   • The code is only created once you tap Allow on the iPhone prompt.");
                println!("     No prompt means no code was ever generated — waiting won't help.");
                println!(
                    "   • iOS 18/26 hides \"Get Verification Code\" while the phone is ONLINE."
                );
                println!("     Turn on Airplane Mode, then Settings → your name → Sign-In &");
                println!("     Security. A popup offers the code while offline.");
                println!(
                    "\n  Type the 6-digit code, or `r` to push again, or `1`/`2`… to text it."
                );

                let mut sms_phone: Option<u64> = None;
                loop {
                    let answer = prompt("  Code: ")?;
                    let trimmed = answer.trim().to_string();

                    if trimmed.eq_ignore_ascii_case("r") {
                        client.resend_to_devices().await?;
                        println!("  pushed again — check your devices");
                        continue;
                    }
                    if let Ok(choice) = trimmed.parse::<usize>() {
                        // A 1-2 digit entry that indexes a phone means "text me"; a 6-digit
                        // entry is the code itself.
                        if trimmed.len() <= 2 && choice >= 1 && choice <= opts.phones.len() {
                            let phone = &opts.phones[choice - 1];
                            client.send_sms(phone.id).await?;
                            sms_phone = Some(phone.id);
                            println!(
                                "  texted {} — enter the code when it lands",
                                phone.masked_number
                            );
                            continue;
                        }
                    }
                    if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
                        println!("  that isn't a 6-digit code — try again");
                        continue;
                    }

                    let result = match sms_phone {
                        Some(id) => client.submit_sms_code(id, &trimmed).await,
                        None => client.submit_two_factor(&trimmed).await,
                    };
                    match result {
                        Ok(()) => {
                            println!(
                                "  \x1b[32mcode accepted, trust token stored in the keyring\x1b[0m"
                            );
                            break;
                        }
                        Err(e) => println!("  {e}"),
                    }
                }
            }
        }
    }

    // ---- 3. account facts -------------------------------------------------------
    let session = client.session();
    println!("\n\x1b[1m── Account ──\x1b[0m");
    println!(
        "  dsid          : {}",
        session.dsid.clone().unwrap_or("—".into())
    );
    println!("  trust token   : {}", redact(session.trust_token.as_ref()));
    println!(
        "  session token : {}",
        redact(session.session_token.as_ref())
    );

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
    let cal_range: &[(&str, &str)] = &[
        ("startDate", "2026-09-01"),
        ("endDate", "2026-09-30"),
        ("lang", "en-us"),
        ("usertz", "UTC"),
    ];

    if let Some(u) = session.service_url("calendar") {
        results.push(
            probe_get(
                &client,
                "Calendar  /ca/startup",
                format!("{u}/ca/startup"),
                cal_range,
            )
            .await,
        );
        results.push(
            probe_get(
                &client,
                "Calendar  /ca/events",
                format!("{u}/ca/events"),
                cal_range,
            )
            .await,
        );
    }
    if let Some(u) = session.service_url("contacts") {
        results.push(
            probe_get(
                &client,
                "Contacts  /co/startup",
                format!("{u}/co/startup"),
                &[
                    ("clientVersion", "2.1"),
                    ("locale", "en_US"),
                    ("order", "last,first"),
                ],
            )
            .await,
        );
    }
    if let Some(u) = session.service_url("reminders") {
        results.push(
            probe_get(
                &client,
                "Reminders /rd/startup",
                format!("{u}/rd/startup"),
                &[
                    ("clientVersion", "4.0"),
                    ("lang", "en-us"),
                    ("usertz", "UTC"),
                ],
            )
            .await,
        );
    }
    if let Some(u) = session.service_url("drivews") {
        results.push(
            probe_post(
                &client,
                "Drive     item details",
                format!("{u}/retrieveItemDetailsInFolders"),
                serde_json::json!([{
                    "drivewsid": "FOLDER::com.apple.CloudDocs::root",
                    "partialData": false,
                }]),
            )
            .await,
        );
    }
    if let Some(u) = session.service_url("findme") {
        results.push(
            probe_get(
                &client,
                "Find My   initClient",
                format!("{u}/fmipservice/client/web/initClient"),
                &[],
            )
            .await,
        );
    }
    // CloudKit zone lists double as the Advanced Data Protection tell: with ADP on,
    // these return nothing readable because Apple's servers hold no keys.
    if let Some(u) = session.service_url("ckdatabasews") {
        let ck: &[(&str, &str)] = &[
            ("ckjsBuildVersion", "17DProjectDev77"),
            ("ckjsVersion", "2.6.1"),
        ];
        results.push(
            probe_get(
                &client,
                "Photos    CloudKit zones",
                format!("{u}/database/1/com.apple.photos.cloud/production/private/zones/list"),
                ck,
            )
            .await,
        );
        results.push(
            probe_get(
                &client,
                "Notes     CloudKit zones",
                format!("{u}/database/1/com.apple.notes/production/private/zones/list"),
                ck,
            )
            .await,
        );
    }

    for r in &results {
        println!("  {}", r.line());
    }

    // ---- 5. the real sync path, end to end --------------------------------------
    // Exercises exactly what the app runs, so a failure here is the app's failure and the
    // output says why.
    println!("\n\x1b[1m── Calendar sync (the real path the app uses) ──\x1b[0m");
    if let Some(root) = session.service_url("calendar") {
        let zone = "Asia/Kolkata";
        let params = client.service_params();
        let http = client.http().clone();

        // Dump exactly what Apple returns, so parsing is checked against reality rather
        // than against an assumption about the shape.
        // Scan month by month with the exact parameters the client uses, so "no events"
        // is a measurement rather than an assumption.
        println!("  \x1b[1mmonth-by-month scan\x1b[0m");
        let months = [
            ("2026-01-01", "2026-01-31"),
            ("2026-03-01", "2026-03-31"),
            ("2026-05-01", "2026-05-31"),
            ("2026-07-01", "2026-07-31"),
            ("2026-08-01", "2026-08-31"),
            ("2026-09-01", "2026-09-30"),
            ("2026-10-01", "2026-10-31"),
            ("2026-12-01", "2026-12-31"),
            ("2027-01-01", "2027-01-31"),
        ];
        let scan_client =
            quince_icloud::calendar::CalendarClient::new(&http, &root, params.clone());
        let mut total = 0usize;
        for (a, b) in months {
            let via_startup = scan_client.startup(a, b, zone).await;
            let via_events = scan_client.events(a, b, zone).await;
            let s_count = via_startup.as_ref().map(|s| s.events.len()).unwrap_or(0);
            let e_count = via_events.as_ref().map(|s| s.events.len()).unwrap_or(0);
            total += s_count.max(e_count);
            println!(
                "    {a}  startup={s_count:<4} events={e_count:<4}{}",
                if via_startup.is_err() || via_events.is_err() {
                    "  (an endpoint errored)"
                } else {
                    ""
                }
            );
        }
        println!("  total events seen across the scan: {total}");

        let (window_start, window_end) = ("2026-06-01", "2027-03-31");

        match quince_core::Library::open(&dir) {
            Err(e) => println!("  \x1b[31mcouldn't open the local library:\x1b[0m {e}"),
            Ok(mut library) => {
                println!("  window {window_start} … {window_end} ({zone})");
                match quince_icloud::sync::calendar(
                    &http,
                    &root,
                    params,
                    &mut library,
                    window_start,
                    window_end,
                    zone,
                )
                .await
                {
                    Err(e) => println!("  \x1b[31mFAIL\x1b[0m {e}"),
                    Ok(summary) => {
                        println!(
                            "  \x1b[32mPASS\x1b[0m stored {} calendar(s), {} event(s)",
                            summary.calendars, summary.events
                        );
                        for c in library.calendars().unwrap_or_default() {
                            println!(
                                "    {:<26} {}",
                                c.title,
                                c.colour.unwrap_or_else(|| "—".into())
                            );
                        }
                        let events = library.events_between(0, i64::MAX).unwrap_or_default();
                        println!("  events held locally: {}", events.len());
                        for e in events.iter().take(8) {
                            println!("    {:>11}  {}", e.starts_at, e.title);
                        }
                        if events.is_empty() {
                            println!(
                                "    \x1b[33m(none in range — that may simply be an empty calendar)\x1b[0m"
                            );
                        }
                    }
                }
            }
        }
    } else {
        println!("  Apple offered no calendar service for this account");
    }

    // ---- Reminders from CloudKit, end to end ------------------------------------
    // The legacy /rd endpoints answer with Apple's placeholder on a migrated account, so
    // this reads the CloudKit zone where the real records live.
    println!("\n\x1b[1m── Reminders via CloudKit (the real records) ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let params = client.service_params();
        let http = client.http().clone();
        match quince_core::Library::open(&dir) {
            Err(e) => println!("  \x1b[31mcouldn't open the local library:\x1b[0m {e}"),
            Ok(mut library) => {
                match quince_icloud::reminders::sync_cloudkit(
                    &http,
                    &ck,
                    params,
                    &mut library,
                    None,
                )
                .await
                {
                    Err(e) => println!("  \x1b[31mFAIL\x1b[0m {e}"),
                    Ok((lists, count, token)) => {
                        println!(
                            "  \x1b[32mPASS\x1b[0m {lists} list(s), {count} reminder(s), resume token {}",
                            if token.is_some() { "stored" } else { "none" }
                        );
                        for l in library.reminder_lists().unwrap_or_default() {
                            let items = library.reminders_in(&l.guid, true).unwrap_or_default();
                            println!("    \x1b[1m{}\x1b[0m — {} item(s)", l.title, items.len());
                            for r in items.iter().take(15) {
                                let mark = if r.is_done() { "x" } else { " " };
                                let note = r
                                    .notes
                                    .as_deref()
                                    .map(|n| {
                                        format!("   — {}", n.chars().take(46).collect::<String>())
                                    })
                                    .unwrap_or_default();
                                println!("      [{mark}] {}{note}", r.title);
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- CloudKit containers ----------------------------------------------------
    // Apple's legacy /rd and /ca endpoints answer with a placeholder once data has been
    // upgraded to the modern format. The real records live in CloudKit, so find out which
    // containers this account actually exposes.
    println!("\n\x1b[1m── CloudKit containers ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let containers = [
            "com.apple.reminders",
            "com.apple.notes",
            "com.apple.photos.cloud",
            "com.apple.calendar",
            "com.apple.CloudDocs",
            "com.apple.shoebox",
        ];
        for container in containers {
            let url = format!("{ck}/database/1/{container}/production/private/zones/list");
            let mut q = client.service_params();
            q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
            q.push(("ckjsVersion".into(), "2.6.1".into()));
            match client
                .http()
                .post(&url)
                .query(&q)
                .json(&serde_json::json!({}))
                .send()
                .await
            {
                Err(e) => println!("  {container:<26} transport error: {e}"),
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    let parsed: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                    let zones = parsed.get("zones").and_then(|z| z.as_array());
                    match zones {
                        Some(list) if !list.is_empty() => {
                            let names: Vec<String> = list
                                .iter()
                                .filter_map(|z| {
                                    z.pointer("/zoneID/zoneName")
                                        .and_then(|n| n.as_str())
                                        .map(ToString::to_string)
                                })
                                .collect();
                            println!(
                                "  \x1b[32m{container:<26}\x1b[0m {} zone(s): {}",
                                list.len(),
                                names.join(", ")
                            );
                        }
                        _ => println!(
                            "  {container:<26} {status} {}",
                            text.chars().take(90).collect::<String>().replace('\n', " ")
                        ),
                    }
                }
            }
        }
    }

    // ---- what a CloudKit reminder actually looks like ---------------------------
    println!("\n\x1b[1m── CloudKit reminders: record shapes ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        // The Reminder type is not indexable, so records/query is refused. changes/zone
        // walks the whole zone instead, which is also how incremental sync works.
        // Omit desiredKeys entirely: an empty array asks Apple for no fields at all.
        let url = format!("{ck}/database/1/com.apple.reminders/production/private/changes/zone");
        let mut q = client.service_params();
        q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
        q.push(("ckjsVersion".into(), "2.6.1".into()));

        let mut sync_token: Option<String> = None;
        let mut types: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut samples: Vec<(String, Vec<String>, Option<String>)> = Vec::new();

        for page in 0..12 {
            let mut zone = serde_json::json!({
                "zoneID": { "zoneName": "Reminders", "zoneType": "REGULAR_CUSTOM_ZONE" },
                "maxChanges": 200,
            });
            if let Some(token) = &sync_token {
                zone["syncToken"] = serde_json::Value::String(token.clone());
            }
            let body = serde_json::json!({ "zones": [zone] });

            let Ok(r) = client.http().post(&url).query(&q).json(&body).send().await else {
                println!("  transport error on page {page}");
                break;
            };
            let text = r.text().await.unwrap_or_default();
            let v: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let Some(zone_result) = v
                .get("zones")
                .and_then(|z| z.as_array())
                .and_then(|a| a.first())
            else {
                println!(
                    "  unexpected: {}",
                    text.chars().take(200).collect::<String>()
                );
                break;
            };

            let records = zone_result
                .get("records")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            for rec in &records {
                let rt = rec
                    .get("recordType")
                    .and_then(|t| t.as_str())
                    .unwrap_or("?");
                *types.entry(rt.to_string()).or_default() += 1;
                if samples.iter().all(|(t, _, _)| t != rt) {
                    let fields: Vec<String> = rec
                        .get("fields")
                        .and_then(|f| f.as_object())
                        .map(|o| o.keys().cloned().collect())
                        .unwrap_or_default();
                    // Any field that looks like a title tells us where the text lives.
                    // Dump a whole Reminder so the shape of TitleDocument is visible.
                    let title = if rt == "Reminder" {
                        // Only the fields whose encoding is still unknown.
                        rec.get("fields").and_then(|f| {
                            let picked = serde_json::json!({
                                "TitleDocument": f.get("TitleDocument"),
                                "NotesDocument": f.get("NotesDocument"),
                                "Priority": f.get("Priority"),
                                "List": f.get("List"),
                            });
                            serde_json::to_string_pretty(&picked)
                                .ok()
                                .map(|s| s.chars().take(1600).collect::<String>())
                        })
                    } else {
                        None
                    };
                    samples.push((rt.to_string(), fields, title));
                }
            }

            sync_token = zone_result
                .get("syncToken")
                .and_then(|t| t.as_str())
                .map(ToString::to_string);
            if !zone_result
                .get("moreComing")
                .and_then(|m| m.as_bool())
                .unwrap_or(false)
            {
                break;
            }
        }

        let total: usize = types.values().sum();
        println!("  {total} record(s) across the zone");
        for (rt, n) in &types {
            println!("    {rt}: {n}");
        }
        for (rt, fields, title) in &samples {
            if let Some(dump) = title {
                println!("    --- full {rt} record ---\n{dump}");
            } else {
                println!("    {rt} fields: {}", fields.join(", "));
            }
        }
    }

    // ---- 5. verdict -------------------------------------------------------------
    let passed = results.iter().filter(|r| r.ok).count();
    println!("\n\x1b[1m── Verdict ──\x1b[0m");
    println!(
        "  services reachable from one sign-in : {passed}/{}",
        results.len()
    );
    println!(
        "  trust token stored                  : {}",
        session.trust_token.is_some()
    );
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
        md.push_str(&format!(
            "- `{}` — **{}** ({}) {}\n",
            r.label.trim(),
            if r.ok { "PASS" } else { "FAIL" },
            r.status,
            r.detail
        ));
    }
    std::fs::write(dir.join("findings.md"), md)?;
    println!("\n  findings: {}\n", dir.join("findings.md").display());
    Ok(())
}
