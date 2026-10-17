//! Live, throwaway event only: an alert set, kept through an edit, cleared; then deleted.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("calendar")
        .ok_or_else(|| anyhow!("no calendar"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, p);
    let snap = cal
        .startup("2026-10-01", "2026-10-31", "Asia/Kolkata")
        .await?;
    let home = snap
        .calendars
        .iter()
        .find(|c| c.writable)
        .ok_or_else(|| anyhow!("no writable calendar"))?;
    let mut event = quince_core::calendar::Event {
        guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
        calendar_guid: home.guid.clone(),
        title: "Quince alert test".into(),
        starts_at: 1_793_000_000,
        ends_at: 1_793_003_600,
        timezone: Some("Asia/Kolkata".into()),
        ..Default::default()
    };
    cal.put_event_with_alerts(&event, None, Some(&[15, 1440]))
        .await?;
    println!("set:     {:?}", cal.alerts(&event).await?);
    event.title = "Quince alert test, renamed".into();
    cal.put_event(&event, None).await?;
    println!("edited:  {:?}", cal.alerts(&event).await?);
    cal.put_event_with_alerts(&event, None, Some(&[])).await?;
    println!("cleared: {:?}", cal.alerts(&event).await?);
    cal.delete_event(&event, None).await?;
    println!("deleted");
    Ok(())
}
