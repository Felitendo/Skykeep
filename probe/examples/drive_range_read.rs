//! Read-only: whether Apple's download links for iCloud Drive files answer HTTP byte ranges, as
//! a mounted folder reading only the part of a file it needs would ask. The root folder is
//! listed and its first file (or the first in its first folder with any) is read in two
//! pieces, each compared with the same bytes from a whole download. Nothing is written.
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
    let docws = client
        .session()
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    use quince_icloud::drive as d;
    let root = d::fetch_folder(
        &http,
        &drivews,
        p.clone(),
        "FOLDER::com.apple.CloudDocs::root",
    )
    .await?;
    let mut file = root
        .iter()
        .find(|i| i.kind == "FILE" && i.size.unwrap_or(0) > 2000)
        .cloned();
    if file.is_none() {
        for f in root.iter().filter(|i| i.kind == "FOLDER") {
            let inner = d::fetch_folder(&http, &drivews, p.clone(), &f.drivewsid).await?;
            if let Some(x) = inner
                .into_iter()
                .find(|i| i.kind == "FILE" && i.size.unwrap_or(0) > 2000)
            {
                file = Some(x);
                break;
            }
        }
    }
    let file = file.ok_or_else(|| anyhow!("no file to read"))?;
    let size = file.size.unwrap_or(0);
    println!(
        "a file of {size} bytes ({})",
        file.extension.as_deref().unwrap_or("-")
    );
    let url = d::download_url(&http, &docws, p.clone(), &file).await?;
    let whole = http.get(&url).send().await?.bytes().await?;
    println!("whole: {} bytes", whole.len());
    for (from, to) in [(0u64, 99u64), (1000, 1999)] {
        let r = http
            .get(&url)
            .header("Range", format!("bytes={from}-{to}"))
            .send()
            .await?;
        let status = r.status();
        let range = r
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_string();
        let body = r.bytes().await?;
        let same = body.as_ref() == &whole[from as usize..=to as usize];
        println!(
            "bytes {from}-{to}: {status}, content-range {range}, {} bytes, matches: {same}",
            body.len()
        );
    }
    Ok(())
}
