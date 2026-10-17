//! Live: delete Reminders lists this probe made and left behind — only lists whose name
//! begins "Quince " and ends "test", nothing else.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? { return Err(anyhow!("no session")); }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let walk = zc.all_records("Reminders", None).await?;
    let mut n = 0;
    for r in walk.records.iter().filter(|r| r["recordType"] == "List") {
        if r["fields"]["Deleted"]["value"].as_i64() == Some(1) { continue; }
        let name = r["fields"]["Name"]["value"].as_str().unwrap_or("");
        if name.starts_with("Quince ") && name.ends_with("test") {
            let guid = r["recordName"].as_str().unwrap_or("");
            quince_icloud::reminders::delete_list_cloudkit(&http, &root, p.clone(), guid).await?;
            println!("deleted leftover list {name:?} ({guid})");
            n += 1;
        }
    }
    println!("{n} leftover test list(s) removed");
    Ok(())
}
