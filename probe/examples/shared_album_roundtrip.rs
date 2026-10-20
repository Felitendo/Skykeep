//! Live, a throwaway only: a shared album made as icloud.com makes one (sharedstreams
//! createalbum), found in the list, viewed, its assets read, then deleted — and the list read
//! again to see it gone. No one is invited; nothing of his is touched.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

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
        .ok_or_else(|| anyhow!("no sharedstreams"))?;
    let p = client.service_params();
    let dsid = p
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("no dsid"))?;
    let http = client.http().clone();
    let call = |endpoint: &'static str, body: Value| {
        let (http, p, url) = (
            http.clone(),
            p.clone(),
            format!(
                "{}/{dsid}/sharedstreams/{endpoint}",
                root.trim_end_matches('/')
            ),
        );
        async move {
            let r = http
                .post(&url)
                .query(&p)
                .header("content-type", "text/plain;charset=UTF-8")
                .body(body.to_string())
                .send()
                .await?;
            let status = r.status();
            let text = r.text().await?;
            anyhow::Ok((
                status,
                serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)),
            ))
        }
    };
    let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
    let (s, v) = call("createalbum", json!({"albumguid": guid, "attributes": {"name": "Quince probe shared", "allowcontributions": "1"}})).await?;
    println!(
        "createalbum: {s} {}",
        v.to_string().chars().take(400).collect::<String>()
    );
    let (s, v) = call("webgetalbumslist", json!({})).await?;
    let mine = v["albums"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["albumguid"].as_str() == Some(&guid))
        .cloned();
    println!(
        "in the list: {s} {}",
        mine.as_ref().map_or("NOT FOUND".into(), |a| a
            .to_string()
            .chars()
            .take(600)
            .collect::<String>())
    );
    let (s, v) = call("webgetalbumview", json!({"albumguids": [guid]})).await?;
    println!(
        "webgetalbumview: {s} {}",
        v.to_string().chars().take(600).collect::<String>()
    );
    let (s, v) = call(
        "webgetassets",
        json!({"albumguid": guid, "offset": "0", "limit": "50"}),
    )
    .await?;
    println!(
        "webgetassets: {s} {}",
        v.to_string().chars().take(400).collect::<String>()
    );
    let key = mine
        .as_ref()
        .and_then(|a| a["clientOrgKey"].as_str().map(str::to_string));
    let mut body = json!({"albumguid": guid});
    if let Some(k) = key {
        body["clientOrgKey"] = json!(k);
    }
    let (s, v) = call("deletealbum", body).await?;
    println!(
        "deletealbum: {s} {}",
        v.to_string().chars().take(200).collect::<String>()
    );
    let (_, v) = call("webgetalbumslist", json!({})).await?;
    let still = v["albums"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|a| a["albumguid"].as_str() == Some(&guid));
    println!("still listed after delete: {still}");
    Ok(())
}
