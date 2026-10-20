//! Live, on a wallpaper he named as a throwaway: a star rating set through Quince's own
//! set_rating, read back from Apple, then cleared. Leaves the photo as it was.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{self, CONTAINER, ZONE};

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
    let rating = |r: &serde_json::Value| r.pointer("/fields/rating/value").cloned();
    let before = zc
        .lookup_record(ZONE, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("before: {:?}", rating(&before));
    let tag = before.get("recordChangeTag").and_then(|t| t.as_str());
    photos::set_rating(&http, &ck, p.clone(), &asset, tag, 3).await?;
    let now = zc.lookup_record(ZONE, &asset).await?.unwrap();
    println!("set:    {:?}", rating(&now));
    let tag = now.get("recordChangeTag").and_then(|t| t.as_str());
    photos::set_rating(&http, &ck, p.clone(), &asset, tag, 0).await?;
    let back = zc.lookup_record(ZONE, &asset).await?.unwrap();
    println!("clear:  {:?}", rating(&back));
    Ok(())
}
