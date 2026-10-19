//! Live: one note in Recently Deleted deleted for good ("Delete Now"), by record name, with his
//! say-so (2026-10-19: the damaged "Heli" note). Refused unless the note is in Recently Deleted.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let guid = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("note record name"))?;
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
    let mut lib = quince_core::Library::in_memory()?;
    let fetched = quince_icloud::notes::fetch_cloudkit(&http, &root, p.clone(), None).await?;
    quince_icloud::notes::apply_cloudkit(&mut lib, fetched)?;
    let note = lib.note(&guid)?.ok_or_else(|| anyhow!("no such note"))?;
    println!("note {:?} in folder {:?}", note.title, note.folder_guid);
    if note.folder_guid.as_deref() != Some("TrashFolder-CloudKit") {
        return Err(anyhow!("not in Recently Deleted; left alone"));
    }
    quince_icloud::notes::delete_cloudkit(&http, &root, p, &note).await?;
    println!("deleted for good");
    Ok(())
}
