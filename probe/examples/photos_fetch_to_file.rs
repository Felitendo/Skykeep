//! Live, read-only: a photo's whole picture fetched to a file in a folder named on the command
//! line with Quince's own fetch_to_file, and compared with the same link fetched directly.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{fetch_to_file, links_of, CONTAINER, ZONE};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().ok_or_else(|| anyhow!("out dir"))?);
    let id = args.next().ok_or_else(|| anyhow!("asset id"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let http = client.http().clone();
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &ck, CONTAINER, client.service_params());
    let a = zc
        .lookup_record(ZONE, &id)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let m = a
        .pointer("/fields/masterRef/value/recordName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no master"))?;
    let master = zc
        .lookup_record(ZONE, m)
        .await?
        .ok_or_else(|| anyhow!("no master"))?;
    let url = links_of(&master).full.ok_or_else(|| anyhow!("no full"))?;
    let target = out.join("full").join(&id);
    println!("fetched: {}", fetch_to_file(&http, &url, &target).await?);
    let direct = http.get(&url).send().await?.bytes().await?;
    let on_disk = std::fs::read(&target)?;
    println!(
        "{} bytes on disk, identical to a direct fetch: {}",
        on_disk.len(),
        on_disk == direct.as_ref()
    );
    Ok(())
}
