//! Live, READ-ONLY: walk the Reminders zone once and save every record to a file given as
//! the first argument, for offline study of record shapes (tags, URLs, smart lists, sections).
//! Writes nothing to iCloud. Prints only record-type counts — never titles.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: <out.json>"))?;
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
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for r in &walk.records {
        let t = r["recordType"]
            .as_str()
            .unwrap_or("(tombstone)")
            .to_string();
        *counts.entry(t).or_default() += 1;
    }
    println!("complete: {}", walk.complete);
    for (t, n) in counts {
        println!("{t}: {n}");
    }
    std::fs::write(&out, serde_json::to_vec(&walk.records)?)?;
    Ok(())
}
