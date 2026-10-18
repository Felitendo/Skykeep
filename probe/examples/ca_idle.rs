//! Live, read-only: poll `/ca/idle` every two seconds and print each change it reports,
//! so a change made elsewhere can be timed from the moment it is made.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
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
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    };
    let mut known: Vec<(String, String)> = Vec::new();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut first = true;
    while std::time::Instant::now() < end {
        let changed = cal.changed(&known, "Asia/Kolkata").await?;
        for (guid, ctag) in &changed {
            known.retain(|(g, _)| g != guid);
            if let Some(ctag) = ctag {
                known.push((guid.clone(), ctag.clone()));
            }
        }
        if !first && !changed.is_empty() {
            println!("change at {:.3}: {changed:?}", now());
        }
        if first {
            println!("watching {} calendars from {:.3}", known.len(), now());
        }
        first = false;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Ok(())
}
