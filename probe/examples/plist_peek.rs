//! Read-only: what keywordsEnc and locationEnc hold, decoded, for one photo.
use anyhow::{anyhow, Result};
use base64::Engine;
#[tokio::main]
async fn main() -> Result<()> {
    let asset = std::env::args().nth(1).ok_or_else(|| anyhow!("asset id"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        client.http(),
        &ck,
        quince_icloud::photos::CONTAINER,
        client.service_params(),
    );
    let zone = quince_icloud::photos::library_zone(&zc).await;
    let r = zc
        .lookup_record(&zone, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    for f in ["keywordsEnc", "locationEnc", "locationV2Enc"] {
        if let Some(v) = r
            .pointer(&format!("/fields/{f}/value"))
            .and_then(|v| v.as_str())
        {
            let bytes = base64::engine::general_purpose::STANDARD.decode(v)?;
            let value: plist::Value = plist::from_bytes(&bytes)?;
            println!("{f}: {value:?}");
        }
    }
    Ok(())
}
