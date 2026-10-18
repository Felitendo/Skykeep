//! Read-only: the fields a "paper" document attachment (PDF, scan) carries — names, types,
//! and for assets their keys — to find where its file is kept.
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    use base64::Engine;
    let mut shown = 0;
    for a in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment")
    {
        let uti = a["fields"]["UTIEncrypted"]["value"]
            .as_str()
            .map(|s| {
                String::from_utf8_lossy(
                    &base64::engine::general_purpose::STANDARD
                        .decode(s)
                        .unwrap_or_default(),
                )
                .into_owned()
            })
            .unwrap_or_default();
        if !uti.contains("paper") || shown >= 2 {
            continue;
        }
        shown += 1;
        println!("== {uti}");
        let rec = zc
            .lookup_record("Notes", a["recordName"].as_str().unwrap())
            .await?
            .unwrap();
        for (k, v) in rec["fields"].as_object().into_iter().flatten() {
            let extra = match v["type"].as_str() {
                Some("ASSETID") => format!(
                    "{:?}",
                    v["value"]
                        .as_object()
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                ),
                Some("REFERENCE") => format!("-> {}", v["value"]["recordName"]),
                Some("ENCRYPTED_BYTES") => {
                    format!("{} b64 chars", v["value"].as_str().map_or(0, str::len))
                }
                _ => String::new(),
            };
            println!("   {k}: {} {extra}", v["type"]);
        }
    }
    // Records that are neither Note nor Attachment nor Media, by type.
    let mut types = std::collections::BTreeMap::new();
    for r in &walk.records {
        *types
            .entry(r["recordType"].as_str().unwrap_or("?").to_string())
            .or_insert(0) += 1;
    }
    println!("record types: {types:?}");
    Ok(())
}
