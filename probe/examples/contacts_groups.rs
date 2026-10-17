//! Live, read-only: the Contacts lists (groups) on the account, by name and size.
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
        .service_url("contacts")
        .ok_or_else(|| anyhow!("no contacts"))?;
    let mut q = client.service_params();
    for (k, v) in [
        ("clientVersion", "2.1"),
        ("locale", "en_US"),
        ("order", "last,first"),
    ] {
        q.push((k.into(), v.into()));
    }
    let v: serde_json::Value = client
        .http()
        .get(format!("{}/co/startup", root.trim_end_matches('/')))
        .query(&q)
        .send()
        .await?
        .json()
        .await?;
    let groups = v["groups"].as_array().cloned().unwrap_or_default();
    println!(
        "{} contacts, {} lists; keys {:?}",
        v["contacts"].as_array().map_or(0, Vec::len),
        groups.len(),
        v.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
    for g in groups.iter().take(10) {
        println!(
            "  {:?}: {} members, etag {}",
            g["name"],
            g["contactIds"].as_array().map_or(0, Vec::len),
            g["etag"]
        );
    }
    Ok(())
}
