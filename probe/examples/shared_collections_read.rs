//! Read-only: the account's shared albums through quince_icloud::shared_collections — every
//! album found, its title and expiry, how many photos it holds and whether each has links.
use anyhow::{anyhow, Result};
use quince_icloud::shared_collections::{changed, places, read};

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
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let p = client.service_params();
    let http = client.http().clone();
    let found = places(&http, &root, p.clone()).await?;
    println!("{} shared album(s)", found.len());
    for place in &found {
        let r = read(&http, &root, p.clone(), place).await?;
        println!(
            "{:?} owned={} title={:?} expires={:?} photos={} with_thumbs={} with_originals={}",
            place.database,
            place.owned(),
            r.about.title,
            r.about
                .expires_at
                .and_then(|s| jiff::Timestamp::from_second(s).ok())
                .map(|t| t.to_string()),
            r.photos.len(),
            r.photos
                .iter()
                .filter(|p| r.links.get(&p.master).is_some_and(|l| l.thumb.is_some()))
                .count(),
            r.photos
                .iter()
                .filter(|p| r.links.get(&p.master).is_some_and(|l| l.original.is_some()))
                .count(),
        );
        for p in r.photos.iter().take(3) {
            println!("  {:?} {}x{} {:?}", p.kind, p.width, p.height, p.filename);
        }
        let again = changed(&http, &root, p.clone(), place, r.token.as_deref()).await?;
        println!("  changed since this read: {again}");
    }
    Ok(())
}
