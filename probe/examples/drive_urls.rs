//! Live, read-only: the `urls` Drive's listing carries for files in a folder.
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
    let http = client.http().clone();
    let p = client.service_params();
    let root: serde_json::Value = http
        .post(format!(
            "{}/retrieveItemDetailsInFolders",
            drivews.trim_end_matches('/')
        ))
        .query(&p)
        .json(&serde_json::json!([{ "drivewsid": quince_core::drive::ROOT, "partialData": false }]))
        .send()
        .await?
        .json()
        .await?;
    let downloads = root[0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "Downloads")
        .cloned()
        .unwrap();
    let folder: serde_json::Value = http
        .post(format!(
            "{}/retrieveItemDetailsInFolders",
            drivews.trim_end_matches('/')
        ))
        .query(&p)
        .json(&serde_json::json!([{ "drivewsid": downloads["drivewsid"], "partialData": false }]))
        .send()
        .await?
        .json()
        .await?;
    let docws = client
        .session()
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let ids: Vec<_> = folder[0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == "FILE")
        .map(|i| serde_json::json!({ "document_id": i["docwsid"] }))
        .collect();
    let r = http
        .post(format!(
            "{}/ws/com.apple.CloudDocs/download/batch",
            docws.trim_end_matches('/')
        ))
        .query(&p)
        .json(&ids)
        .send()
        .await?;
    println!("batch {}", r.status());
    let t = r.text().await?;
    let v: serde_json::Value = serde_json::from_str(&t)?;
    for d in v.as_array().unwrap() {
        println!(
            "{} thumb={} keys={:?}",
            d["document_id"],
            d.get("thumbnail_token")
                .map(|x| x["url"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>())
                .unwrap_or("NONE".into()),
            d.as_object().unwrap().keys().collect::<Vec<_>>()
        );
    }
    for i in folder[0]["items"].as_array().unwrap().iter().take(0) {
        let keys: Vec<_> = i.as_object().unwrap().keys().cloned().collect();
        println!(
            "{} {}: keys={:?}\n   urls={}",
            i["name"],
            i["extension"],
            keys,
            i["urls"].to_string().chars().take(300).collect::<String>()
        );
    }
    Ok(())
}
