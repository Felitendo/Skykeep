//! Read-only: the kinds of attachment the account's notes hold, counted by type, and for one
//! image the fields Apple keeps on it (names only) — what showing pictures in a note needs.
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
    let mut kinds = std::collections::BTreeMap::new();
    let mut shown = 0;
    for r in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment" || r["recordType"] == "Media")
    {
        let uti = r["fields"]["UTIEncrypted"]["value"]
            .as_str()
            .map(|s| {
                use base64::Engine;
                String::from_utf8_lossy(
                    &base64::engine::general_purpose::STANDARD
                        .decode(s)
                        .unwrap_or_default(),
                )
                .into_owned()
            })
            .or_else(|| r["fields"]["UTI"]["value"].as_str().map(str::to_string))
            .unwrap_or_else(|| "?".into());
        *kinds
            .entry(format!(
                "{} {}",
                r["recordType"].as_str().unwrap_or(""),
                uti
            ))
            .or_insert(0) += 1;
        if shown < 2
            && (uti.contains("jpeg")
                || uti.contains("png")
                || uti.contains("heic")
                || r["recordType"] == "Media")
        {
            shown += 1;
            println!(
                "{} fields: {:?}",
                r["recordType"],
                r["fields"]
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
            );
        }
    }
    for (k, n) in kinds {
        println!("{n:4}  {k}");
    }
    Ok(())
}
