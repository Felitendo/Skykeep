//! Read-only: how many calendars and events Apple holds in the window a sync keeps — counts only.
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
    let snap = quince_icloud::sync::fetch_calendar(
        client.http(),
        &root,
        client.service_params(),
        "2026-09-01",
        "2027-01-31",
        "Asia/Kolkata",
    )
    .await?;
    println!(
        "calendars={} events={}",
        snap.calendars.len(),
        snap.events.len()
    );
    Ok(())
}
