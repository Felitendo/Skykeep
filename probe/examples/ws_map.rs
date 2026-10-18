//! Read-only: the names of the services in Apple's webservices map (hosts only, no tokens).
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    for k in [
        "ckdatabasews",
        "ckdeviceservice",
        "push",
        "pushws",
        "webcourier",
        "ckdeviceservicews",
        "keyvalue",
        "calendar",
        "contacts",
        "drivews",
        "docws",
    ] {
        println!(
            "{k}: {:?}",
            client
                .session()
                .service_url(k)
                .map(|u| u.split('?').next().unwrap_or("").to_string())
        );
    }
    Ok(())
}
