//! Live, throwaway note only: a note made here is filed as the phone files one — both
//! `Folders` and `Folder` — then sent to Recently Deleted.
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
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        p.clone(),
        None,
        "Quince filing test\nsafe to delete",
        None,
    )
    .await?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let rec = zc
        .lookup_record("Notes", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("Folders = {}", rec["fields"]["Folders"]["value"]);
    println!(
        "Folder  = {}",
        rec["fields"]["Folder"]["value"]["recordName"]
    );
    let note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title: "Quince filing test".into(),
        snippet: None,
        body: None,
        locked: false,
        pinned: false,
        created_at: None,
        modified_at: None,
        editable: true,
        etag: rec["recordChangeTag"].as_str().map(String::from),
        runs: None,
    };
    quince_icloud::notes::delete_cloudkit(&http, &root, p, &note).await?;
    println!("sent to Recently Deleted");
    Ok(())
}
