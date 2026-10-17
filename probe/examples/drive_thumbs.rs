//! Live, read-only: Apple's thumbnails for the files in Downloads, through Quince's own call.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let drivews = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let docws = client
        .session()
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let mut lib = quince_core::Library::open(
        &std::env::temp_dir().join(format!("qth-{}", uuid::Uuid::new_v4())),
    )?;
    quince_icloud::drive::sync_folder(
        &http,
        &drivews,
        p.clone(),
        &mut lib,
        quince_core::drive::ROOT,
    )
    .await?;
    let downloads = lib
        .drive_children(quince_core::drive::ROOT)?
        .into_iter()
        .find(|i| i.name == "Downloads")
        .ok_or_else(|| anyhow!("no Downloads"))?;
    quince_icloud::drive::sync_folder(&http, &drivews, p.clone(), &mut lib, &downloads.drivewsid)
        .await?;
    let files = lib.drive_children(&downloads.drivewsid)?;
    for (id, bytes) in quince_icloud::drive::thumbnails(&http, &docws, p, &files).await? {
        let name = files
            .iter()
            .find(|f| f.drivewsid == id)
            .map(|f| f.display_name())
            .unwrap_or_default();
        println!(
            "{name}: {} bytes, starts {:02x?}",
            bytes.len(),
            &bytes[..4.min(bytes.len())]
        );
    }
    Ok(())
}
