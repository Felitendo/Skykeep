//! Read-only: download the JPEG attachment's full file and its previews, printing only sizes
//! and content types — the bytes are not kept.
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    use base64::Engine;
    let jpeg = walk
        .records
        .iter()
        .find(|r| {
            r["recordType"] == "Attachment"
                && r["fields"]["UTIEncrypted"]["value"]
                    .as_str()
                    .map(|s| {
                        base64::engine::general_purpose::STANDARD
                            .decode(s)
                            .unwrap_or_default()
                            == b"public.jpeg"
                    })
                    .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("no jpeg"))?;
    let id = jpeg["recordName"].as_str().unwrap();
    // Fresh copies: an asset's address expires.
    let att = zc
        .lookup_record("Notes", id)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!(
        "size {}x{}, previews: widths {:?}",
        att["fields"]["Width"]["value"],
        att["fields"]["Height"]["value"],
        att["fields"]["PreviewWidths"]["value"]
    );
    let get = |url: &str| {
        let url = url.replace("${f}", "file");
        let http = http.clone();
        async move {
            let r = http.get(&url).send().await?;
            let ct = r.headers().get("content-type").cloned();
            let st = r.status();
            let n = r.bytes().await?.len();
            anyhow::Ok(format!("{st} {ct:?} {n} bytes"))
        }
    };
    if let Some(list) = att["fields"]["PreviewImages"]["value"].as_array() {
        for (i, a) in list.iter().enumerate().take(4) {
            if let Some(u) = a["downloadURL"].as_str() {
                println!("preview {i}: {}", get(u).await?);
            } else {
                println!(
                    "preview {i}: no url, keys {:?}",
                    a.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())
                );
            }
        }
    }
    let media = att["fields"]["Media"]["value"]["recordName"]
        .as_str()
        .unwrap();
    let m = zc
        .lookup_record("Notes", media)
        .await?
        .ok_or_else(|| anyhow!("no media"))?;
    match m["fields"]["Asset"]["value"]["downloadURL"].as_str() {
        Some(u) => println!("full file: {}", get(u).await?),
        None => println!(
            "media asset keys {:?}",
            m["fields"]["Asset"]["value"]
                .as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
        ),
    }
    println!(
        "filename: {:?}",
        m["fields"]["FilenameEncrypted"]["value"]
            .as_str()
            .map(|s| String::from_utf8_lossy(
                &base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .unwrap_or_default()
            )
            .into_owned()
            .chars()
            .count())
    );
    Ok(())
}
