//! Live, a throwaway only: Quince's own shared-albums client — a shared album made, found in the
//! list, deleted, and the list read again to see it gone. No one is invited.
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
        .service_url("sharedstreams")
        .ok_or_else(|| anyhow!("no sharedstreams"))?;
    let params = client.service_params();
    let http = client.http().clone();
    let s = quince_icloud::shared_albums::SharedStreams {
        http: &http,
        root: &root,
        params: &params,
    };
    let before = s.list().await?;
    println!("before: {} shared albums", before.len());
    let guid = s.create("Quince module probe").await?;
    let listed = s.list().await?.into_iter().find(|a| a.guid == guid);
    println!("made and listed: {listed:?}");
    anyhow::ensure!(listed
        .as_ref()
        .is_some_and(|a| a.owned && a.contributions && a.name == "Quince module probe"));
    s.delete(&guid).await?;
    let gone = !s.list().await?.iter().any(|a| a.guid == guid);
    println!("deleted, gone from the list: {gone}");
    anyhow::ensure!(gone);
    Ok(())
}
