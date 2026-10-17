//! Live, throwaway list and reminder only: an all-day date, then a timed one, read back.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? { return Err(anyhow!("no session")); }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    use quince_icloud::reminders as r;
    let list = r::create_list_cloudkit(&http, &root, p.clone(), "Quince all-day test").await?;
    let guid = r::create_cloudkit(&http, &root, p.clone(), &list, "All-day test").await?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let reminder = quince_core::reminders::Reminder { guid: guid.clone(), list_guid: list.clone(), title: "All-day test".into(), ..Default::default() };
    let show = |rec: &serde_json::Value| format!("AllDay={} DueDate={} TimeZone={}", rec["fields"]["AllDay"]["value"], rec["fields"]["DueDate"]["value"], rec["fields"]["TimeZone"]["value"]);
    r::edit_cloudkit(&http, &root, p.clone(), &reminder, &r::ReminderEdit { due_at: Some(Some(1_792_195_200)), all_day: Some(true), ..Default::default() }).await?;
    println!("all day: {}", show(&zc.lookup_record("Reminders", &guid).await?.ok_or_else(|| anyhow!("gone"))?));
    r::edit_cloudkit(&http, &root, p.clone(), &reminder, &r::ReminderEdit { due_at: Some(Some(1_792_230_000)), ..Default::default() }).await?;
    println!("timed:   {}", show(&zc.lookup_record("Reminders", &guid).await?.ok_or_else(|| anyhow!("gone"))?));
    r::delete_list_cloudkit(&http, &root, p, &list).await?;
    println!("test list deleted");
    Ok(())
}
