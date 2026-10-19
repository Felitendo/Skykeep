//! Live: one small test JPEG ("Quince upload test") through icloud.com's current upload
//! (photosupload: createUploadUrl → bytes → putAsset → uploadStatus), each answer printed, the
//! new photo looked up, then sent to Recently Deleted.
use anyhow::{anyhow, Result};
use quince_icloud::photos::Flag;
use serde_json::{json, Value};
#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("jpeg path"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let up = client
        .session()
        .service_url("photosupload")
        .ok_or_else(|| anyhow!("no photosupload"))?;
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let dsid = p
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("no dsid"))?;
    let call = |endpoint: &str, body: Value| {
        let (http, url, dsid) = (
            http.clone(),
            format!("{}/photosupload/{endpoint}", up.trim_end_matches('/')),
            dsid.clone(),
        );
        async move {
            let r = http
                .post(url)
                .query(&[("dsid", dsid)])
                .header("content-type", "text/plain;charset=UTF-8")
                .body(body.to_string())
                .send()
                .await?;
            let s = r.status();
            let t = r.text().await?;
            println!("  {s} {}", t.chars().take(400).collect::<String>());
            anyhow::Ok(serde_json::from_str::<Value>(&t).unwrap_or(Value::Null))
        }
    };
    let bytes = std::fs::read(&path)?;
    let name = std::path::Path::new(&path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let id = uuid::Uuid::new_v4().to_string();
    println!("createUploadUrl");
    let urls = call(
        "createUploadUrl",
        json!({"zoneName": "PrimarySync", "assets": {&id: bytes.len()}}),
    )
    .await?;
    let url = urls["uploadUrls"][&id]
        .as_str()
        .ok_or_else(|| anyhow!("no upload url"))?
        .to_string();
    println!("bytes → upload url");
    let r = http
        .post(&url)
        .header("content-type", "text/plain")
        .body(bytes)
        .send()
        .await?;
    let s = r.status();
    let receipt: Value = serde_json::from_str(&r.text().await?).unwrap_or(Value::Null);
    println!(
        "  {s} isDuplicate={} singleFile keys={:?}",
        receipt["isDuplicate"],
        receipt["singleFile"]
            .as_object()
            .map(|o| o.keys().collect::<Vec<_>>())
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    println!("putAsset");
    let put = call("putAsset", json!({"zoneName": "PrimarySync", "localTimeZoneId": "Asia/Kolkata", "importGroup": uuid::Uuid::new_v4().to_string(),
        "files": [{"fileName": name, "lastModDate": now, "timeZoneOffset": -330, "singleFileUploadRequest": receipt["singleFile"]}]})).await?;
    let job = put[0]["uploadJobId"]
        .as_str()
        .ok_or_else(|| anyhow!("no job"))?
        .to_string();
    let asset = put[0]["cplAsset"].as_str().unwrap_or("").to_string();
    for i in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        println!("uploadStatus #{i}");
        let st = call("uploadStatus", json!({"uploadJobIds": [&job]})).await?;
        let s = &st[&job];
        if s.get("errorCode").is_some() || s["progress"].as_f64().unwrap_or(0.0) >= 100.0 {
            break;
        }
    }
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &ck, "com.apple.photos.cloud", p.clone());
    let rec = zc.lookup_record("PrimarySync", &asset).await?;
    println!("asset {asset} in the zone: {}", rec.is_some());
    if let Some(a) = rec {
        quince_icloud::photos::set_flag(
            &http,
            &ck,
            p,
            &asset,
            a["recordChangeTag"].as_str(),
            Flag::Deleted,
            true,
        )
        .await?;
        println!("test photo sent to Recently Deleted");
    }
    Ok(())
}
