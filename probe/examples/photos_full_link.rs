//! Live, read-only: which renditions a photo's master offers, and that the "full" one Quince
//! picks for zooming downloads as a displayable image.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{links_of, CONTAINER, ZONE};

#[tokio::main]
async fn main() -> Result<()> {
    let asset = std::env::args().nth(1).ok_or_else(|| anyhow!("asset id"))?;
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
    let a = zc
        .lookup_record(ZONE, &asset)
        .await?
        .ok_or_else(|| anyhow!("no asset"))?;
    let master = a
        .pointer("/fields/masterRef/value/recordName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no master"))?;
    let m = zc
        .lookup_record(ZONE, master)
        .await?
        .ok_or_else(|| anyhow!("no master record"))?;
    let mut keys: Vec<_> = m["fields"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| k.starts_with("res") && k.ends_with("Res"))
        .cloned()
        .collect();
    keys.sort();
    println!(
        "itemType {:?}; renditions {keys:?}",
        m.pointer("/fields/itemType/value")
    );
    let l = links_of(&m);
    let full = l.full.ok_or_else(|| anyhow!("no full link"))?;
    let bytes = http.get(&full).send().await?.bytes().await?;
    println!(
        "full: {} bytes, starts {:02x?}",
        bytes.len(),
        &bytes[..4.min(bytes.len())]
    );
    Ok(())
}
