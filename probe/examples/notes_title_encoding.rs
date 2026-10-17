//! Read-only: how each note's TitleEncrypted is encoded — plain UTF-8 or gzip — and what it
//! says, beside the text's first line. Shows which notes icloud.com would show garbled.
use anyhow::{anyhow, Result};
use base64::Engine;
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
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let f = &r["fields"];
        let raw = |k: &str| {
            f[k]["value"]
                .as_str()
                .and_then(|v| {
                    base64::engine::general_purpose::STANDARD
                        .decode(v.trim())
                        .ok()
                })
                .unwrap_or_default()
        };
        let t = raw("TitleEncrypted");
        let s = raw("SnippetEncrypted");
        let kind = |b: &[u8]| {
            if b.starts_with(&[0x1f, 0x8b]) {
                "GZIP"
            } else if std::str::from_utf8(b).is_ok() {
                "plain"
            } else {
                "bytes"
            }
        };
        let first = f["TextDataEncrypted"]["value"]
            .as_str()
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default();
        println!(
            "{} title={} snippet={} folder={} | {:?} | first line {:?}",
            r["recordName"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(12)
                .collect::<String>(),
            kind(&t),
            kind(&s),
            f["Folder"]["value"]["recordName"].as_str().unwrap_or("-"),
            String::from_utf8_lossy(&t)
                .chars()
                .take(30)
                .collect::<String>(),
            first
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(30)
                .collect::<String>()
        );
    }
    Ok(())
}
