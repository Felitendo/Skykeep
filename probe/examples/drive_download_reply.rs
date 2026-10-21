//! Read-only: the shape of Apple's reply when a download is prepared — its field names, and the
//! version markers beside the item's own — for one file in the Drive root. No file is fetched.
use anyhow::{anyhow, Result};
const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";
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
    let (http, params) = (client.http().clone(), client.service_params());
    let items = quince_icloud::drive::fetch_folder(&http, &drivews, params.clone(), ROOT).await?;
    let file = items
        .iter()
        .find(|i| i.kind == "FILE")
        .ok_or_else(|| anyhow!("no file in the root"))?;
    println!("item: etag {:?} size {:?}", file.etag, file.size);
    let id = file.drivewsid.rsplit("::").next().unwrap_or_default();
    let url = format!(
        "{}/ws/com.apple.CloudDocs/download/by_id",
        docws.trim_end_matches('/')
    );
    let mut q = params;
    q.push(("document_id".into(), id.to_string()));
    let body: serde_json::Value = http.get(url).query(&q).send().await?.json().await?;
    let keys = |v: &serde_json::Value| v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>());
    println!("reply keys: {:?}", keys(&body));
    println!("data_token keys: {:?}", keys(&body["data_token"]));
    for k in ["double_etag", "etag", "size"] {
        println!("{k}: {}", body[k]);
    }
    println!("data_token.size: {}", body["data_token"]["size"]);
    // The app's own check: this version is served; one Apple has moved on from is refused.
    let ok = quince_icloud::drive::download_url_of_version(
        &http,
        &docws,
        q[..q.len() - 1].to_vec(),
        file,
    )
    .await;
    let mut stale = file.clone();
    stale.etag = Some("1a::19".into());
    let refused = quince_icloud::drive::download_url_of_version(
        &http,
        &docws,
        q[..q.len() - 1].to_vec(),
        &stale,
    )
    .await;
    println!(
        "same version: {}; stale version refused: {}",
        ok.is_ok(),
        matches!(refused, Err(quince_icloud::Error::Conflict))
    );
    anyhow::ensure!(ok.is_ok() && matches!(refused, Err(quince_icloud::Error::Conflict)));
    Ok(())
}
