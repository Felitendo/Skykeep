//! Read-only: a note's picture as the window gets it — fetched, and made a JPEG when it is
//! HEIC. Prints sizes, kinds and timings only.
use anyhow::{anyhow, Result};
use std::time::Instant;
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
    let info = quince_icloud::note_media::info(&http, &root, p.clone(), &id).await?;
    println!(
        "kind {} size {:?} {:?}x{:?} preview {}",
        info.kind, info.size, info.width, info.height, info.has_preview
    );
    let t = Instant::now();
    let bytes = quince_icloud::note_media::bytes(
        &http,
        &root,
        p,
        &id,
        quince_icloud::note_media::Part::File,
    )
    .await?;
    println!(
        "fetched {} bytes in {} ms, HEIF: {}",
        bytes.len(),
        t.elapsed().as_millis(),
        quince_media::is_heif(&bytes)
    );
    let t = Instant::now();
    let jpeg = quince_media::heic_to_jpeg(&bytes).map_err(|e| anyhow!(e))?;
    let img = image::load_from_memory(&jpeg)?;
    println!(
        "as JPEG: {} bytes, {}x{}, in {} ms",
        jpeg.len(),
        img.width(),
        img.height(),
        t.elapsed().as_millis()
    );
    Ok(())
}
