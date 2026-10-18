//! Read-only: the fields an orphaned table attachment carries, with their types.
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
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let rec = zc
        .lookup_record("Notes", "f7e8966f-6b1e-4882-880a-5d7ccf00d5e7")
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    for (k, v) in rec["fields"].as_object().into_iter().flatten() {
        println!("{k}: {}", v["type"]);
    }
    Ok(())
}
