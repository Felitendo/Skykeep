//! Live, throwaway only: 60 "Quince purge N" folders made in one request inside a test
//! folder, all moved to Recently Deleted, then deleted for good as Delete All sends them (in
//! fifties) — and Recently Deleted read again to see none of them is left.
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
    let top = drive::create_folder(&http, &dws, p.clone(), ROOT, "Quince purge-sixty test").await?;
    let names: Vec<String> = (0..60).map(|n| format!("Quince purge {n}")).collect();
    let made = drive::create_folders(&http, &dws, p.clone(), &top.drivewsid, &names).await?;
    let ids: Vec<String> = made.into_iter().filter_map(|(_, r)| r.ok()).collect();
    println!("made {}", ids.len());
    // All to Recently Deleted in one request, as a person deleting them would.
    let mut lib = quince_core::Library::in_memory()?;
    drive::sync_folder(&http, &dws, p.clone(), &mut lib, &top.drivewsid).await?;
    let items: Vec<_> = lib
        .drive_children(&top.drivewsid)?
        .into_iter()
        .filter(|i| ids.contains(&i.drivewsid))
        .collect();
    let body = serde_json::json!({ "items": items.iter().map(|i| serde_json::json!({ "drivewsid": i.drivewsid, "etag": i.etag, "clientId": i.drivewsid })).collect::<Vec<_>>() });
    let r = http
        .post(format!("{}/moveItemsToTrash", dws.trim_end_matches('/')))
        .query(&p)
        .json(&body)
        .send()
        .await?;
    println!("to trash: {}", r.status());
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let trash: Vec<_> = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| ids.contains(&i.drivewsid))
        .collect();
    println!("{} in Recently Deleted", trash.len());
    let t = std::time::Instant::now();
    let refused = drive::delete_forever_many(&http, &dws, p.clone(), &trash).await?;
    println!(
        "Delete All: {} refused, in {:?}",
        refused.len(),
        t.elapsed()
    );
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let left = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| ids.contains(&i.drivewsid))
        .count();
    println!("left in Recently Deleted 10s later: {left}");
    drive::delete(&http, &dws, p.clone(), &top).await?;
    let t = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| i.drivewsid == top.drivewsid)
        .collect::<Vec<_>>();
    println!(
        "test folder purged: {:?}",
        drive::delete_forever_many(&http, &dws, p.clone(), &t).await?
    );
    Ok(())
}
