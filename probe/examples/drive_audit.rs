//! Throwaway: every folder in Drive, as Apple has it and as this app stored it.
use anyhow::{anyhow, Result};
use std::collections::HashMap;

const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let http = client.http().clone();
    let params = client.service_params();

    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;

    // Walk the whole tree, the way someone clicking through it would.
    let mut queue = vec![ROOT.to_string()];
    let mut seen = 0;
    let mut problems = 0;
    while let Some(folder) = queue.pop() {
        quince_icloud::drive::sync_folder(&http, &root, params.clone(), &mut library, &folder)
            .await?;
        let ours: HashMap<String, quince_core::drive::DriveItem> = library
            .drive_children(&folder)?
            .into_iter()
            .map(|i| (i.drivewsid.clone(), i))
            .collect();

        // Apple's own answer for the same folder, asked again.
        let mut fresh = quince_core::Library::open(&store)?;
        quince_icloud::drive::sync_folder(&http, &root, params.clone(), &mut fresh, &folder)
            .await?;
        let theirs = fresh.drive_children(&folder)?;

        for item in &theirs {
            seen += 1;
            let Some(kept) = ours.get(&item.drivewsid) else {
                println!("MISSING  {}/{}", folder, item.name);
                problems += 1;
                continue;
            };
            let mut says = Vec::new();
            if kept.name != item.name {
                says.push(format!("name {:?} vs {:?}", kept.name, item.name));
            }
            if kept.kind != item.kind {
                says.push(format!("kind {} vs {}", kept.kind, item.kind));
            }
            if kept.size != item.size {
                says.push(format!("size {:?} vs {:?}", kept.size, item.size));
            }
            if kept.parent_id != item.parent_id {
                says.push(format!(
                    "parent {:?} vs {:?}",
                    kept.parent_id, item.parent_id
                ));
            }
            if !says.is_empty() {
                println!("DIFFERS  {:<24} {}", item.name, says.join("; "));
                problems += 1;
            }
            if item.kind == "FOLDER" || item.kind == "APP_LIBRARY" {
                queue.push(item.drivewsid.clone());
            }
        }
    }
    println!("\n{seen} items compared, {problems} disagreement(s)");
    Ok(())
}
