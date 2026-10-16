//! Throwaway, read-only: every Notes Folder record, every field, as Apple holds it.
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
    let nz = quince_icloud::cloudkit::ZoneClient::new(
        client.http(),
        &root,
        "com.apple.notes",
        client.service_params(),
    );
    let walk = nz.all_records("Notes", None).await?;
    for r in walk
        .records
        .iter()
        .filter(|r| r.get("recordType").and_then(Value::as_str) == Some("Folder"))
    {
        println!("{}", serde_json::to_string_pretty(r)?);
    }
    Ok(())
}
