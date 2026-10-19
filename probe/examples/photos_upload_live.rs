//! Live: one small test JPEG ("Quince upload test") uploaded to his library as icloud.com
//! uploads one, Apple's answer printed, the new photo looked up, then sent to Recently Deleted.
use anyhow::{anyhow, Result};
use quince_icloud::photos::Flag;
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
    let upload = client
        .session()
        .service_url("uploadimagews")
        .ok_or_else(|| anyhow!("no uploadimagews"))?;
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let bytes = std::fs::read(&path)?;
    let name = std::path::Path::new(&path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let mut q: Vec<(String, String)> = p.clone();
    q.push(("filename".into(), name.clone()));
    q.push((
        "lastModDate".into(),
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis())
        .to_string(),
    ));
    q.push(("timezoneOffset".into(), "-330".into()));
    let t = std::time::Instant::now();
    let r = http
        .post(format!("{}/upload", upload.trim_end_matches('/')))
        .query(&q)
        .body(bytes)
        .send()
        .await?;
    let status = r.status();
    let text = r.text().await?;
    println!(
        "upload {name}: {status} in {:?}\n{}",
        t.elapsed(),
        text.chars().take(600).collect::<String>()
    );
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &ck, "com.apple.photos.cloud", p.clone());
    let mut asset = None;
    for rec in v["records"].as_array().into_iter().flatten() {
        let n = rec["recordName"].as_str().unwrap_or("");
        let found = zc.lookup_record("PrimarySync", n).await?;
        println!(
            "{} {n}: in the zone now: {}",
            rec["recordType"],
            found.is_some()
        );
        if rec["recordType"] == "CPLAsset" {
            asset = found;
        }
    }
    if let Some(a) = asset {
        let name = quince_icloud::cloudkit::field::record_name(&a).unwrap_or_default();
        quince_icloud::photos::set_flag(
            &http,
            &ck,
            p,
            &name,
            a["recordChangeTag"].as_str(),
            Flag::Deleted,
            true,
        )
        .await?;
        println!("test photo sent to Recently Deleted");
    }
    Ok(())
}
