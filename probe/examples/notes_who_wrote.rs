//! Read-only: which device last wrote each note — to tell this app's writes from others'.
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
    let walk = zc.all_records("Notes", None).await?;
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let text = r["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .and_then(quince_icloud::ckdocument::stored_text_from_base64)
            .unwrap_or_default();
        println!(
            "{:>4} {:>14} {:?}",
            r["modified"]["deviceID"].as_str().unwrap_or("?"),
            r["modified"]["timestamp"],
            text.lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(30)
                .collect::<String>()
        );
    }
    Ok(())
}
