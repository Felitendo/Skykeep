//! Live, read-only: a Live Photo's moving part downloads at exactly the size its master
//! advertises (`resOriginalVidComplFileSize`) — what the Photos folder lists the .MOV at — and
//! is a QuickTime file. Takes asset ids; prints only sizes and the file's first box.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{links_of, CONTAINER};

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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &ck, CONTAINER, p);
    let zone = quince_icloud::photos::library_zone(&zc).await;
    for (n, asset) in std::env::args().skip(1).enumerate() {
        let a = zc
            .lookup_record(&zone, &asset)
            .await?
            .ok_or_else(|| anyhow!("no asset"))?;
        let master = a
            .pointer("/fields/masterRef/value/recordName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("no master"))?;
        let m = zc
            .lookup_record(&zone, master)
            .await?
            .ok_or_else(|| anyhow!("no master record"))?;
        let said = m
            .pointer("/fields/resOriginalVidComplFileSize/value")
            .and_then(serde_json::Value::as_i64);
        let live = links_of(&m).live.ok_or_else(|| anyhow!("no live link"))?;
        let bytes = http.get(&live).send().await?.bytes().await?;
        println!(
            "live photo {n}: advertised {said:?}, downloaded {}, box {:?}, equal: {}",
            bytes.len(),
            String::from_utf8_lossy(&bytes[4..12.min(bytes.len())]),
            said == i64::try_from(bytes.len()).ok()
        );
    }
    Ok(())
}
