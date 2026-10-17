//! Live, throwaway list and reminder only: a daily reminder, completed through Apple's own
//! CompleteRecurringReminder query, read back before and after — then the list deleted.
use anyhow::{anyhow, Result};
use serde_json::Value;
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
    let (http, p) = (client.http().clone(), client.service_params());
    use quince_icloud::reminders as r;
    let list = r::create_list_cloudkit(&http, &root, p.clone(), "Quince repeat test").await?;
    let guid = r::create_cloudkit(&http, &root, p.clone(), &list, "Daily test").await?;
    let mut rem = quince_core::reminders::Reminder {
        guid: guid.clone(),
        list_guid: list.clone(),
        title: "Daily test".into(),
        ..Default::default()
    };
    let due = 1_792_300_000; // a moment tomorrow
    r::edit_cloudkit(
        &http,
        &root,
        p.clone(),
        &rem,
        &r::ReminderEdit {
            due_at: Some(Some(due)),
            ..Default::default()
        },
    )
    .await?;
    rem.due_at = Some(due);
    r::set_recurrence_cloudkit(
        &http,
        &root,
        p.clone(),
        &rem,
        Some(r::Repeat {
            frequency: 0,
            interval: 1,
        }),
    )
    .await?;
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let show = |v: &Value| {
        format!(
            "Completed={} DueDate={} RecurrenceRuleIDs={}",
            v["fields"]["Completed"]["value"],
            v["fields"]["DueDate"]["value"],
            v["fields"]["RecurrenceRuleIDs"]["value"]
        )
    };
    let before = zc
        .lookup_record("Reminders", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("before: {}", show(&before));
    rem.recur_frequency = Some(0);
    rem.recur_interval = Some(1);
    rem.timezone = Some("Asia/Kolkata".into());
    r::set_completed_cloudkit(&http, &root, p.clone(), &rem, true).await?;
    println!("ticked through Quince's own set_completed_cloudkit");
    let after = zc
        .lookup_record("Reminders", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("after:  {}", show(&after));
    r::delete_list_cloudkit(&http, &root, p, &list).await?;
    println!("test list deleted");
    Ok(())
}
