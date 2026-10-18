//! Read-only: for each kind of non-table attachment, its field names and types, what it
//! references, and the shape of the Media record it leads to (field names, asset keys only).
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
    let uti = |r: &serde_json::Value| {
        r["fields"]["UTIEncrypted"]["value"]
            .as_str()
            .map(|s| {
                String::from_utf8_lossy(
                    &base64::engine::general_purpose::STANDARD
                        .decode(s)
                        .unwrap_or_default(),
                )
                .into_owned()
            })
            .unwrap_or_default()
    };
    let mut seen = std::collections::HashSet::new();
    for a in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment")
    {
        let u = uti(a);
        if u.contains("table") || !seen.insert(u.clone()) {
            continue;
        }
        println!("== {u}");
        for (k, v) in a["fields"].as_object().into_iter().flatten() {
            let r = v["value"]["recordName"]
                .as_str()
                .map(|n| format!(" -> {n}"))
                .unwrap_or_default();
            println!("   {k}: {}{r}", v["type"]);
        }
        if let Some(m) = a["fields"]["Media"]["value"]["recordName"].as_str() {
            if let Some(media) = walk.records.iter().find(|r| r["recordName"] == m) {
                for (k, v) in media["fields"].as_object().into_iter().flatten() {
                    let keys = v["value"]
                        .as_object()
                        .map(|o| o.keys().cloned().collect::<Vec<_>>());
                    println!("   media.{k}: {} {:?}", v["type"], keys);
                }
            }
        }
    }
    Ok(())
}
