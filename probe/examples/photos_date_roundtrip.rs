//! Live, on a wallpaper he named as a throwaway: its date moved a day later through Quince's own
//! set_date, read back from Apple, then put back exactly as it was.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{self, CONTAINER};

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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &ck, CONTAINER, p.clone());
    let zone = quince_icloud::photos::library_zone(&zc).await;
    let date = |r: &serde_json::Value| {
        r.pointer("/fields/assetDate/value")
            .and_then(|v| v.as_i64())
    };
    let before = zc
        .lookup_record(&zone, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let was = date(&before).ok_or_else(|| anyhow!("no date"))?;
    println!("before: {was}");
    let tag = before.get("recordChangeTag").and_then(|t| t.as_str());
    photos::set_date(&http, &ck, p.clone(), &asset, tag, was + 86_400_000).await?;
    let now = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!(
        "moved:  {:?} (a day later: {})",
        date(&now),
        date(&now) == Some(was + 86_400_000)
    );
    let tag = now.get("recordChangeTag").and_then(|t| t.as_str());
    photos::set_date(&http, &ck, p.clone(), &asset, tag, was).await?;
    let back = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!(
        "back:   {:?} (as it was: {})",
        date(&back),
        date(&back) == Some(was)
    );
    Ok(())
}
