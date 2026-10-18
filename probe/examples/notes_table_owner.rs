//! Read-only: every table attachment, the note it belongs to, whether that note's text
//! still places it, and its cells.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    for a in walk.records.iter().filter(|r| r["recordType"] == "Attachment") {
        let id = a["recordName"].as_str().unwrap_or("");
        let owner = a["fields"]["Note"]["value"]["recordName"].as_str().unwrap_or("");
        let note = walk.records.iter().find(|r| r["recordName"] == owner);
        let body = note.and_then(|n| n["fields"]["TextDataEncrypted"]["value"].as_str()).unwrap_or("");
        let placed = quince_icloud::ckdocument::runs_from_base64(body).iter().any(|r| r.attachment.as_deref() == Some(id));
        let cells = a["fields"]["MergeableDataEncrypted"]["value"].as_str().and_then(quince_icloud::table::cells_from_base64);
        println!("{id} → note {owner} ({:?}) placed in text: {placed} deleted: {} modified-by {}", quince_icloud::ckdocument::stored_text_from_base64(body).unwrap_or_default().chars().take(20).collect::<String>(), a["fields"]["Deleted"]["value"], a["modified"]["timestamp"]);
        println!("   {:?}", cells.map(|c| c.into_iter().take(3).collect::<Vec<_>>()));
    }
    Ok(())
}
