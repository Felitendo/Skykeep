//! Live, on a wallpaper he named as a throwaway: Hide then Unhide through Quince's own
//! set_flag, each read back from Apple. Leaves the photo as it was.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{self, Flag, CONTAINER};

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
    let hidden = |r: &serde_json::Value| r.pointer("/fields/isHidden/value").cloned();
    let before = zc
        .lookup_record(&zone, &asset)
        .await?
        .ok_or_else(|| anyhow!("no such photo"))?;
    println!("before: isHidden={:?}", hidden(&before));
    let tag = before
        .get("recordChangeTag")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let after = photos::set_flag(
        &http,
        &ck,
        p.clone(),
        &asset,
        tag.as_deref(),
        Flag::Hidden,
        true,
    )
    .await?;
    let now = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!("hidden: isHidden={:?}", hidden(&now));
    let tag = after
        .get("recordChangeTag")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .or_else(|| {
            now.get("recordChangeTag")
                .and_then(|t| t.as_str())
                .map(str::to_string)
        });
    photos::set_flag(
        &http,
        &ck,
        p.clone(),
        &asset,
        tag.as_deref(),
        Flag::Hidden,
        false,
    )
    .await?;
    let back = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!("unhidden: isHidden={:?}", hidden(&back));
    Ok(())
}
