//! Live, throwaway only: a "Quince folder delete test" folder with a note in it, deleted as
//! a stale screen would ask — knowing of neither the note nor anything inside — and then: is
//! the note in Recently Deleted, and the folder gone? The note is left in Recently Deleted.
use anyhow::{anyhow, Result};
use quince_icloud::notes;
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
    let folder =
        notes::create_folder_cloudkit(&http, &root, p.clone(), "Quince folder delete test", None)
            .await?;
    let note = notes::create_cloudkit(
        &http,
        &root,
        p.clone(),
        Some(&folder),
        "Quince folder delete test note\nsafe to delete",
        None,
    )
    .await?;
    // As a screen that has not yet seen the note, nor the folder's contents.
    notes::delete_folder_cloudkit(&http, &root, p.clone(), &[], &[], &folder).await?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let n = zc.lookup_record("Notes", &note).await?;
    let f = zc.lookup_record("Notes", &folder).await?;
    println!(
        "note's folder now: {:?}; folder still there: {}",
        n.as_ref()
            .and_then(|r| r["fields"]["Folder"]["value"]["recordName"]
                .as_str()
                .map(str::to_string)),
        f.is_some()
    );
    Ok(())
}
