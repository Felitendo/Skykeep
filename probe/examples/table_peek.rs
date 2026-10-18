//! Read-only: one attachment's cells as Apple holds them now.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let id = std::env::args().nth(1).ok_or_else(|| anyhow!("id"))?;
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
    let v = rec["fields"]["MergeableDataEncrypted"]["value"]
        .as_str()
        .unwrap_or("");
    println!(
        "{:?}",
        quince_icloud::table::cells_from_base64(v)
            .map(|c| c.into_iter().take(3).collect::<Vec<_>>())
    );
    Ok(())
}
