//! Live, throwaway only: one "Quince create-many test" folder, then 5 folders made inside it in
//! one createFolders request — each answered with its own id — then all of it purged.
use anyhow::{anyhow, Result};
use quince_icloud::drive;
const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let dws = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let top = drive::create_folder(&http, &dws, p.clone(), ROOT, "Quince create-many test").await?;
    let names: Vec<String> = ["00", "0a", "1f", "ff", "info"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let t = std::time::Instant::now();
    let made = drive::create_folders(&http, &dws, p.clone(), &top.drivewsid, &names).await?;
    println!(
        "one request, {:?}: {:?}",
        t.elapsed(),
        made.iter()
            .map(|(n, r)| (n.as_str(), r.is_ok()))
            .collect::<Vec<_>>()
    );
    let mut lib = quince_core::Library::in_memory()?;
    drive::sync_folder(&http, &dws, p.clone(), &mut lib, &top.drivewsid).await?;
    let listed: Vec<String> = lib
        .drive_children(&top.drivewsid)?
        .into_iter()
        .map(|i| i.name)
        .collect();
    println!(
        "folder now holds: {listed:?}; ids match: {}",
        made.iter().all(|(_, r)| r.as_ref().is_ok_and(|id| lib
            .drive_item(id)
            .ok()
            .flatten()
            .is_some()))
    );
    drive::delete(&http, &dws, p.clone(), &top).await?;
    let t = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| i.drivewsid == top.drivewsid)
        .collect::<Vec<_>>();
    println!(
        "purged: {:?}",
        drive::delete_forever_many(&http, &dws, p.clone(), &t).await?
    );
    Ok(())
}
