//! Read-only: what Quince reads of one note attachment (note_media::info), and whether its
//! file can be fetched — as the window asks for it.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let id = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("attachment id"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    println!(
        "info: {:?}",
        quince_icloud::note_media::info(&http, &root, p.clone(), &id).await
    );
    match quince_icloud::note_media::bytes(
        &http,
        &root,
        p,
        &id,
        quince_icloud::note_media::Part::File,
    )
    .await
    {
        Ok(b) => println!("file: {} bytes", b.len()),
        Err(e) => println!("file: {e}"),
    }
    Ok(())
}
