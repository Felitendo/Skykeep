//! Throwaway: a reminder written here gets the alarm that makes a phone ring.
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::Value;

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
    let zone = quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &root,
        "com.apple.reminders",
        params.clone(),
    );

    // A real list to put it in.
    let walk = zone.all_records("Reminders", None).await?;
    let list = walk
        .records
        .iter()
        .filter(|r| r.get("recordType").and_then(Value::as_str) == Some("List"))
        .find(|r| {
            f(r, "Deleted").and_then(Value::as_i64) != Some(1)
                && f(r, "IsGroup").and_then(Value::as_i64) != Some(1)
        })
        .and_then(|r| r.pointer("/recordName").and_then(Value::as_str))
        .ok_or_else(|| anyhow!("no list to use"))?
        .to_string();

    let guid = quince_icloud::reminders::create_cloudkit(
        &http,
        &root,
        params.clone(),
        &list,
        "Quince alarm test — safe to delete",
    )
    .await?;
    println!("created {guid} in {list}");

    let due = jiff::Timestamp::now().as_second() + 3600;
    quince_icloud::reminders::set_alarm_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        Some(due),
        "Asia/Kolkata",
    )
    .await?;

    // Read it back from Apple, not from what we sent.
    let after = zone.all_records("Reminders", None).await?;
    let reminder = after
        .records
        .iter()
        .find(|r| r.pointer("/recordName").and_then(Value::as_str) == Some(guid.as_str()))
        .ok_or_else(|| anyhow!("the reminder is gone"))?;
    let ids: Vec<String> = f(reminder, "AlarmIDs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    println!("AlarmIDs after: {ids:?}");
    if ids.len() != 1 {
        return Err(anyhow!("expected exactly one alarm"));
    }

    let alarm = after
        .records
        .iter()
        .find(|r| {
            r.pointer("/recordName").and_then(Value::as_str)
                == Some(format!("Alarm/{}", ids[0]).as_str())
        })
        .ok_or_else(|| anyhow!("the Alarm record is not in the zone"))?;
    let trigger_id = f(alarm, "TriggerID")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no TriggerID"))?;
    println!("Alarm -> trigger {trigger_id}");

    let trigger = after
        .records
        .iter()
        .find(|r| {
            r.pointer("/recordName").and_then(Value::as_str)
                == Some(format!("AlarmTrigger/{trigger_id}").as_str())
        })
        .ok_or_else(|| anyhow!("the AlarmTrigger record is not in the zone"))?;
    let raw = f(trigger, "DateComponentsData")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no date"))?;
    println!("trigger Type={:?}", f(trigger, "Type"));
    println!(
        "trigger fires at: {}",
        String::from_utf8_lossy(&B64.decode(raw)?)
    );

    // Clearing must take all three away, not leave a phone ringing for a time that moved.
    quince_icloud::reminders::set_alarm_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        None,
        "Asia/Kolkata",
    )
    .await?;
    let cleared = zone.all_records("Reminders", None).await?;
    let still_there = cleared
        .records
        .iter()
        .filter(|r| {
            let n = r
                .pointer("/recordName")
                .and_then(Value::as_str)
                .unwrap_or("");
            (n == format!("Alarm/{}", ids[0]) || n == format!("AlarmTrigger/{trigger_id}"))
                && f(r, "Deleted").and_then(Value::as_i64) != Some(1)
        })
        .count();
    let after_clear = cleared
        .records
        .iter()
        .find(|r| r.pointer("/recordName").and_then(Value::as_str) == Some(guid.as_str()))
        .and_then(|r| f(r, "AlarmIDs").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0);
    println!("after clearing: {still_there} alarm records left, AlarmIDs has {after_clear}");

    // Tidy up: mark it deleted the way the app does.
    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, params.clone(), &mut library, None)
        .await?;
    if let Some(r) = library
        .reminder_lists()?
        .iter()
        .flat_map(|l| library.reminders_in(&l.guid, true).unwrap_or_default())
        .find(|r| r.guid == guid)
    {
        quince_icloud::reminders::delete_cloudkit(&http, &root, params.clone(), &r).await?;
        println!("deleted the test reminder");
    }
    if still_there > 0 || after_clear > 0 {
        return Err(anyhow!("clearing left something behind"));
    }
    println!("\nOK — the alarm lands, points at the right time, and goes away cleanly.");
    Ok(())
}
