//! Live: put back, as plain text, every note title and snippet this app wrote gzipped —
//! which icloud.com showed as the gzip's bytes. Only those two fields are written, each to
//! exactly the text it already holds; the note itself is not touched. `--apply` to write.
use anyhow::{anyhow, Result};
use base64::Engine;
#[tokio::main]
async fn main() -> Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
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
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut n = 0;
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let f = &r["fields"];
        let mut changes = serde_json::Map::new();
        for key in ["TitleEncrypted", "SnippetEncrypted"] {
            let Some(raw) = f[key]["value"]
                .as_str()
                .and_then(|v| b64.decode(v.trim()).ok())
            else {
                continue;
            };
            if !raw.starts_with(&[0x1f, 0x8b]) {
                continue;
            }
            let mut text = String::new();
            std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&raw[..]), &mut text)?;
            changes.insert(key.into(), serde_json::json!({ "type": "ENCRYPTED_BYTES", "value": quince_icloud::ckdocument::bare_text_to_base64(&text) }));
        }
        if changes.is_empty() {
            continue;
        }
        let name = r["recordName"].as_str().unwrap_or("");
        println!("{name}: {:?}", changes.keys().collect::<Vec<_>>());
        if apply {
            zc.update_record(
                "Notes",
                name,
                "Note",
                r["recordChangeTag"].as_str(),
                serde_json::Value::Object(changes),
            )
            .await?;
        }
        n += 1;
    }
    println!(
        "{n} note(s) {}",
        if apply {
            "repaired"
        } else {
            "to repair (dry run)"
        }
    );
    Ok(())
}
