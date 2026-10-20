//! Read-only: what a Photos pass costs walked whole against read from the last position.
use anyhow::{anyhow, Result};
use std::time::Instant;
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let t = Instant::now();
    let whole = quince_icloud::photos::fetch(&http, &ck, p.clone(), None).await?;
    println!("whole library: {} records in {:.1} s", whole.records.len(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    let since = quince_icloud::photos::fetch(&http, &ck, p, whole.token.as_deref()).await?;
    println!("changes since: {} records in {:.2} s", since.records.len(), t.elapsed().as_secs_f64());
    Ok(())
}
