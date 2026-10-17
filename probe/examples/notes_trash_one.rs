//! Live: move one note, by record name, to Recently Deleted — for a test note left behind.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? { return Err(anyhow!("no session")); }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let guid = std::env::args().nth(1).ok_or_else(|| anyhow!("record name"))?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(client.http(), &root, "com.apple.notes", client.service_params());
    let rec = zc.lookup_record("Notes", &guid).await?.ok_or_else(|| anyhow!("no such note"))?;
    let title = quince_icloud::cloudkit::field::document_text(&rec, "TitleEncrypted").unwrap_or_default();
    if !title.starts_with("Quince ") { return Err(anyhow!("refusing: {title:?} is not a Quince test note")); }
    let note = quince_core::notes::Note { guid: guid.clone(), folder_guid: None, title, snippet: None, body: None, locked: false, pinned: false, created_at: None, modified_at: None, editable: true, etag: rec["recordChangeTag"].as_str().map(String::from), runs: None };
    quince_icloud::notes::delete_cloudkit(client.http(), &root, client.service_params(), &note).await?;
    println!("moved {guid} ({}) to Recently Deleted", note.title);
    Ok(())
}
