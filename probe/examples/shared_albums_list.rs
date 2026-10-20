//! Read-only: his shared albums, as icloud.com asks for them (sharedstreams webgetalbumslist and
//! webgetpendingalbums), printed with what each holds. Nothing is written.
use anyhow::{anyhow, Result};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("sharedstreams")
        .ok_or_else(|| anyhow!("no sharedstreams service"))?;
    let p = client.service_params();
    let dsid = p
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("no dsid"))?;
    let http = client.http().clone();
    for endpoint in ["webgetalbumslist", "webgetpendingalbums"] {
        let url = format!(
            "{}/{dsid}/sharedstreams/{endpoint}",
            root.trim_end_matches('/')
        );
        let r = http
            .post(&url)
            .query(&p)
            .header("content-type", "text/plain;charset=UTF-8")
            .body("{}")
            .send()
            .await?;
        let status = r.status();
        let text = r.text().await?;
        println!("{endpoint}: {status}");
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => {
                let s = serde_json::to_string_pretty(&v)?;
                println!("{}", s.chars().take(2500).collect::<String>());
            }
            Err(_) => println!("{}", text.chars().take(400).collect::<String>()),
        }
    }
    Ok(())
}
