//! Live, on a wallpaper he named as a throwaway: a title and a caption set through Quince's
//! own set_title_and_caption (captionEnc and extendedDescEnc), read back from Apple, then
//! cleared. Leaves the photo as it was.
use anyhow::{anyhow, Result};
use base64::Engine;
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
    let text = |r: &serde_json::Value, f: &str| {
        r.pointer(&format!("/fields/{f}/value"))
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
    let show = |r: &serde_json::Value| {
        format!(
            "title={:?} caption={:?}",
            text(r, "captionEnc"),
            text(r, "extendedDescEnc")
        )
    };
    let before = zc
        .lookup_record(&zone, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("before: {}", show(&before));
    photos::set_title_and_caption(
        &http,
        &ck,
        p.clone(),
        &asset,
        Some("Spring bloom"),
        Some("Quince test — a caption ✿"),
    )
    .await?;
    let now = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!("set:    {}", show(&now));
    photos::set_title_and_caption(&http, &ck, p.clone(), &asset, Some(""), Some("")).await?;
    let back = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!("clear:  {}", show(&back));
    Ok(())
}
