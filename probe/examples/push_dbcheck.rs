//! Read-only: the fallback poller's check (quince_icloud::push::database_changed) against each
//! container — seed, then ask again. Prints whether it moved, never the token.
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let http = client.http().clone();
    let params = client.service_params();
    for c in [
        "com.apple.notes",
        "com.apple.reminders",
        "com.apple.photos.cloud",
    ] {
        let t = std::time::Instant::now();
        let (moved, token) =
            quince_icloud::push::database_changed(&http, &root, &params, c, None).await?;
        let seed = t.elapsed();
        let t = std::time::Instant::now();
        let again =
            quince_icloud::push::database_changed(&http, &root, &params, c, token.as_deref())
                .await?;
        println!(
            "{c}: seed moved={moved} token={} in {seed:?}; again moved={} in {:?}",
            token.is_some(),
            again.0,
            t.elapsed()
        );
    }
    Ok(())
}
