//! Live, read-only: how reminders with a date store it — AllDay, DueDate, TimeZone.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        client.http(),
        &root,
        "com.apple.reminders",
        client.service_params(),
    );
    let walk = zc.all_records("Reminders", None).await?;
    let mut seen = (0, 0);
    for r in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Reminder" && r["fields"].get("DueDate").is_some())
    {
        let f = &r["fields"];
        let all = f["AllDay"]["value"].as_i64().unwrap_or(-1);
        if (all == 1 && seen.0 < 3) || (all != 1 && seen.1 < 2) {
            if all == 1 {
                seen.0 += 1
            } else {
                seen.1 += 1
            }
            println!(
                "AllDay={} ({}) DueDate={} StartDate={} TimeZone={} keys={:?}",
                f["AllDay"]["value"],
                f["AllDay"]["type"],
                f["DueDate"]["value"],
                f["StartDate"]["value"],
                f["TimeZone"],
                f.as_object()
                    .unwrap()
                    .keys()
                    .filter(|k| k.contains("Date") || k.contains("Time") || k.contains("All"))
                    .collect::<Vec<_>>()
            );
        }
    }
    Ok(())
}
