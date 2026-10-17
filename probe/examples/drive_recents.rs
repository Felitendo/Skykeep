//! Read-only: Drive's Recents, as icloud.com asks for them.
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
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let (http, mut p) = (client.http().clone(), client.service_params());
    p.push(("limit".into(), "50".into()));
    p.push(("unified_format".into(), "true".into()));
    for app in ["com.apple.CloudDocs", "_all_"] {
        let url = format!("{}/ws/{app}/list/recents", root.trim_end_matches('/'));
        let r = http.get(&url).query(&p).send().await?;
        let status = r.status();
        let text = r.text().await?;
        println!(
            "{app}: {status} {}",
            text.chars().take(1500).collect::<String>()
        );
    }
    Ok(())
}
