//! Live, read-only: what Drive's Recently Deleted holds, as Quince reads it.
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
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let rich =
        quince_icloud::drive::trash_items(client.http(), &root, client.service_params()).await?;
    for t in rich.iter().take(3) {
        println!(
            "  {} expires_at={:?} from={:?}",
            t.item.display_name(),
            t.expires_at,
            t.restore_path
        );
    }
    let items = quince_icloud::drive::trash(client.http(), &root, client.service_params()).await?;
    println!("{} in Recently Deleted", items.len());
    for i in items.iter().take(2) {
        println!(
            "  {} | {} | {:?} | etag={:?} parent={:?}",
            i.drivewsid,
            i.display_name(),
            i.size,
            i.etag,
            i.parent_id
        );
    }
    Ok(())
}
