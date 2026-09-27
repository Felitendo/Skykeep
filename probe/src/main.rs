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
                Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) => {
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
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Notes", None).await
        {
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

            match quince_icloud::notes::create_cloudkit(
                &http,
                &ck,
                params.clone(),
                None,
                body,
                None,
            )
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
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Notes", None).await
        {
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
                            parent_guid: None,
                            recur_frequency: None,
                            recur_interval: None,
                            etag: None,
                            raw: None,
                            timezone: None,
                            section_guid: None,
                        };

                        // Subtasks: nest one reminder under another, read it back, free it.
                        match quince_icloud::reminders::create_cloudkit(
                            &http,
                            &ck,
                            params.clone(),
                            &from.guid,
                            "Quince subtask test — safe to delete",
                        )
                        .await
                        {
                            Err(e) => println!("  \x1b[31mFAIL\x1b[0m create subtask: {e}"),
                            Ok(child_guid) => {
                                let child = quince_core::reminders::Reminder {
                                    guid: child_guid.clone(),
                                    ..reminder.clone()
                                };
                                let parent_of = |name: String| {
                                    let zc = &zc;
                                    async move {
                                        let rec = zc
                                            .lookup_record("Reminders", &name)
                                            .await
                                            .ok()
                                            .flatten()?;
                                        quince_icloud::cloudkit::field::reference(
                                            &rec,
                                            "ParentReminder",
                                        )
                                    }
                                };

                                match quince_icloud::reminders::edit_cloudkit(
                                    &http,
                                    &ck,
                                    params.clone(),
                                    &child,
                                    &quince_icloud::reminders::ReminderEdit {
                                        parent_guid: Some(Some(guid.clone())),
                                        ..Default::default()
                                    },
                                )
                                .await
                                {
                                    Err(e) => println!("  \x1b[31mFAIL\x1b[0m nest: {e}"),
                                    Ok(()) => match parent_of(child_guid.clone()).await {
                                        Some(p) if p == guid => {
                                            println!("  \x1b[32mPASS\x1b[0m Apple nested it under its parent");
                                        }
                                        other => println!(
                                            "  \x1b[31mFAIL\x1b[0m parent came back as {other:?}"
                                        ),
                                    },
                                }

                                match quince_icloud::reminders::edit_cloudkit(
                                    &http,
                                    &ck,
                                    params.clone(),
                                    &child,
                                    &quince_icloud::reminders::ReminderEdit {
                                        parent_guid: Some(None),
                                        ..Default::default()
                                    },
                                )
                                .await
                                {
                                    Err(e) => println!("  \x1b[31mFAIL\x1b[0m un-nest: {e}"),
                                    Ok(()) => match parent_of(child_guid.clone()).await {
                                        None => {
                                            println!("  \x1b[32mPASS\x1b[0m and let it back out")
                                        }
                                        Some(p) => {
                                            println!("  \x1b[31mFAIL\x1b[0m still nested under {p}")
                                        }
                                    },
                                }

                                let _ = quince_icloud::reminders::delete_cloudkit(
                                    &http,
                                    &ck,
                                    params.clone(),
                                    &child,
                                )
                                .await;
                                let _ = zc.delete_record("Reminders", &child_guid).await;
                            }
                        }

                        // Repeats: set one, read it back, change it, clear it.
                        let dated = quince_core::reminders::Reminder {
                            due_at: Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
                                    + 86_400,
                            ),
                            ..reminder.clone()
                        };
                        let _ = quince_icloud::reminders::edit_cloudkit(
                            &http,
                            &ck,
                            params.clone(),
                            &dated,
                            &quince_icloud::reminders::ReminderEdit {
                                due_at: Some(dated.due_at),
                                ..Default::default()
                            },
                        )
                        .await;

                        let rule_of = |name: String| {
                            let zc = &zc;
                            async move {
                                let rec =
                                    zc.lookup_record("Reminders", &name).await.ok().flatten()?;
                                let ids = rec
                                    .pointer("/fields/RecurrenceRuleIDs/value")?
                                    .as_array()?
                                    .iter()
                                    .filter_map(|v| v.as_str())
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>();
                                let id = ids.first()?.clone();
                                let rule = zc
                                    .lookup_record("Reminders", &format!("RecurrenceRule/{id}"))
                                    .await
                                    .ok()
                                    .flatten()?;
                                Some((
                                    quince_icloud::cloudkit::field::int(&rule, "Frequency")?,
                                    quince_icloud::cloudkit::field::int(&rule, "Interval")?,
                                ))
                            }
                        };

                        for (freq, every, what) in
                            [(1_i64, 2_i64, "every 2 weeks"), (0, 1, "every day")]
                        {
                            match quince_icloud::reminders::set_recurrence_cloudkit(
                                &http,
                                &ck,
                                params.clone(),
                                &dated,
                                Some(quince_icloud::reminders::Repeat {
                                    frequency: freq,
                                    interval: every,
                                }),
                            )
                            .await
                            {
                                Err(e) => println!("  \x1b[31mFAIL\x1b[0m set {what}: {e}"),
                                Ok(()) => match rule_of(guid.clone()).await {
                                    Some((f, i)) if f == freq && i == every => {
                                        println!("  \x1b[32mPASS\x1b[0m Apple stored {what}");
                                    }
                                    other => println!(
                                        "  \x1b[31mFAIL\x1b[0m asked for {what}, Apple has {other:?}"
                                    ),
                                },
                            }
                        }

                        match quince_icloud::reminders::set_recurrence_cloudkit(
                            &http,
                            &ck,
                            params.clone(),
                            &dated,
                            None,
                        )
                        .await
                        {
                            Err(e) => println!("  \x1b[31mFAIL\x1b[0m clear repeat: {e}"),
                            Ok(()) => match rule_of(guid.clone()).await {
                                None => println!("  \x1b[32mPASS\x1b[0m the repeat is gone"),
                                Some(found) => println!(
                                    "  \x1b[31mFAIL\x1b[0m cleared, but Apple still has {found:?}"
                                ),
                            },
                        }

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

    // ---- what is in a RecurrenceRule record? --------------------------------------
    // Setting a repeat means writing one of these and linking it. Measure it first.
    println!("\n\x1b[1m── RecurrenceRule: every field Apple writes ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.reminders",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Reminders", None).await
        {
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("RecurrenceRule"))
                .take(2)
            {
                println!(
                    "  {}",
                    quince_icloud::cloudkit::field::record_name(rec).unwrap_or_default()
                );
                if let Some(fields) = rec.get("fields").and_then(|f| f.as_object()) {
                    for (k, v) in fields {
                        println!(
                            "      {k} = {}",
                            v.to_string().chars().take(120).collect::<String>()
                        );
                    }
                }
            }

            // And how the reminder points at its rule.
            for rec in records
                .iter()
                .filter(|r| {
                    r.get("recordType").and_then(|t| t.as_str()) == Some("Reminder")
                        && r.pointer("/fields/RecurrenceRuleIDs/value")
                            .and_then(|v| v.as_array())
                            .is_some_and(|a| !a.is_empty())
                })
                .take(2)
            {
                println!(
                    "  reminder {} RecurrenceRuleIDs = {}",
                    quince_icloud::cloudkit::field::record_name(rec).unwrap_or_default(),
                    rec.pointer("/fields/RecurrenceRuleIDs")
                        .map(std::string::ToString::to_string)
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect::<String>()
                );
            }
        }
    }

    // ---- what does the contacts service actually hand back? -----------------------
    // Writing a contact needs the sync and preference tokens Apple hands out at startup,
    // and the exact shape of a card. Measure both before writing anything.
    println!("\n\x1b[1m── Contacts: tokens and card shape ──\x1b[0m");
    if let Some(root) = session.service_url("contacts") {
        let mut query = client.service_params();
        query.push(("clientVersion".into(), "2.1".into()));
        query.push(("locale".into(), "en_US".into()));
        query.push(("order".into(), "last,first".into()));

        match client
            .http()
            .get(format!("{}/co/startup", root.trim_end_matches('/')))
            .query(&query)
            .send()
            .await
        {
            Err(e) => println!("  request failed: {e}"),
            Ok(response) => {
                let text = response.text().await.unwrap_or_default();
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Err(e) => println!("  not JSON: {e}"),
                    Ok(body) => {
                        if let Some(obj) = body.as_object() {
                            let mut keys: Vec<&String> = obj.keys().collect();
                            keys.sort();
                            println!(
                                "  top-level keys: {}",
                                keys.iter()
                                    .map(|k| k.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                            for token in ["syncToken", "prefToken", "collectionSyncToken"] {
                                if let Some(v) = obj.get(token).and_then(|v| v.as_str()) {
                                    println!("  {token} = {v}");
                                }
                            }
                        }
                        // One whole card, so a write can echo every field back rather than
                        // only the ones this app happens to model.
                        if let Some(card) = body
                            .get("contacts")
                            .and_then(|c| c.as_array())
                            .and_then(|c| c.first())
                        {
                            println!("  one card, every key:");
                            if let Some(obj) = card.as_object() {
                                for (k, v) in obj {
                                    println!(
                                        "      {k} = {}",
                                        v.to_string().chars().take(90).collect::<String>()
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- does writing a contact actually work? ------------------------------------
    // Create one obviously-named contact, read it back from Apple, change it, read it back
    // again, then delete it. Only ever touches the contact it made. Runs with --write-test.
    if std::env::args().any(|a| a == "--write-test") {
        println!("\n\x1b[1m── Contacts write round trip ──\x1b[0m");
        if let Some(root) = session.service_url("contacts") {
            let http = client.http().clone();
            let params = client.service_params();

            let find = |guid: String| {
                let http = http.clone();
                let root = root.clone();
                let params = params.clone();
                async move {
                    quince_icloud::contacts::fetch(&http, &root, params)
                        .await
                        .ok()
                        .and_then(|all| all.into_iter().find(|c| c.guid == guid))
                }
            };

            match quince_icloud::contacts::fetch_with_tokens(&http, &root, params.clone()).await {
                Err(e) => println!("  \x1b[31mFAIL\x1b[0m tokens: {e}"),
                Ok((_, tokens)) => {
                    let draft = quince_core::contacts::Contact {
                        guid: String::new(),
                        first_name: Some("Quince".into()),
                        middle_name: None,
                        photo_url: None,
                        photo_signature: None,
                        photo_crop: None,
                        last_name: Some("Write Test".into()),
                        company: None,
                        is_company: false,
                        birthday: None,
                        notes: Some("safe to delete".into()),
                        etag: None,
                        methods: vec![quince_core::contacts::ContactMethod {
                            kind: "phone".into(),
                            label: Some("mobile".into()),
                            value: "+10000000000".into(),
                        }],
                    };

                    match quince_icloud::contacts::create(
                        &http,
                        &root,
                        params.clone(),
                        &tokens,
                        &draft,
                    )
                    .await
                    {
                        Err(e) => println!("  \x1b[31mFAIL\x1b[0m create: {e}"),
                        Ok(guid) => {
                            println!("  \x1b[32mPASS\x1b[0m Apple accepted the create ({guid})");
                            let stored = find(guid.clone()).await;
                            match &stored {
                                Some(c) if c.last_name.as_deref() == Some("Write Test") => {
                                    println!(
                                        "  \x1b[32mPASS\x1b[0m Apple really has it, with {} method(s)",
                                        c.methods.len()
                                    );
                                }
                                other => println!("  \x1b[31mFAIL\x1b[0m read back as {other:?}"),
                            }

                            if let Some(mut stored) = stored {
                                // Editing needs the current tokens and Apple's own card, so
                                // fields this app does not model survive the write.
                                let refreshed = quince_icloud::contacts::fetch_with_tokens(
                                    &http,
                                    &root,
                                    params.clone(),
                                )
                                .await
                                .map(|(_, t)| t);
                                let card = quince_icloud::contacts::card(
                                    &http,
                                    &root,
                                    params.clone(),
                                    &guid,
                                )
                                .await
                                .ok()
                                .flatten();

                                stored.company = Some("Quince".into());
                                match refreshed {
                                    Err(e) => println!("  \x1b[31mFAIL\x1b[0m refresh tokens: {e}"),
                                    Ok(tokens) => match quince_icloud::contacts::update(
                                        &http,
                                        &root,
                                        params.clone(),
                                        &tokens,
                                        &stored,
                                        card.as_ref(),
                                    )
                                    .await
                                    {
                                        Err(e) => println!("  \x1b[31mFAIL\x1b[0m update: {e}"),
                                        Ok(()) => match find(guid.clone()).await {
                                            Some(c) if c.company.as_deref() == Some("Quince") => {
                                                println!("  \x1b[32mPASS\x1b[0m the edit is what Apple now holds");
                                            }
                                            other => println!(
                                                "  \x1b[31mFAIL\x1b[0m edit not stored: {:?}",
                                                other.map(|c| c.company)
                                            ),
                                        },
                                    },
                                }

                                let latest = find(guid.clone()).await;
                                let tokens = quince_icloud::contacts::fetch_with_tokens(
                                    &http,
                                    &root,
                                    params.clone(),
                                )
                                .await
                                .map(|(_, t)| t);
                                match (latest, tokens) {
                                    (Some(c), Ok(tokens)) => match quince_icloud::contacts::delete(
                                        &http, &root, params.clone(), &tokens, &c,
                                    )
                                    .await
                                    {
                                        Err(e) => println!(
                                            "  \x1b[31mFAIL\x1b[0m delete: {e}\n  \x1b[33mthe test contact {guid} is still there\x1b[0m"
                                        ),
                                        Ok(()) => match find(guid.clone()).await {
                                            None => println!("  \x1b[32mPASS\x1b[0m gone from Apple; nothing left behind"),
                                            Some(_) => println!("  \x1b[31mFAIL\x1b[0m delete accepted but it is still there"),
                                        },
                                    },
                                    _ => println!("  \x1b[33mcould not re-read before deleting\x1b[0m"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- how does Apple express sections and subtasks? -----------------------------
    println!("\n\x1b[1m── Reminders: sections and subtasks ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.reminders",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Reminders", None).await
        {
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("ListSection"))
                .take(3)
            {
                println!(
                    "  ListSection {}",
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

            // Which fields on a Reminder could carry a parent or a section?
            let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("Reminder"))
            {
                if let Some(fields) = rec.get("fields").and_then(|f| f.as_object()) {
                    for k in fields.keys() {
                        seen.insert(k.clone());
                    }
                }
            }
            println!(
                "  every field seen on a Reminder: {}",
                seen.iter().cloned().collect::<Vec<_>>().join(", ")
            );

            // And any reminder that actually has a parent or section set.
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("Reminder"))
            {
                for key in ["ParentID", "Parent", "ListSection", "SectionID", "Subtasks"] {
                    if let Some(v) = rec.pointer(&format!("/fields/{key}")) {
                        println!(
                            "  {} has {key} = {}",
                            quince_icloud::cloudkit::field::document_text(rec, "TitleDocument")
                                .unwrap_or_default()
                                .chars()
                                .take(28)
                                .collect::<String>(),
                            v.to_string().chars().take(100).collect::<String>()
                        );
                    }
                }
            }
        }
    }

    // ---- how does a reminder belong to a section? ----------------------------------
    // No reminder field names a section, so membership is expressed some other way. Find it
    // before building anything on a guess.
    println!("\n\x1b[1m── Reminders: how section membership is expressed ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.reminders",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Reminders", None).await
        {
            let sections: std::collections::HashMap<String, String> = records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("ListSection"))
                .filter(|r| !quince_icloud::cloudkit::field::flag(r, "Deleted"))
                .filter_map(|r| {
                    Some((
                        quince_icloud::cloudkit::field::record_name(r)?,
                        quince_icloud::cloudkit::field::string(r, "DisplayName")
                            .unwrap_or_default(),
                    ))
                })
                .collect();
            println!("  live sections: {sections:?}");

            // Does any reminder point its List reference at a section rather than a list?
            let mut via_list = 0;
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("Reminder"))
            {
                if let Some(target) = quince_icloud::cloudkit::field::reference(rec, "List") {
                    if sections.contains_key(&target) {
                        via_list += 1;
                        if via_list <= 3 {
                            println!(
                                "  \x1b[32m{}\x1b[0m has List -> {} ({})",
                                quince_icloud::cloudkit::field::document_text(rec, "TitleDocument")
                                    .unwrap_or_default()
                                    .chars()
                                    .take(34)
                                    .collect::<String>(),
                                target,
                                sections[&target]
                            );
                        }
                    }
                }
            }
            println!("  reminders whose List points at a section: {via_list}");

            // And what does the owning List record say about its sections?
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("List"))
                .filter(|r| {
                    quince_icloud::cloudkit::field::record_name(r).as_deref()
                        == Some("List/A233F124-67A7-46EC-9FFB-2908DA86C61D")
                })
            {
                println!("  the list that owns a section:");
                if let Some(fields) = rec.get("fields").and_then(|f| f.as_object()) {
                    for (k, v) in fields {
                        println!(
                            "      {k} = {}",
                            v.to_string().chars().take(120).collect::<String>()
                        );
                    }
                }
            }
        }
    }

    // ---- is the section-membership asset readable? ---------------------------------
    // Membership lives in a CloudKit asset with a checksum beside it, not in a reference.
    // Whether sections can be shown at all depends on that blob being tractable, so look
    // before promising anything.
    println!("\n\x1b[1m── Section membership: what is in the asset ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.reminders",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Reminders", None).await
        {
            let asset = records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("List"))
                .find_map(|r| {
                    r.pointer("/fields/MembershipsOfRemindersInSectionsAsData/value/downloadURL")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                });

            match asset {
                None => println!("  no list on this account carries one"),
                Some(url) => match client.http().get(&url).send().await {
                    Err(e) => println!("  \x1b[31mdownload failed:\x1b[0m {e}"),
                    Ok(response) => {
                        let status = response.status();
                        let bytes = response.bytes().await.unwrap_or_default();
                        println!("  {status}, {} byte(s)", bytes.len());
                        if bytes.is_empty() {
                            return Ok(());
                        }
                        println!(
                            "  first bytes: {}",
                            bytes
                                .iter()
                                .take(24)
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        // It starts `{"memberships":` — this is plain JSON, not a blob.
                        match std::str::from_utf8(&bytes)
                            .ok()
                            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                        {
                            None => println!("  not readable as JSON"),
                            Some(parsed) => {
                                if let Some(obj) = parsed.as_object() {
                                    println!(
                                        "  top-level keys: {}",
                                        obj.keys().cloned().collect::<Vec<_>>().join(", ")
                                    );
                                }
                                // What does ONE membership entry carry? If nothing names a
                                // section, membership must be expressed somewhere else.
                                if let Some(first) =
                                    parsed.pointer("/memberships/0").and_then(|v| v.as_object())
                                {
                                    println!(
                                        "  one membership's keys: {}",
                                        first.keys().cloned().collect::<Vec<_>>().join(", ")
                                    );
                                }
                                let count = parsed
                                    .pointer("/memberships")
                                    .and_then(|v| v.as_array())
                                    .map_or(0, Vec::len);
                                println!("  {count} membership(s) in this list's asset");
                                // Anything that is not the memberships array is the part
                                // that would say which section.
                                if let Some(obj) = parsed.as_object() {
                                    for (k, v) in obj {
                                        if k != "memberships" {
                                            println!(
                                                "  {k} = {}",
                                                v.to_string().chars().take(300).collect::<String>()
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }

    // ---- what does a Notes Folder record carry? ------------------------------------
    // "All Notes" must not include the ones in Recently Deleted, so something has to say
    // which folder that is — and it cannot be the name, which is translated.
    println!("\n\x1b[1m── Notes folders: every field ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.notes",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Notes", None).await
        {
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("Folder"))
            {
                println!(
                    "  {}",
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

    // ---- what does Drive's docws protocol actually support? ------------------------
    // Only reading has been measured. Renaming, deleting, moving, downloading and making a
    // folder each need their own shape, so find out what the service actually accepts
    // before writing any of it. --write-test also proves create + rename + delete live.
    println!("\n\x1b[1m── Drive: docws capability probe ──\x1b[0m");
    println!("  drivews = {:?}", session.service_url("drivews"));
    println!("  docws   = {:?}", session.service_url("docws"));
    if let Some(root) = session.service_url("drivews") {
        let params = client.service_params();
        let http = client.http().clone();

        // A real file, from the local library rather than an empty root — read-only.
        if let Ok(mut library) = quince_core::Library::open(&dir) {
            // Sync a couple of levels so there is something real to test against, whether
            // or not this probe's own library has ever browsed Drive before.
            let _ = quince_icloud::drive::sync_folder(
                &http,
                &root,
                params.clone(),
                &mut library,
                "FOLDER::com.apple.CloudDocs::root",
            )
            .await;
            let root_children = library
                .drive_children("FOLDER::com.apple.CloudDocs::root")
                .unwrap_or_default();
            if let Some(sub) = root_children.iter().find(|i| i.kind == "FOLDER") {
                let _ = quince_icloud::drive::sync_folder(
                    &http,
                    &root,
                    params.clone(),
                    &mut library,
                    &sub.drivewsid,
                )
                .await;
            }
            let file = root_children
                .iter()
                .find(|i| i.kind == "FILE")
                .cloned()
                .or_else(|| {
                    // The root's own files did not have to include one; look one level
                    // into the first folder if the root came up empty.
                    root_children
                        .iter()
                        .find(|i| i.kind == "FOLDER")
                        .and_then(|f| library.drive_children(&f.drivewsid).ok())
                        .and_then(|items| items.into_iter().find(|i| i.kind == "FILE"))
                });

            match file {
                None => println!("  no file synced yet to test with"),
                Some(file) => {
                    println!(
                        "  testing against: {} ({} bytes)",
                        file.name,
                        file.size.unwrap_or(0)
                    );
                    let docwsid = file
                        .drivewsid
                        .rsplit("::")
                        .next()
                        .unwrap_or(&file.drivewsid);

                    // Downloads are served from the `docws` host, not `drivews`.
                    if let Some(docws_root) = session.service_url("docws") {
                        let dl = http
                            .get(format!(
                                "{}/ws/com.apple.CloudDocs/download/by_id",
                                docws_root.trim_end_matches('/')
                            ))
                            .query(&[("document_id", docwsid)])
                            .send()
                            .await;
                        match dl {
                            Err(e) => println!("  by_id request failed: {e}"),
                            Ok(r) => {
                                let text = r.text().await.unwrap_or_default();
                                let url = serde_json::from_str::<serde_json::Value>(&text)
                                    .ok()
                                    .and_then(|v| {
                                        v.pointer("/data_token/url")
                                            .and_then(|u| u.as_str())
                                            .map(ToString::to_string)
                                    });
                                match url {
                                    None => println!("  no data_token.url in the reply"),
                                    Some(url) => match http.get(&url).send().await {
                                        Err(e) => println!("  fetching the bytes failed: {e}"),
                                        Ok(bytes_response) => {
                                            let status = bytes_response.status();
                                            let bytes =
                                                bytes_response.bytes().await.unwrap_or_default();
                                            let matches_reported_size =
                                                file.size.is_none_or(|s| s as usize == bytes.len());
                                            println!(
                                                "  \x1b[32mPASS\x1b[0m fetched {} byte(s), status {status}, matches reported size: {matches_reported_size}",
                                                bytes.len()
                                            );
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- does the real Drive backend work end to end? ------------------------------
    // Not the raw HTTP calls above — the actual quince_icloud::drive functions the app
    // uses. Create a folder, download a real file into it (proving the docws host and the
    // data_token redirect), rename, delete. Runs with --write-test.
    if std::env::args().any(|a| a == "--write-test") {
        println!("\n\x1b[1m── Drive: the real backend, end to end ──\x1b[0m");
        if let (Some(drivews), Some(docws)) =
            (session.service_url("drivews"), session.service_url("docws"))
        {
            let params = client.service_params();
            let http = client.http().clone();

            match quince_icloud::drive::create_folder(
                &http,
                &drivews,
                params.clone(),
                quince_core::drive::ROOT,
                "Quince backend test — safe to delete",
            )
            .await
            {
                Err(e) => println!("  \x1b[31mFAIL\x1b[0m create_folder: {e}"),
                Ok(folder) => {
                    println!("  \x1b[32mPASS\x1b[0m created {}", folder.drivewsid);

                    match quince_icloud::drive::rename(
                        &http,
                        &drivews,
                        params.clone(),
                        &folder,
                        "Quince backend test renamed",
                    )
                    .await
                    {
                        Ok(()) => println!("  \x1b[32mPASS\x1b[0m renamed"),
                        Err(e) => println!("  \x1b[31mFAIL\x1b[0m rename: {e}"),
                    }

                    // A real download, through the actual function the app calls — proves
                    // the docws host, the data_token redirect, and the bytes landing on
                    // disk all still work together.
                    if let Ok(mut library) = quince_core::Library::open(&dir) {
                        let _ = quince_icloud::drive::sync_folder(
                            &http,
                            &drivews,
                            params.clone(),
                            &mut library,
                            quince_core::drive::ROOT,
                        )
                        .await;
                        let root_children = library
                            .drive_children(quince_core::drive::ROOT)
                            .unwrap_or_default();
                        println!(
                            "  root has {} item(s): {:?}",
                            root_children.len(),
                            root_children
                                .iter()
                                .map(|i| (&i.name, &i.kind))
                                .collect::<Vec<_>>()
                        );
                        if let Some(file) = root_children.into_iter().find(|i| i.kind == "FILE") {
                            let destination = std::env::temp_dir()
                                .join(format!("quince-backend-test-{}", uuid::Uuid::new_v4()));
                            match quince_icloud::drive::download_to(
                                &http,
                                &docws,
                                params.clone(),
                                &file,
                                &destination,
                            )
                            .await
                            {
                                Err(e) => println!("  \x1b[31mFAIL\x1b[0m download_to: {e}"),
                                Ok(()) => {
                                    let bytes =
                                        std::fs::metadata(&destination).map(|m| m.len()).ok();
                                    println!(
                                        "  \x1b[32mPASS\x1b[0m downloaded {} to disk: {:?} byte(s)",
                                        file.name, bytes
                                    );
                                    std::fs::remove_file(&destination).ok();
                                }
                            }
                        }
                    }

                    match quince_icloud::drive::delete(&http, &drivews, params, &folder).await {
                        Ok(()) => println!("  \x1b[32mPASS\x1b[0m deleted; nothing left behind"),
                        Err(e) => println!(
                            "  \x1b[31mFAIL\x1b[0m delete: {e}\n  \x1b[33m{} is still there\x1b[0m",
                            folder.drivewsid
                        ),
                    }
                }
            }
        }
    }

    // ---- how does uploading a file actually work? ----------------------------------
    // pyicloud's upload is a two-step dance: ask docws for a signed upload URL, PUT the
    // bytes there, then tell drivews what was uploaded so it becomes a real Drive item.
    // Measure each step against the real service before writing any of it. --write-test
    // only, and it cleans up after itself.
    if std::env::args().any(|a| a == "--write-test") {
        println!("\n\x1b[1m── Drive: upload capability probe ──\x1b[0m");
        if let (Some(drivews), Some(docws)) =
            (session.service_url("drivews"), session.service_url("docws"))
        {
            let params = client.service_params();
            let http = client.http().clone();
            let bytes = b"Quince upload test - safe to delete\n";
            let name = "quince-upload-test.txt";

            // Step 1: ask for somewhere to put the bytes. This is a JSON POST, not query
            // params — sending it as query params is what "Unexpected input at column 0"
            // turned out to mean: the server tried to parse an empty body as JSON.
            let url = format!(
                "{}/ws/com.apple.CloudDocs/upload/web",
                docws.trim_end_matches('/')
            );
            let request_body = serde_json::json!({
                "filename": name,
                "type": "FILE",
                "content_type": "text/plain",
                "size": bytes.len(),
            });
            let response = http
                .post(&url)
                .query(&params)
                .json(&request_body)
                .send()
                .await;
            match response {
                Err(e) => println!("  upload/web request failed: {e}"),
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    println!("  upload/web -> {status}: {text}");
                    let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) else {
                        println!("  not JSON, stopping here");
                        return Ok(());
                    };
                    // Print the whole shape rather than guess a path — the response nests
                    // this differently across Apple's own client versions.
                    println!(
                        "  full parsed shape:\n  {}",
                        serde_json::to_string_pretty(&body)
                            .unwrap_or_default()
                            .lines()
                            .collect::<Vec<_>>()
                            .join("\n  ")
                    );
                    let upload_url = [
                        body.pointer("/url"),
                        body.pointer("/data/upload_url"),
                        body.pointer("/0/url"),
                    ]
                    .into_iter()
                    .flatten()
                    .find_map(|v| v.as_str())
                    .map(ToString::to_string);
                    let Some(upload_url) = upload_url else {
                        println!("  no upload URL found at any guessed path");
                        return Ok(());
                    };

                    // Step 2: PUT the bytes there.
                    let put = http.put(&upload_url).body(bytes.to_vec()).send().await;
                    match put {
                        Err(e) => println!("  PUT to the upload URL failed: {e}"),
                        Ok(r) => {
                            let status = r.status();
                            let text = r.text().await.unwrap_or_default();
                            println!(
                                "  PUT bytes -> {status}: {}",
                                text.chars().take(300).collect::<String>()
                            );
                            let Ok(receipt) = serde_json::from_str::<serde_json::Value>(&text)
                            else {
                                println!("  not JSON, stopping here");
                                return Ok(());
                            };

                            // Step 3: tell drivews this document now belongs in a folder.
                            let single_file = receipt
                                .pointer("/singleFile")
                                .cloned()
                                .unwrap_or(receipt.clone());
                            let commit_url = format!(
                                "{}/ws/com.apple.CloudDocs/update/documents",
                                docws.trim_end_matches('/')
                            );
                            let commit_body = serde_json::json!({
                                "data": {
                                    "signature": single_file.get("fileChecksum"),
                                    "wrapping_key": single_file.get("wrappingKey"),
                                    "reference_signature": single_file.get("referenceChecksum"),
                                    "receipt": single_file.get("receipt"),
                                    "size": bytes.len(),
                                },
                                "command": "add_file",
                                "document_id": single_file.get("document_id"),
                                "path": {
                                    "starting_document_id": "FOLDER::com.apple.CloudDocs::root",
                                    "path": name,
                                },
                                "allow_conflict": true,
                                "file_flags": { "is_writable": true, "is_executable": false, "is_hidden": false },
                                "mtime": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                                "btime": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                            });
                            let commit = http
                                .post(&commit_url)
                                .query(&params)
                                .json(&commit_body)
                                .send()
                                .await;
                            match commit {
                                Err(e) => println!("  commit request failed: {e}"),
                                Ok(r) => {
                                    let status = r.status();
                                    let text = r.text().await.unwrap_or_default();
                                    println!(
                                        "  commit -> {status}: {}",
                                        text.chars().take(400).collect::<String>()
                                    );
                                }
                            }

                            // Clean up whatever landed, whether or not the commit worked.
                            let _ = quince_core::Library::open(&dir).ok();
                            let cleanup = quince_icloud::drive::sync_folder(
                                &http,
                                &drivews,
                                client.service_params(),
                                &mut quince_core::Library::open(&dir)?,
                                quince_core::drive::ROOT,
                            )
                            .await;
                            if cleanup.is_ok() {
                                let library = quince_core::Library::open(&dir)?;
                                if let Some(uploaded) = library
                                    .drive_children(quince_core::drive::ROOT)?
                                    .into_iter()
                                    .find(|i| i.name.starts_with("quince-upload-test"))
                                {
                                    match quince_icloud::drive::delete(
                                        &http,
                                        &drivews,
                                        client.service_params(),
                                        &uploaded,
                                    )
                                    .await
                                    {
                                        Ok(()) => println!("  cleaned up the uploaded test file"),
                                        Err(e) => println!(
                                            "  \x1b[33mcouldn't clean up: {e} — {} may still be there\x1b[0m",
                                            uploaded.name
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

    // ---- why does anything we write show up blank? ---------------------------------
    // Apple's note is NoteStoreProto{2: Document{3: Note}}; the Note holds its text at
    // field 2 and a repeated AttributeRun at field 5, each run's field 1 being a length.
    // If those lengths are the text measured in UTF-16 code units, then writing text
    // without them — or without updating them — leaves every Apple client with runs that
    // no longer describe what is there, which is exactly what a blank note looks like.
    println!("\n\x1b[1m── Do attribute runs carry the text length? ──\x1b[0m");
    if let Some(ck) = session.service_url("ckdatabasews") {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.notes",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Notes", None).await
        {
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
                let Ok(raw) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    encoded.trim(),
                ) else {
                    continue;
                };
                let Some(text) = quince_icloud::ckdocument::text_from_bytes(&raw) else {
                    continue;
                };
                let title = quince_icloud::cloudkit::field::document_text(rec, "TitleEncrypted")
                    .unwrap_or_default();
                let lengths = quince_icloud::ckdocument::debug_attribute_lengths(&raw);
                let total: u64 = lengths.iter().sum();
                let utf16 = text.encode_utf16().count() as u64;

                println!(
                    "  {:<34} chars {:<6} utf16 {:<6} runs {:?} total {} {}",
                    title.chars().take(32).collect::<String>(),
                    text.chars().count(),
                    utf16,
                    lengths,
                    total,
                    if total == utf16 {
                        "\x1b[32m← matches UTF-16 exactly\x1b[0m"
                    } else {
                        "\x1b[33m← does not match\x1b[0m"
                    }
                );
            }
        }

        // What is actually inside one run? Writing one from scratch means knowing whether a
        // bare length is enough or whether Apple expects styling fields alongside it.
        println!("\n  \x1b[1mThe raw bytes of real attribute runs\x1b[0m");
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zc.all_records("Notes", None).await
        {
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
                let Ok(raw) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    encoded.trim(),
                ) else {
                    continue;
                };
                let runs = quince_icloud::ckdocument::debug_attribute_runs(&raw);
                if runs.is_empty() {
                    continue;
                }
                let title = quince_icloud::cloudkit::field::document_text(rec, "TitleEncrypted")
                    .unwrap_or_default();
                println!("    {}", title.chars().take(30).collect::<String>());
                for run in runs.iter().take(3) {
                    println!(
                        "      {} byte(s): {}",
                        run.len(),
                        run.iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            }
        }

        // A reminder's title uses the same document format, and also comes back blank when
        // written from here.
        println!("\n  \x1b[1mReminder titles, same format\x1b[0m");
        let zr = quince_icloud::cloudkit::ZoneClient::new(
            client.http(),
            &ck,
            "com.apple.reminders",
            client.service_params(),
        );
        if let Ok(quince_icloud::cloudkit::ZoneWalk { records, .. }) =
            zr.all_records("Reminders", None).await
        {
            for rec in records
                .iter()
                .filter(|r| r.get("recordType").and_then(|t| t.as_str()) == Some("Reminder"))
                .take(4)
            {
                let Some(encoded) = rec
                    .pointer("/fields/TitleDocument/value")
                    .and_then(|v| v.as_str())
                else {
                    continue;
                };
                let Ok(raw) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    encoded.trim(),
                ) else {
                    continue;
                };
                let text = quince_icloud::ckdocument::text_from_bytes(&raw).unwrap_or_default();
                let lengths = quince_icloud::ckdocument::debug_attribute_lengths(&raw);
                let total: u64 = lengths.iter().sum();
                let utf16 = text.encode_utf16().count() as u64;
                println!(
                    "    {:<34} utf16 {:<5} runs {:?} total {} {}",
                    text.chars().take(32).collect::<String>(),
                    utf16,
                    lengths,
                    total,
                    if total == utf16 {
                        "\x1b[32m← matches\x1b[0m"
                    } else {
                        "\x1b[33m← does not match\x1b[0m"
                    }
                );
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
