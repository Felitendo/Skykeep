//! Read-only: an attachment's table document saved inflated to disk, for study offline.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let (id, out) = (
        std::env::args().nth(1).ok_or_else(|| anyhow!("id"))?,
        std::env::args().nth(2).ok_or_else(|| anyhow!("out"))?,
    );
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let rec = zc
        .lookup_record("Notes", &id)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(
        rec["fields"]["MergeableDataEncrypted"]["value"]
            .as_str()
            .unwrap_or(""),
    )?;
    let mut plain = Vec::new();
    use std::io::Read;
    if raw.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut plain)?;
    } else {
        flate2::read::ZlibDecoder::new(&raw[..]).read_to_end(&mut plain)?;
    }
    std::fs::write(&out, &plain)?;
    println!(
        "{} bytes, gzip {}",
        plain.len(),
        raw.starts_with(&[0x1f, 0x8b])
    );
    Ok(())
}
