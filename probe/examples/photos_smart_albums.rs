//! Read-only: which assets Apple puts in each Media Types smart album, and the fields they
//! carry that could decide it — to mirror Apple's own rule locally. Nothing is written.
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
    let (http, mut p) = (client.http().clone(), client.service_params());
    p.push(("remapEnums".into(), "true".into()));
    let url = format!(
        "{}/database/1/com.apple.photos.cloud/production/private/records/query",
        root.trim_end_matches('/')
    );
    for album in [
        "PANORAMA",
        "LIVE",
        "SCREENSHOT",
        "VIDEO",
        "DEPTH",
        "TIMELAPSE",
        "SLOMO",
        "ANIMATED",
        "EXPOSURE",
    ] {
        // Walk the album rank by rank, as icloud.com pages it, until it runs out.
        let mut all: Vec<Value> = Vec::new();
        let mut rank = 0i64;
        loop {
            let body = json!({"query":{"recordType":"CPLAssetAndMasterInSmartAlbumByAssetDate","filterBy":[
                {"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":"ASCENDING","type":"STRING"}},
                {"fieldName":"smartAlbum","comparator":"EQUALS","fieldValue":{"value":album,"type":"STRING"}},
                {"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":rank,"type":"INT64"}}]},
                "zoneID":{"zoneName":"PrimarySync"},"resultsLimit":100});
            let v: Value = http
                .post(&url)
                .query(&p)
                .header("content-type", "text/plain")
                .body(body.to_string())
                .send()
                .await?
                .json()
                .await?;
            let page: Vec<Value> = v["records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["recordType"] == "CPLAsset")
                .cloned()
                .collect();
            if page.is_empty() || all.len() > 2000 {
                break;
            }
            rank += page.len() as i64;
            all.extend(page);
        }
        let mut subtypes = std::collections::BTreeMap::new();
        for a in &all {
            *subtypes
                .entry(a["fields"]["assetSubtypeV2"]["value"].to_string())
                .or_insert(0) += 1;
        }
        println!(
            "{album}: {} assets; assetSubtypeV2 → {subtypes:?}",
            all.len()
        );
    }
    Ok(())
}
