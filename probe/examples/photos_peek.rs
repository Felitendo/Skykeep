//! Live, read-only: one asset's date fields as Apple holds them.
use anyhow::{anyhow, Result};
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
    let r = zc
        .lookup_record(quince_icloud::photos::ZONE, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    for (k, v) in r["fields"].as_object().unwrap() {
        if k.contains("ate") || k.starts_with("is") {
            println!("{k} = {}", v["value"]);
        }
    }
    Ok(())
}
