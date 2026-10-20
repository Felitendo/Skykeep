//! Read only: one photo's original fetched into a folder, and made a JPEG the app's way.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (master, out) = (&args[1], std::path::PathBuf::from(&args[2]));
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
    let links = quince_icloud::photos::links(&http, &root, p, master)
        .await?
        .ok_or_else(|| anyhow!("no links"))?;
    let url = links.original.ok_or_else(|| anyhow!("no original"))?;
    let heic = out.join("original.heic");
    quince_icloud::photos::fetch_to_file(&http, &url, &heic).await?;
    let bytes = std::fs::read(&heic)?;
    println!(
        "original {} bytes, heif = {}",
        bytes.len(),
        quince_media::is_heif(&bytes)
    );
    let t = std::time::Instant::now();
    let jpg = quince_media::heic_to_jpeg(&bytes).map_err(|e| anyhow!(e))?;
    println!("decoded to JPEG in {:?}, {} bytes", t.elapsed(), jpg.len());
    std::fs::write(out.join("ours.jpg"), jpg)?;
    Ok(())
}
