//! Live: does Apple push a Photos change? A channel parked on com.apple.photos.cloud, then the
//! throwaway wallpaper favorited and unfavorited (he named the wallpapers throwaway,
//! 2026-10-19); prints when each push lands after its write. Ends as it began.
use anyhow::{anyhow, Result};
use quince_icloud::photos::Flag;
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let device = client
        .session()
        .service_url("ckdeviceservice")
        .ok_or_else(|| anyhow!("no ckdevice"))?;
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let ch = quince_icloud::push::open(
        &http,
        &device,
        &p,
        "PROBE-PHOTOS",
        &["com.apple.photos.cloud"],
    )
    .await?;
    let mut lib = quince_core::Library::in_memory()?;
    quince_icloud::photos::apply(
        &mut lib,
        quince_icloud::photos::fetch(&http, &ck, p.clone(), None).await?,
    )?;
    let photo = lib
        .photos(quince_core::photos::View::Library)?
        .into_iter()
        .find(|x| {
            x.filename
                .as_deref()
                .is_some_and(|n| n.starts_with("wildflower-meadows-wallpaper"))
        })
        .ok_or_else(|| anyhow!("no wallpaper"))?;
    for on in [true, false] {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            &http,
            &ck,
            "com.apple.photos.cloud",
            p.clone(),
        );
        let tag = zc
            .lookup_record("PrimarySync", &photo.id)
            .await?
            .and_then(|r| r["recordChangeTag"].as_str().map(str::to_string));
        let t = std::time::Instant::now();
        quince_icloud::photos::set_flag(
            &http,
            &ck,
            p.clone(),
            None,
            &photo.id,
            tag.as_deref(),
            Flag::Favorite,
            on,
        )
        .await?;
        let wrote = t.elapsed();
        let got = quince_icloud::push::wait(&http, &ch, std::time::Duration::from_secs(20)).await?;
        println!(
            "favorite → {on}: written in {wrote:?}; push {got:?} {:?} after the write began",
            t.elapsed()
        );
    }
    Ok(())
}
