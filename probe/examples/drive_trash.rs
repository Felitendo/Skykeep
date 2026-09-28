//! Throwaway: move named probe leftovers in the Drive root to the trash.
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
    let http = client.http().clone();
    let params = client.service_params();
    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut lib = quince_core::Library::open(&store)?;
    let folder = std::env::var("FOLDER").unwrap_or(quince_core::drive::ROOT.into());
    quince_icloud::drive::sync_folder(&http, &drivews, params.clone(), &mut lib, &folder).await?;
    let names: Vec<String> = std::env::args().skip(1).collect();
    for item in lib.drive_children(&folder)? {
        if names
            .iter()
            .any(|n| *n == item.display_name() || *n == item.name)
        {
            quince_icloud::drive::delete(&http, &drivews, params.clone(), &item).await?;
            println!("trashed {}", item.display_name());
        }
    }
    Ok(())
}
