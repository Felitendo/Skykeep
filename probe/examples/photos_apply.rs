//! Read-only: his Photos library walked and applied to an in-memory library, each view's
//! count and the newest few, and how many have links. Nothing is written to Apple.
use anyhow::{anyhow, Result};
use quince_core::photos::View;
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let fetched = quince_icloud::photos::fetch(&http, &root, p, None).await?;
    let mut lib = quince_core::Library::in_memory()?;
    let (changed, links) = quince_icloud::photos::apply(&mut lib, fetched)?;
    println!(
        "applied {changed}; links for {} masters ({} with a thumbnail)",
        links.len(),
        links.values().filter(|l| l.thumb.is_some()).count()
    );
    for v in [
        View::Library,
        View::Favorites,
        View::Videos,
        View::Screenshots,
        View::Recents,
        View::RecentlyDeleted,
    ] {
        println!("{v:?}: {}", lib.photos(v)?.len());
    }
    for ph in lib.photos(View::Library)?.iter().take(3) {
        println!(
            "  {} {:?} {}x{} {:?} taken {}",
            ph.id, ph.kind, ph.width, ph.height, ph.filename, ph.taken_at
        );
    }
    for a in lib.albums()? {
        let inside = lib.album_photos(&a.id)?;
        let names: Vec<String> = inside
            .iter()
            .map(|p| p.filename.clone().unwrap_or_default())
            .collect();
        println!("album {:?}: {} photos {names:?}", a.name, inside.len());
    }
    Ok(())
}
