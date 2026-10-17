//! Live: remove a folder left by an earlier round trip, by record name.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let name = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("record name"))?;
    quince_icloud::notes::delete_folder_cloudkit(
        client.http(),
        &root,
        client.service_params(),
        &[],
        &[],
        &name,
    )
    .await?;
    println!("gone: {name}");
    Ok(())
}
