//! Live, read-only: the account's storage, as icloud.com's "X GB available" reads it.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let r = client
        .http()
        .post("https://setup.icloud.com/setup/ws/1/storageUsageInfo")
        .query(&client.service_params())
        .send()
        .await?;
    println!("{}", r.status());
    let t = r.text().await?;
    println!("{}", t.chars().take(900).collect::<String>());
    Ok(())
}
