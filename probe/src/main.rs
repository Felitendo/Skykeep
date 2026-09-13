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
                // Resume from the stored token, so this measures what a real refresh costs
                // rather than repeating a cold walk every time.
                let since = library.sync_token("reminders").ok().flatten();
                let started = std::time::Instant::now();
                println!(
                    "  {} sync…",
                    if since.is_some() {
                        "incremental"
                    } else {
                        "first (full)"
                    }
                );
                let outcome = quince_icloud::reminders::sync_cloudkit(
                    &http,
                    &ck,
                    params,
                    &mut library,
                    since.as_deref(),
                )
                .await;
                println!("  took {:.1}s", started.elapsed().as_secs_f64());
                match outcome {
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

    // ---- what Notes records look like -------------------------------------------
    println!("\n\x1b[1m── Notes: record census and shapes ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        for zone_name in ["Notes", "Metadata"] {
            let zc = quince_icloud::cloudkit::ZoneClient::new(
                client.http(),
                &ck,
                "com.apple.notes",
                client.service_params(),
            );
            match zc.all_records(zone_name, None).await {
                Err(e) => println!(
                    "  {zone_name:<12} error: {}",
                    e.to_string().chars().take(70).collect::<String>()
                ),
                Ok((records, _)) => {
                    let mut census: std::collections::BTreeMap<String, usize> =
                        std::collections::BTreeMap::new();
                    for r in &records {
                        let k = r
                            .get("recordType")
                            .and_then(|t| t.as_str())
                            .unwrap_or("(tombstone)");
                        *census.entry(k.to_string()).or_default() += 1;
                    }
                    let summary: Vec<String> =
                        census.iter().map(|(k, n)| format!("{k}={n}")).collect();
                    println!(
                        "  {zone_name:<12} {} record(s): {}",
                        records.len(),
                        summary.join(" ")
                    );

                    // Does the reminder decoder handle note text too, or is the shape different?
                    for r in records
                        .iter()
                        .filter(|r| {
                            matches!(
                                r.get("recordType").and_then(|t| t.as_str()),
                                Some("Note" | "Folder")
                            )
                        })
                        .take(4)
                    {
                        let kind = r.get("recordType").and_then(|t| t.as_str()).unwrap_or("");
                        let decode = |name: &str| {
                            quince_icloud::cloudkit::field::document_text(r, name)
                                .map(|t| {
                                    t.chars().take(64).collect::<String>().replace('\n', " / ")
                                })
                                .unwrap_or_else(|| "<did not decode>".into())
                        };
                        println!("    {kind}: title={:?}", decode("TitleEncrypted"));
                        if kind == "Note" {
                            println!("      text={:?}", decode("TextDataEncrypted"));
                        }
                        // Emit a small body so its attribute runs can be read directly.
                        if kind == "Note" {
                            if let Some(raw) =
                                quince_icloud::cloudkit::field::string(r, "TextDataEncrypted")
                            {
                                if raw.len() < 1200 {
                                    println!("      BODY_B64 {raw}");
                                }
                            }
                        }
                    }

                    // One sample per type, so field names come from reality.
                    let mut seen: std::collections::BTreeSet<String> =
                        std::collections::BTreeSet::new();
                    for r in &records {
                        let Some(kind) = r.get("recordType").and_then(|t| t.as_str()) else {
                            continue;
                        };
                        if !seen.insert(kind.to_string()) {
                            continue;
                        }
                        let fields: Vec<String> = r
                            .get("fields")
                            .and_then(|f| f.as_object())
                            .map(|o| {
                                o.iter()
                                    .map(|(k, v)| {
                                        format!(
                                            "{k}:{}",
                                            v.get("type").and_then(|t| t.as_str()).unwrap_or("?")
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        println!("    {kind}: {}", fields.join(", "));
                    }
                }
            }
        }
    }

    // ---- what a contact record looks like ---------------------------------------
    println!("\n\x1b[1m── Contacts: shape ──\x1b[0m");
    if let Some(root) = session.service_url("contacts") {
        let mut q = client.service_params();
        q.push(("clientVersion".into(), "2.1".into()));
        q.push(("locale".into(), "en_US".into()));
        q.push(("order".into(), "last,first".into()));
        if let Ok(r) = client
            .http()
            .get(format!("{}/co/startup", root.trim_end_matches('/')))
            .query(&q)
            .send()
            .await
        {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            let v: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let contacts = v.get("contacts").and_then(|c| c.as_array());
            println!("  {status}: {} contact(s)", contacts.map_or(0, Vec::len));
            if let Some(first) = contacts.and_then(|c| c.first()) {
                if let Some(obj) = first.as_object() {
                    let mut keys: Vec<String> = obj
                        .iter()
                        .map(|(k, val)| {
                            let kind = match val {
                                serde_json::Value::Array(a) => format!("array[{}]", a.len()),
                                serde_json::Value::String(_) => "string".into(),
                                serde_json::Value::Bool(_) => "bool".into(),
                                serde_json::Value::Number(_) => "number".into(),
                                serde_json::Value::Object(_) => "object".into(),
                                serde_json::Value::Null => "null".into(),
                            };
                            format!("{k}:{kind}")
                        })
                        .collect();
                    keys.sort();
                    println!("    fields: {}", keys.join(", "));
                }
                // Show the shape of the nested arrays, which is where phones and emails live.
                for key in ["phones", "emailAddresses", "urls", "streetAddresses"] {
                    if let Some(entry) = first
                        .get(key)
                        .and_then(|a| a.as_array())
                        .and_then(|a| a.first())
                    {
                        println!(
                            "    {key}[0]: {}",
                            serde_json::to_string(entry).unwrap_or_default()
                        );
                    }
                }
            }
        }
    }

    // ---- Photos: which query actually returns assets? ---------------------------
    // changes/zone returns only CPLLibraryInfo, so assets must come from records/query —
    // which for photos is allowed, unlike the reminders types.
    println!("\n\x1b[1m── Photos: finding the assets ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let url =
            format!("{ck}/database/1/com.apple.photos.cloud/production/private/records/query");
        let mut q = client.service_params();
        q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
        q.push(("ckjsVersion".into(), "2.6.1".into()));
        q.push(("remapEnums".into(), "True".into()));

        // The smart-album queries Apple's own web client uses, plus the plain record types.
        let attempts: [(&str, serde_json::Value); 4] = [
            (
                "index count",
                serde_json::json!({
                    "query": {
                        "recordType": "HyperionIndexCountLookup",
                        "filterBy": [{
                            "fieldName": "indexCountID",
                            "comparator": "IN",
                            "fieldValue": { "type": "STRING_LIST",
                                            "value": ["CPLAssetByAssetDateWithoutHiddenOrDeleted"] }
                        }]
                    },
                    "zoneID": { "zoneName": "PrimarySync" },
                }),
            ),
            (
                "assets by date",
                serde_json::json!({
                    "query": {
                        "recordType": "CPLAssetAndMasterByAssetDateWithoutHiddenOrDeleted",
                        "filterBy": [
                            { "fieldName": "startRank", "comparator": "EQUALS",
                              "fieldValue": { "type": "INT64", "value": 0 } },
                            { "fieldName": "direction", "comparator": "EQUALS",
                              "fieldValue": { "type": "STRING", "value": "ASCENDING" } }
                        ]
                    },
                    "resultsLimit": 4,
                    "zoneID": { "zoneName": "PrimarySync" },
                }),
            ),
            (
                "CPLAsset",
                serde_json::json!({
                    "query": { "recordType": "CPLAsset" },
                    "resultsLimit": 3,
                    "zoneID": { "zoneName": "PrimarySync" },
                }),
            ),
            (
                "CPLMaster",
                serde_json::json!({
                    "query": { "recordType": "CPLMaster" },
                    "resultsLimit": 3,
                    "zoneID": { "zoneName": "PrimarySync" },
                }),
            ),
        ];

        for (label, body) in attempts {
            match client.http().post(&url).query(&q).json(&body).send().await {
                Err(e) => println!("  {label:<18} transport: {e}"),
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    let v: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                    match v.get("records").and_then(|r| r.as_array()) {
                        Some(records) => {
                            println!(
                                "  \x1b[32m{label:<18}\x1b[0m {status}  {} record(s)",
                                records.len()
                            );
                            for rec in records.iter().take(2) {
                                let rt = rec
                                    .get("recordType")
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("?");
                                // An index count carries the library size, which is the
                                // difference between "no photos" and "wrong query".
                                if let Some(count) = rec.pointer("/fields/itemCount/value") {
                                    println!("      {rt}: itemCount = {count}");
                                    continue;
                                }
                                let mut fields: Vec<String> = rec
                                    .get("fields")
                                    .and_then(|f| f.as_object())
                                    .map(|o| o.keys().cloned().collect())
                                    .unwrap_or_default();
                                fields.sort();
                                println!(
                                    "      {rt}: {}",
                                    fields.join(", ").chars().take(300).collect::<String>()
                                );
                            }
                        }
                        None => println!(
                            "  {label:<18} {status}  {}",
                            text.chars()
                                .take(120)
                                .collect::<String>()
                                .replace('\n', " ")
                        ),
                    }
                }
            }
        }
    }

    // ---- what Drive items look like ---------------------------------------------
    println!("\n\x1b[1m── iCloud Drive: shape ──\x1b[0m");
    if let Some(root) = session.service_url("drivews") {
        let mut q = client.service_params();
        let body = serde_json::json!([{
            "drivewsid": "FOLDER::com.apple.CloudDocs::root",
            "partialData": false,
        }]);
        q.push(("clientVersion".into(), "2.1".into()));
        match client
            .http()
            .post(format!(
                "{}/retrieveItemDetailsInFolders",
                root.trim_end_matches('/')
            ))
            .query(&q)
            .json(&body)
            .send()
            .await
        {
            Err(e) => println!("  transport: {e}"),
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                let v: serde_json::Value =
                    serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                let folders = v.as_array().map(Vec::as_slice).unwrap_or(&[]);
                println!("  {status}: {} folder(s)", folders.len());
                for folder in folders.iter().take(1) {
                    let items = folder
                        .get("items")
                        .and_then(|i| i.as_array())
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    println!("    items: {}", items.len());
                    for item in items.iter().take(4) {
                        let mut keys: Vec<String> = item
                            .as_object()
                            .map(|o| o.keys().cloned().collect())
                            .unwrap_or_default();
                        keys.sort();
                        let kind = item.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let ext = item.get("extension").and_then(|e| e.as_str()).unwrap_or("");
                        let size = item
                            .get("size")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(-1);
                        println!(
                            "      [{kind}] {name}{}{ext}  size={size}",
                            if ext.is_empty() { "" } else { "." }
                        );
                        println!("        fields: {}", keys.join(", "));
                    }
                }
            }
        }
    }

    // ---- ticking a repeating reminder -------------------------------------------
    // A repeating reminder is never "done" — ticking it rolls the due date to the next
    // occurrence. This prints, for every repeating reminder on the account, exactly what
    // the app would write if you ticked it now. Read-only: nothing is sent to Apple.
    println!("\n\x1b[1m── Repeating reminders: what ticking would write ──\x1b[0m");
    match quince_core::Library::open(&dir) {
        Err(e) => println!("  \x1b[31mcouldn't open the local library:\x1b[0m {e}"),
        Ok(library) => {
            let day = |secs: i64| -> String {
                jiff::Timestamp::from_second(secs)
                    .map(|t| {
                        t.to_zoned(jiff::tz::TimeZone::system())
                            .strftime("%a %d %b %Y %H:%M")
                            .to_string()
                    })
                    .unwrap_or_else(|_| secs.to_string())
            };
            let mut seen = 0usize;
            for list in library.reminder_lists().unwrap_or_default() {
                for r in library.reminders_in(&list.guid, true).unwrap_or_default() {
                    if !r.repeats() {
                        continue;
                    }
                    seen += 1;
                    let freq = match r.recur_frequency {
                        Some(0) => "daily",
                        Some(1) => "weekly",
                        Some(2) => "monthly",
                        Some(3) => "yearly",
                        _ => "?",
                    };
                    println!(
                        "  \x1b[1m{}\x1b[0m  ({list}, every {n} {freq})",
                        r.title,
                        list = list.title,
                        n = r.recur_interval.unwrap_or(1),
                    );
                    println!("    due now  {}", r.due_at.map_or("none".into(), day));
                    match r.next_occurrence() {
                        Some(next) => println!("    tick →   {}", day(next)),
                        None => {
                            println!("    tick →   \x1b[33mno next occurrence computable\x1b[0m")
                        }
                    }
                }
            }
            if seen == 0 {
                println!("  (no repeating reminders on this account)");
            }
        }
    }

    // ---- can a note be saved without damaging it? -------------------------------
    // Editing a note means rewriting Apple's document while keeping the version vectors,
    // folder links and merge state around the text. This checks, for every real note on the
    // account, whether putting the text back exactly as it came out reproduces the original
    // bytes. Where it does, a save is safe. Read-only — nothing is sent to Apple.
    println!("\n\x1b[1m── Notes: is every note safe to edit? ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.notes",
            client.service_params(),
        );
        if let Ok((records, _)) = zc.all_records("Notes", None).await {
            let (mut safe, mut refused) = (0, 0);
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("Note"))
            {
                let Some(encoded) = rec
                    .pointer("/fields/TextDataEncrypted/value")
                    .and_then(|v| v.as_str())
                else {
                    continue;
                };
                let title = quince_icloud::cloudkit::field::document_text(rec, "TitleEncrypted")
                    .unwrap_or_else(|| "(untitled)".into());
                let title: String = title.chars().take(44).collect();
                let Some(raw) = quince_icloud::ckdocument::debug_inflate(encoded) else {
                    println!("  \x1b[33mSKIP\x1b[0m {title:<46} could not decompress");
                    refused += 1;
                    continue;
                };
                let text = quince_icloud::ckdocument::text_from_bytes(&raw).unwrap_or_default();
                match quince_icloud::ckdocument::replace_text(&raw, &text) {
                    Some(rebuilt) if rebuilt == raw => {
                        println!(
                            "  \x1b[32mSAFE\x1b[0m {title:<46} {} byte(s) reproduced exactly",
                            raw.len()
                        );
                        safe += 1;
                    }
                    _ => {
                        println!("  \x1b[33mREAD-ONLY\x1b[0m {title:<41} shape not reproducible");
                        refused += 1;
                    }
                }
            }
            println!("\n  {safe} note(s) editable, {refused} left read-only");
        }
    }

    // ---- does writing an event actually work? -----------------------------------
    // Apple's calendar write protocol is not documented, so this proves the shape rather
    // than assuming it: create one obviously-named event, read it back, delete it. Runs
    // only with --write-test, and cleans up after itself.
    if std::env::args().any(|a| a == "--write-test") {
        println!("\n\x1b[1m── Calendar write round trip ──\x1b[0m");
        if let Some(root) = session.service_url("calendar") {
            match quince_core::Library::open(&dir) {
                Err(e) => println!("  \x1b[31mcouldn't open the local library:\x1b[0m {e}"),
                Ok(mut library) => {
                    let calendars = library.calendars().unwrap_or_default();
                    match calendars.first() {
                        None => println!("  no calendar to write to"),
                        Some(calendar) => {
                            // Tomorrow, so the test event never lands in a window
                            // someone is looking at right now.
                            let starts = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
                                + 86_400;
                            let event = quince_core::calendar::Event {
                                guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
                                calendar_guid: calendar.guid.clone(),
                                title: "Quince write test — safe to delete".into(),
                                location: None,
                                starts_at: starts,
                                ends_at: starts + 1_800,
                                all_day: false,
                                timezone: Some("Asia/Kolkata".into()),
                                url: None,
                                notes: None,
                                etag: None,
                            };
                            println!("  writing to \x1b[1m{}\x1b[0m…", calendar.title);
                            let params = client.service_params();
                            let http = client.http().clone();

                            match quince_icloud::sync::put_event(
                                &http,
                                &root,
                                params.clone(),
                                &mut library,
                                &event,
                            )
                            .await
                            {
                                Err(e) => println!("  \x1b[31mFAIL\x1b[0m create: {e}"),
                                Ok(()) => {
                                    println!("  \x1b[32mPASS\x1b[0m Apple accepted the create");

                                    // The real proof: ask Apple for that day afresh and see
                                    // whether the event is in what comes back. A write that
                                    // returns 200 and stores nothing would pass otherwise.
                                    let day = jiff::Timestamp::from_second(starts)
                                        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
                                        .to_zoned(jiff::tz::TimeZone::get("Asia/Kolkata").unwrap())
                                        .strftime("%Y-%m-%d")
                                        .to_string();
                                    let found = quince_icloud::sync::fetch_calendar(
                                        &http,
                                        &root,
                                        params.clone(),
                                        &day,
                                        &day,
                                        "Asia/Kolkata",
                                    )
                                    .await
                                    .map(|snap| {
                                        snap.events.into_iter().find(|e| e.guid == event.guid)
                                    })
                                    .unwrap_or(None);

                                    match &found {
                                        Some(back) => println!(
                                            "  \x1b[32mPASS\x1b[0m Apple really has it: {:?} etag={:?}",
                                            back.title, back.etag
                                        ),
                                        None => println!(
                                            "  \x1b[31mFAIL\x1b[0m Apple took the write but does not have the event"
                                        ),
                                    }
                                    let to_delete = found.unwrap_or(event);
                                    match quince_icloud::sync::delete_event(
                                        &http, &root, params, &mut library, &to_delete,
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            let still_there = quince_icloud::sync::fetch_calendar(
                                                &http,
                                                &root,
                                                client.service_params(),
                                                &day,
                                                &day,
                                                "Asia/Kolkata",
                                            )
                                            .await
                                            .map(|snap| {
                                                snap.events
                                                    .iter()
                                                    .any(|e| e.guid == to_delete.guid)
                                            })
                                            .unwrap_or(false);
                                            if still_there {
                                                println!(
                                                    "  \x1b[31mFAIL\x1b[0m Apple took the delete but still has the event"
                                                );
                                            } else {
                                                println!("  \x1b[32mPASS\x1b[0m gone from Apple; nothing left behind");
                                            }
                                        }
                                        Err(e) => println!(
                                            "  \x1b[31mFAIL\x1b[0m delete: {e}\n  \x1b[33mthe test event is still in {}\x1b[0m",
                                            calendar.title
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- does writing a note actually work? --------------------------------------
    // Same discipline as the calendar: create a note, edit it, read both back from Apple,
    // then delete it. Only ever touches the note it made. Runs with --write-test.
    if std::env::args().any(|a| a == "--write-test") {
        println!("\n\x1b[1m── Notes write round trip ──\x1b[0m");
        if let Some(ck) = session.service_url("ckdatabasews") {
            let params = client.service_params();
            let http = client.http().clone();
            let zc = quince_icloud::cloudkit::ZoneClient::new(
                client.http(),
                &ck,
                "com.apple.notes",
                client.service_params(),
            );
            let body = "Quince write test\nsafe to delete";

            match quince_icloud::notes::create_cloudkit(&http, &ck, params.clone(), None, body)
                .await
            {
                Err(e) => println!("  \x1b[31mFAIL\x1b[0m create: {e}"),
                Ok(guid) => {
                    println!("  \x1b[32mPASS\x1b[0m Apple accepted the create ({guid})");

                    let read_back = |name: String| {
                        let zc = &zc;
                        async move {
                            zc.lookup_record("Notes", &name)
                                .await
                                .ok()
                                .flatten()
                                .and_then(|rec| {
                                    quince_icloud::cloudkit::field::document_text(
                                        &rec,
                                        "TextDataEncrypted",
                                    )
                                })
                        }
                    };

                    match read_back(guid.clone()).await {
                        Some(text) if text == body => {
                            println!("  \x1b[32mPASS\x1b[0m Apple really has it, text matches");
                        }
                        Some(text) => println!(
                            "  \x1b[33mstored, but the text came back as {:?}\x1b[0m",
                            text.chars().take(50).collect::<String>()
                        ),
                        None => {
                            println!("  \x1b[31mFAIL\x1b[0m Apple took the write but has no note")
                        }
                    }

                    // Now the part that matters: an edit must rewrite the document in place.
                    let edited = "Quince write test\nsafe to delete\nedited by the round trip";
                    let note = quince_core::notes::Note {
                        guid: guid.clone(),
                        folder_guid: None,
                        title: "Quince write test".into(),
                        snippet: None,
                        body: Some(body.into()),
                        locked: false,
                        pinned: false,
                        created_at: None,
                        modified_at: None,
                        editable: true,
                        etag: None,
                    };
                    match quince_icloud::notes::edit_cloudkit(
                        &http,
                        &ck,
                        params.clone(),
                        &note,
                        edited,
                    )
                    .await
                    {
                        Err(e) => println!("  \x1b[31mFAIL\x1b[0m edit: {e}"),
                        Ok(()) => match read_back(guid.clone()).await {
                            Some(text) if text == edited => {
                                println!("  \x1b[32mPASS\x1b[0m the edit is what Apple now holds");
                            }
                            Some(text) => println!(
                                "  \x1b[31mFAIL\x1b[0m edit stored the wrong text: {:?}",
                                text.chars().take(60).collect::<String>()
                            ),
                            None => {
                                println!("  \x1b[31mFAIL\x1b[0m the note vanished after the edit")
                            }
                        },
                    }

                    match quince_icloud::notes::delete_cloudkit(&http, &ck, params, &note).await {
                        Ok(()) => println!("  \x1b[32mPASS\x1b[0m deleted; nothing left behind"),
                        Err(e) => println!(
                            "  \x1b[31mFAIL\x1b[0m delete: {e}\n  \x1b[33mthe test note {guid} is still there\x1b[0m"
                        ),
                    }
                }
            }
        }
    }

    // ---- what does a Note_UserSpecific record actually look like? -----------------
    println!("\n\x1b[1m── Note_UserSpecific: the shape Apple writes ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.notes",
            client.service_params(),
        );
        if let Ok((records, _)) = zc.all_records("Notes", None).await {
            for rec in records
                .iter()
                .filter(|r| {
                    r.get("recordType")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t.contains("UserSpecific"))
                })
                .take(3)
            {
                println!(
                    "  {} {}",
                    rec.get("recordType")
                        .and_then(|t| t.as_str())
                        .unwrap_or("?"),
                    quince_icloud::cloudkit::field::record_name(rec).unwrap_or_default()
                );
                if let Some(fields) = rec.get("fields").and_then(|f| f.as_object()) {
                    for (k, v) in fields {
                        println!(
                            "      {k} = {}",
                            v.to_string().chars().take(110).collect::<String>()
                        );
                    }
                }
            }
        }
    }

    // ---- do priority and moving actually work? -----------------------------------
    // Create a reminder, set its priority, move it to another list, read each back from
    // Apple, then delete it. Only ever touches the reminder it made. Runs with --write-test.
    if std::env::args().any(|a| a == "--write-test") {
        println!("\n\x1b[1m── Reminders: priority and moving ──\x1b[0m");
        if let Some(ck) = session.service_url("ckdatabasews") {
            let params = client.service_params();
            let http = client.http().clone();
            let lists = quince_core::Library::open(&dir)
                .ok()
                .and_then(|l| l.reminder_lists().ok())
                .unwrap_or_default();

            if lists.len() < 2 {
                println!(
                    "  need two lists to test a move; this account has {}",
                    lists.len()
                );
            } else {
                let (from, to) = (&lists[0], &lists[1]);
                match quince_icloud::reminders::create_cloudkit(
                    &http,
                    &ck,
                    params.clone(),
                    &from.guid,
                    "Quince write test — safe to delete",
                )
                .await
                {
                    Err(e) => println!("  \x1b[31mFAIL\x1b[0m create: {e}"),
                    Ok(guid) => {
                        println!("  \x1b[32mPASS\x1b[0m created in {}", from.title);
                        let zc = quince_icloud::cloudkit::ZoneClient::new(
                            client.http(),
                            &ck,
                            "com.apple.reminders",
                            client.service_params(),
                        );
                        let reminder = quince_core::reminders::Reminder {
                            guid: guid.clone(),
                            list_guid: from.guid.clone(),
                            title: "Quince write test — safe to delete".into(),
                            notes: None,
                            due_at: None,
                            completed_at: None,
                            priority: 0,
                            flagged: false,
                            recur_frequency: None,
                            recur_interval: None,
                            etag: None,
                            raw: None,
                        };

                        let edit = quince_icloud::reminders::ReminderEdit {
                            priority: Some(1),
                            list_guid: Some(to.guid.clone()),
                            ..Default::default()
                        };
                        match quince_icloud::reminders::edit_cloudkit(
                            &http,
                            &ck,
                            params.clone(),
                            &reminder,
                            &edit,
                        )
                        .await
                        {
                            Err(e) => println!("  \x1b[31mFAIL\x1b[0m edit: {e}"),
                            Ok(()) => match zc.lookup_record("Reminders", &guid).await {
                                Ok(Some(rec)) => {
                                    let priority =
                                        quince_icloud::cloudkit::field::int(&rec, "Priority");
                                    let list =
                                        quince_icloud::cloudkit::field::reference(&rec, "List");
                                    let moved = list.as_deref() == Some(to.guid.as_str());
                                    println!(
                                        "  {} priority is {:?} (asked for high = 1)",
                                        if priority == Some(1) {
                                            "\x1b[32mPASS\x1b[0m"
                                        } else {
                                            "\x1b[31mFAIL\x1b[0m"
                                        },
                                        priority
                                    );
                                    println!(
                                        "  {} list is now {}",
                                        if moved {
                                            "\x1b[32mPASS\x1b[0m"
                                        } else {
                                            "\x1b[31mFAIL\x1b[0m"
                                        },
                                        if moved {
                                            to.title.clone()
                                        } else {
                                            format!("{list:?}")
                                        }
                                    );
                                }
                                _ => println!("  \x1b[31mFAIL\x1b[0m the reminder vanished"),
                            },
                        }

                        match quince_icloud::reminders::delete_cloudkit(
                            &http, &ck, params, &reminder,
                        )
                        .await
                        {
                            Ok(()) => println!("  \x1b[32mPASS\x1b[0m deleted; nothing left behind"),
                            Err(e) => println!(
                                "  \x1b[31mFAIL\x1b[0m delete: {e}\n  \x1b[33mtest reminder {guid} is still there\x1b[0m"
                            ),
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
