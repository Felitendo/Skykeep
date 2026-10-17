//! Live, read-only: the fields Drive's Recently Deleted carries per item.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let drivews = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let v: serde_json::Value = client
        .http()
        .post(format!(
            "{}/retrieveItemDetailsInFolders",
            drivews.trim_end_matches('/')
        ))
        .query(&client.service_params())
        .json(&serde_json::json!([{ "drivewsid": "TRASH_ROOT", "partialData": false }]))
        .send()
        .await?
        .json()
        .await?;
    if let Some(i) = v[0]["items"].as_array().and_then(|a| a.first()) {
        println!(
            "keys: {:?}",
            i.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
        println!(
            "dateExpiration={} dateChanged={} restorePath={}",
            i["dateExpiration"], i["dateChanged"], i["restorePath"]
        );
    }
    Ok(())
}
