//! Live: delete for good, through Quince's own set_flag, photos already in Recently Deleted —
//! only the test images this project uploaded. Refuses anything not deleted or not named
//! quince-upload-test.jpg, and reads each back from Apple after.
use anyhow::{anyhow, Result};
use base64::Engine;
use quince_icloud::photos::{self, Flag, CONTAINER};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &ck, CONTAINER, p.clone());
    let zone = quince_icloud::photos::library_zone(&zc).await;
    for asset in std::env::args().skip(1) {
        let a = zc
            .lookup_record(&zone, &asset)
            .await?
            .ok_or_else(|| anyhow!("{asset}: gone"))?;
        let master = a
            .pointer("/fields/masterRef/value/recordName")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let m = zc.lookup_record(&zone, master).await?;
        let name = m
            .as_ref()
            .and_then(|m| {
                m.pointer("/fields/filenameEnc/value")
                    .and_then(|v| v.as_str())
            })
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let deleted = a
            .pointer("/fields/isDeleted/value")
            .and_then(|v| v.as_i64())
            == Some(1);
        if !deleted || name != "quince-upload-test.jpg" {
            println!("{asset}: skipped ({name}, deleted={deleted})");
            continue;
        }
        let tag = a.get("recordChangeTag").and_then(|t| t.as_str());
        photos::set_flag(
            &http,
            &ck,
            p.clone(),
            None,
            &asset,
            tag,
            Flag::Expunged,
            true,
        )
        .await?;
        let after = zc.lookup_record(&zone, &asset).await?;
        println!(
            "{asset} ({name}): after = {}",
            after.map_or("record gone".into(), |r| format!(
                "isExpunged={:?}",
                r.pointer("/fields/isExpunged/value")
            ))
        );
    }
    Ok(())
}
