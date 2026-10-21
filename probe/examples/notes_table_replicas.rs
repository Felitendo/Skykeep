//! Read-only: for one note and its table attachments, the base64 fields that name replicas
//! (`ReplicaIDToNotesVersionDataEncrypted`, `ReplicaIDToUserIDEncrypted`) and the table's
//! document, each saved raw (decoded, inflated where compressed) for diffing. Prints only
//! field names, sizes and record tags; no note text.
use anyhow::{anyhow, Result};
use base64::Engine;

fn save(out: &str, name: &str, field: &str, value: &str) {
    let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(value.trim()) else {
        println!("   {field}: not base64");
        return;
    };
    let plain = quince_icloud::ckdocument::debug_inflate(value).unwrap_or_else(|| raw.clone());
    let path = format!("{out}/{name}.{field}.bin");
    std::fs::write(&path, &plain).ok();
    println!(
        "   {field}: {} bytes raw, {} plain{} -> {path}",
        raw.len(),
        plain.len(),
        if raw.starts_with(&[0x1f, 0x8b]) {
            " (gzip)"
        } else {
            ""
        }
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let note = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("note record name"))?;
    let out = std::env::args().nth(2).ok_or_else(|| anyhow!("dump dir"))?;
    std::fs::create_dir_all(&out)?;
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p);
    let walk = zc.all_records("Notes", None).await?;
    for r in walk.records.iter().filter(|r| {
        r["recordName"] == note.as_str()
            || (r["recordType"] == "Attachment"
                && r["fields"]["Note"]["value"]["recordName"] == note.as_str())
    }) {
        let name = r["recordName"].as_str().unwrap_or("x");
        println!(
            "== {} {name} tag={} modified={}",
            r["recordType"], r["recordChangeTag"], r["modified"]["timestamp"]
        );
        for field in [
            "ReplicaIDToNotesVersionDataEncrypted",
            "ReplicaIDToUserIDEncrypted",
            "MergeableDataEncrypted",
        ] {
            if let Some(v) = r["fields"][field]["value"].as_str() {
                save(&out, name, field, v);
            }
        }
    }
    Ok(())
}
