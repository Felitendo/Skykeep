//! Read-only: each album's contents, asked as icloud.com asks (CPLContainerRelationLiveByAssetDate
//! with parentId), to see what comes back. Nothing is written.
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
    let (http, p) = (client.http().clone(), client.service_params());
    let params = p.clone();
    let fetched = quince_icloud::photos::fetch(&http, &root, p, None).await?;
    let mut lib = quince_core::Library::in_memory()?;
    quince_icloud::photos::apply(&mut lib, fetched)?;
    for album in lib.albums()? {
        let body = json!({"query":{"recordType":"CPLContainerRelationLiveByAssetDate","filterBy":[
                    {"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":0,"type":"INT64"}},
                    {"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":"ASCENDING","type":"STRING"}},
                    {"fieldName":"parentId","comparator":"EQUALS","fieldValue":{"value":album.id,"type":"STRING"}}]},
            "zoneID":{"zoneName":"PrimarySync"},"resultsLimit":200});
        let mut q = params.clone();
        q.push(("remapEnums".into(), "true".into()));
        let v: Value = http
            .post(format!(
                "{}/database/1/com.apple.photos.cloud/production/private/records/query",
                root.trim_end_matches('/')
            ))
            .query(&q)
            .header("content-type", "text/plain")
            .body(body.to_string())
            .send()
            .await?
            .json()
            .await?;
        let count_body = json!({"query":{"recordType":"HyperionIndexCountLookup","filterBy":[{"fieldName":"indexCountID","comparator":"IN","fieldValue":{"value":[format!("CPLContainerRelationNotDeletedByAssetDate:{}", album.id)],"type":"STRING_LIST"}}]},"zoneID":{"zoneName":"PrimarySync"}});
        let c: Value = http
            .post(format!(
                "{}/database/1/com.apple.photos.cloud/production/private/records/query",
                root.trim_end_matches('/')
            ))
            .query(&q)
            .header("content-type", "text/plain")
            .body(count_body.to_string())
            .send()
            .await?
            .json()
            .await?;
        println!(
            "{:?} count: {}",
            album.name, c["records"][0]["fields"]["itemCount"]
        );
        if v.get("records").is_none() {
            println!(
                "{:?}: {}",
                album.name,
                v.to_string().chars().take(300).collect::<String>()
            );
            continue;
        }
        let records = v["records"].as_array().cloned().unwrap_or_default();
        let kinds: Vec<String> = records
            .iter()
            .map(|r| r["recordType"].as_str().unwrap_or("?").to_string())
            .collect();
        let items: Vec<String> = records
            .iter()
            .filter(|r| r["recordType"] == "CPLContainerRelation")
            .map(|r| {
                r["fields"]["itemId"]["value"]
                    .as_str()
                    .unwrap_or("?")
                    .chars()
                    .take(8)
                    .collect()
            })
            .collect();
        let known = items
            .iter()
            .filter(|i| {
                lib.photos(quince_core::photos::View::Library)
                    .unwrap()
                    .iter()
                    .any(|p| p.id.starts_with(i.as_str()))
            })
            .count();
        println!(
            "{:?} ({}): {} records {:?}; relations {} (in library: {known})",
            album.name,
            album.id,
            records.len(),
            kinds.iter().collect::<std::collections::BTreeSet<_>>(),
            items.len()
        );
        let _ = Value::Null;
    }
    Ok(())
}
