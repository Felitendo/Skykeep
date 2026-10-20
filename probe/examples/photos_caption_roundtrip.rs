//! Live, on a wallpaper he named as a throwaway: a caption set through Quince's own
//! set_caption, read back from Apple, then cleared. Leaves the photo as it was.
use anyhow::{anyhow, Result};
use base64::Engine;
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
    let caption = |r: &serde_json::Value| {
        r.pointer("/fields/captionEnc/value")
            .and_then(|v| v.as_str())
            .map(|v| {
                String::from_utf8_lossy(
                    &base64::engine::general_purpose::STANDARD
                        .decode(v)
                        .unwrap_or_default(),
                )
                .into_owned()
            })
    };
    let before = zc
        .lookup_record(ZONE, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("before: {:?}", caption(&before));
    let tag = before.get("recordChangeTag").and_then(|t| t.as_str());
    let after = photos::set_caption(
        &http,
        &ck,
        p.clone(),
        &asset,
        tag,
        "Spring bloom — Quince test ✿",
    )
    .await?;
    let now = zc.lookup_record(ZONE, &asset).await?.unwrap();
    println!("set:    {:?}", caption(&now));
    let tag = after
        .get("recordChangeTag")
        .and_then(|t| t.as_str())
        .or_else(|| now.get("recordChangeTag").and_then(|t| t.as_str()));
    photos::set_caption(&http, &ck, p.clone(), &asset, tag, "").await?;
    let back = zc.lookup_record(ZONE, &asset).await?.unwrap();
    println!("clear:  {:?}", caption(&back));
    Ok(())
}
