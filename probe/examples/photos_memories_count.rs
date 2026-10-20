//! Read-only: how many Memories his library has, by Apple's own count (HyperionIndexCountLookup
//! of CPLMemoryNotIsRejectedByCreateDate, as icloud.com counts them), and the first few
//! CPLMemory records' field names. Nothing is written.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, mut q) = (client.http().clone(), client.service_params());
    q.push(("remapEnums".into(), "true".into()));
    let url = format!(
        "{}/database/1/com.apple.photos.cloud/production/private/records/query",
        root.trim_end_matches('/')
    );
    let post = |body: Value| {
        let (http, url, q) = (http.clone(), url.clone(), q.clone());
        async move {
            anyhow::Ok(
                http.post(&url)
                    .query(&q)
                    .header("content-type", "text/plain")
                    .body(body.to_string())
                    .send()
                    .await?
                    .json::<Value>()
                    .await?,
            )
        }
    };
    let ids = [
        "CPLMemoryNotIsRejectedByCreateDate",
        "CPLMemoryIsFavoritedByCreateDate",
    ];
    let v = post(json!({"query":{"recordType":"HyperionIndexCountLookup","filterBy":[
        {"fieldName":"indexCountID","comparator":"IN","fieldValue":{"value":ids,"type":"STRING_LIST"}}]},
        "zoneID":{"zoneName":"PrimarySync"}})).await?;
    for r in v["records"].as_array().into_iter().flatten() {
        println!(
            "count {} = {}",
            r["recordName"], r["fields"]["itemCount"]["value"]
        );
    }
    if v["records"].as_array().is_none() {
        println!("count answer: {v}");
    }
    let v = post(json!({"query":{"recordType":"CPLMemoryNotIsRejectedByCreateDate","filterBy":[
        {"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":"DESCENDING","type":"STRING"}},
        {"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":0,"type":"INT64"}}]},
        "zoneID":{"zoneName":"PrimarySync"},"resultsLimit":5})).await?;
    match v["records"].as_array() {
        Some(rs) => {
            for r in rs {
                let keys: Vec<_> = r["fields"]
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                println!("{} {} {:?}", r["recordType"], r["recordName"], keys);
            }
        }
        None => println!(
            "list answer: {}",
            v.to_string().chars().take(600).collect::<String>()
        ),
    }
    Ok(())
}
