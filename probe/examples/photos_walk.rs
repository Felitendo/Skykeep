//! Read-only: the Photos zone walked through its change feed — record types, how long, whether
//! assets carry their image links, and the sync token for later passes. Nothing is written.
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.photos.cloud", p);
    let t = std::time::Instant::now();
    let walk = zc.all_records("PrimarySync", None).await?;
    let mut types: BTreeMap<String, u32> = BTreeMap::new();
    let (mut with_thumb, mut deleted, mut hidden) = (0, 0, 0);
    for r in &walk.records {
        *types
            .entry(r["recordType"].as_str().unwrap_or("(gone)").to_string())
            .or_default() += 1;
        let f = &r["fields"];
        if f["resJPEGThumbRes"]["value"]["downloadURL"].is_string() {
            with_thumb += 1;
        }
        if f["isDeleted"]["value"] == 1 {
            deleted += 1;
        }
        if f["isHidden"]["value"] == 1 {
            hidden += 1;
        }
    }
    println!(
        "walked in {:?}: {} records, complete={}, token={}",
        t.elapsed(),
        walk.records.len(),
        walk.complete,
        walk.token.is_some()
    );
    println!("types: {types:?}");
    if let Some(r) = walk
        .records
        .iter()
        .find(|r| r["recordType"] == "CPLContainerRelation")
    {
        println!(
            "relation: {}",
            serde_json::to_string(r)?
                .chars()
                .take(700)
                .collect::<String>()
        );
    }
    println!(
        "masters with a thumbnail link: {with_thumb}; assets deleted: {deleted}; hidden: {hidden}"
    );
    let t = std::time::Instant::now();
    let again = zc.all_records("PrimarySync", walk.token.as_deref()).await?;
    println!(
        "incremental pass from the token: {} records in {:?}",
        again.records.len(),
        t.elapsed()
    );
    Ok(())
}
