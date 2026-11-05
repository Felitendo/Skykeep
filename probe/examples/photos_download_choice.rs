//! Live, read-only: for each of icloud.com's Download Options, the file Quince picks for a photo
//! and a video, downloaded to a folder named on the command line and checked by its first bytes.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{download_choice, Want, CONTAINER};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().ok_or_else(|| anyhow!("out dir"))?);
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
    let zone = quince_icloud::photos::library_zone(&zc).await;
    for id in args {
        let asset = zc
            .lookup_record(&zone, &id)
            .await?
            .ok_or_else(|| anyhow!("gone"))?;
        let m = asset
            .pointer("/fields/masterRef/value/recordName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("no master"))?;
        let master = zc
            .lookup_record(&zone, m)
            .await?
            .ok_or_else(|| anyhow!("no master"))?;
        let video = master
            .pointer("/fields/itemType/value")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.contains("movie") || t.contains("mpeg-4"));
        for want in [Want::Originals, Want::HighestQuality, Want::MostCompatible] {
            let (url, name) =
                download_choice(&asset, &master, want, video).ok_or_else(|| anyhow!("no file"))?;
            let bytes = http
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            std::fs::write(out.join(&name), &bytes)?;
            println!(
                "{want:?}: {name}  {} bytes, starts {:02x?}",
                bytes.len(),
                &bytes[..8.min(bytes.len())]
            );
        }
    }
    Ok(())
}
