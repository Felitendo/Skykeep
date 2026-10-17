//! Live, read-only: the kinds of alarm trigger on the account, with one example of each.
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
    let mut seen = std::collections::BTreeMap::new();
    for r in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "AlarmTrigger")
    {
        let kind = r["fields"]["Type"]["value"]
            .as_str()
            .unwrap_or("?")
            .to_string();
        seen.entry(kind).or_insert_with(|| {
            r["fields"]
                .to_string()
                .chars()
                .take(500)
                .collect::<String>()
        });
    }
    let n = walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "AlarmTrigger")
        .count();
    println!("{n} triggers");
    for (k, v) in seen {
        println!("{k}: {v}");
    }
    Ok(())
}
