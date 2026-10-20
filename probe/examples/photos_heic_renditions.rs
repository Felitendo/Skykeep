//! Read only: of the HEIC originals, how many Apple also keeps as a full-size JPEG
//! (resJPEGFullRes), and how large their screen-sized one (resJPEGMedRes) is.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let fetched = quince_icloud::photos::fetch(&http, &root, p, None).await?;
    let (mut heic, mut with_full, mut med_small) = (0, 0, 0);
    let mut med_sizes = std::collections::BTreeMap::new();
    for r in &fetched.records {
        if r["recordType"] != "CPLMaster" || r["fields"]["itemType"]["value"] != "public.heic" {
            continue;
        }
        heic += 1;
        let f = &r["fields"];
        if f.get("resJPEGFullRes").is_some() {
            with_full += 1;
        }
        let mw = f["resJPEGMedWidth"]["value"]
            .as_i64()
            .unwrap_or(0)
            .max(f["resJPEGMedHeight"]["value"].as_i64().unwrap_or(0));
        if mw < 1500 {
            med_small += 1;
        }
        *med_sizes.entry(mw).or_insert(0) += 1;
    }
    println!("HEIC masters: {heic}; with a full-size JPEG: {with_full}; screen-sized under 1500 px: {med_small}");
    println!("screen-sized long sides: {med_sizes:?}");
    Ok(())
}
