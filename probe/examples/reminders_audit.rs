//! Throwaway: every reminder, as Apple has it and as this app stored it.
//!
//! The same shape of question that found a note filed in the wrong folder: not "did the write
//! return 200" but "do the two sides agree, right now, about real data".
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn f<'a>(r: &'a Value, k: &str) -> Option<&'a Value> {
    r.pointer(&format!("/fields/{k}/value"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let http = client.http().clone();
    let params = client.service_params();

    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, params.clone(), &mut library, None)
        .await?;

    let zone =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", params);
    let walk = zone.all_records("Reminders", None).await?;

    let mut mine: HashMap<String, quince_core::reminders::Reminder> = HashMap::new();
    for list in library.reminder_lists()? {
        for r in library.reminders_in(&list.guid, true)? {
            mine.insert(r.guid.clone(), r);
        }
    }
    println!(
        "{} lists, {} reminders in the library",
        library.reminder_lists()?.len(),
        mine.len()
    );

    let mut problems = 0;
    let mut checked = 0;
    let mut live = HashSet::new();

    for r in walk
        .records
        .iter()
        .filter(|r| r.get("recordType").and_then(Value::as_str) == Some("Reminder"))
        .filter(|r| f(r, "Deleted").and_then(Value::as_i64) != Some(1))
    {
        let guid = r
            .pointer("/recordName")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        live.insert(guid.clone());
        let title = f(r, "TitleDocument")
            .and_then(Value::as_str)
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default();

        let Some(ours) = mine.get(&guid) else {
            println!(
                "MISSING  {:?}  {guid}",
                title.chars().take(30).collect::<String>()
            );
            problems += 1;
            continue;
        };
        checked += 1;

        let mut says = Vec::new();
        if ours.title != title {
            says.push(format!("title {:?} vs {:?}", ours.title, title));
        }

        let apple_list = f(r, "List")
            .and_then(|v| v.get("recordName"))
            .and_then(Value::as_str);
        if Some(ours.list_guid.as_str()) != apple_list {
            says.push(format!("list {:?} vs {apple_list:?}", ours.list_guid));
        }
        let apple_due = f(r, "DueDate").and_then(Value::as_i64).map(|ms| ms / 1000);
        if ours.due_at != apple_due {
            says.push(format!("due {:?} vs {apple_due:?}", ours.due_at));
        }

        // A recurring reminder is one record that Apple advances: ticking today's occurrence
        // sets Completed and moves the due date to the next one. So the flag alone does not
        // say whether there is work outstanding, and a phone shows such a reminder unticked.
        // The first version of this check did read the flag alone and called three daily
        // reminders wrong when the app had them right.
        let recurring = f(r, "RecurrenceRuleIDs")
            .and_then(Value::as_array)
            .is_some_and(|ids| !ids.is_empty());
        let now = jiff::Timestamp::now().as_second();
        let pending = recurring && apple_due.is_some_and(|due| due > now);
        let apple_done = f(r, "Completed").and_then(Value::as_i64) == Some(1) && !pending;
        if ours.completed_at.is_some() != apple_done {
            says.push(format!(
                "done {} vs {apple_done}",
                ours.completed_at.is_some()
            ));
        }
        let apple_flagged = f(r, "Flagged").and_then(Value::as_i64) == Some(1);
        if ours.flagged != apple_flagged {
            says.push(format!("flagged {} vs {apple_flagged}", ours.flagged));
        }
        let apple_priority = f(r, "Priority").and_then(Value::as_i64).unwrap_or(0);
        if ours.priority != apple_priority {
            says.push(format!("priority {} vs {apple_priority}", ours.priority));
        }
        let apple_zone = f(r, "TimeZone").and_then(Value::as_str);
        if ours.timezone.as_deref() != apple_zone {
            says.push(format!("zone {:?} vs {apple_zone:?}", ours.timezone));
        }
        if !says.is_empty() {
            println!(
                "DIFFERS  {:<30} {}",
                title.chars().take(28).collect::<String>(),
                says.join("; ")
            );
            problems += 1;
        }
    }

    for guid in mine.keys() {
        if !live.contains(guid) {
            println!("GHOST    {guid} — the library has it, the account does not");
            problems += 1;
        }
    }

    println!("\n{checked} reminders compared, {problems} disagreement(s)");
    Ok(())
}
