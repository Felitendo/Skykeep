//! Read-only: the zones others have shared with this account, in Reminders', Notes' and Photos'
//! CloudKit containers (each container's shared database), as names and owners.
use anyhow::{anyhow, Result};
use quince_icloud::cloudkit::{Database, ZoneClient};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let p = client.service_params();
    let http = client.http().clone();
    for container in ["com.apple.reminders", "com.apple.notes", "com.apple.photos.cloud"] {
        let zones = ZoneClient::new(&http, &root, container, p.clone())
            .in_database(Database::Shared)
            .zone_ids()
            .await;
        match zones {
            Ok(z) => println!("{container}: {} shared zone(s) {:?}", z.len(), z.iter().map(|(n, o)| (n.as_str(), o.as_deref().map(|o| &o[..o.len().min(8)]))).collect::<Vec<_>>()),
            Err(e) => println!("{container}: {e}"),
        }
    }
    Ok(())
}
