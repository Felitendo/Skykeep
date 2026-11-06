//! Read-only: every Shared Library this account owns or joined, read whole through the app's
//! own engine (fetch_in + apply) into a library in memory — nothing written to iCloud or to the
//! app's library. Prints how many photos each holds and the kinds of the first few.
use anyhow::{anyhow, Result};

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
    let places = quince_icloud::photos::shared_libraries(&http, &root, p.clone()).await?;
    println!(
        "{} Shared Librar{}",
        places.len(),
        if places.len() == 1 { "y" } else { "ies" }
    );
    let mut lib = quince_core::Library::in_memory()?;
    for place in &places {
        let mut since = None;
        let mut rounds = 0;
        loop {
            rounds += 1;
            let f = quince_icloud::photos::fetch_in(
                &http,
                &root,
                p.clone(),
                Some(place),
                since.as_deref(),
            )
            .await?;
            let done = f.complete;
            since = f.token.clone();
            quince_icloud::photos::apply(&mut lib, f)?;
            if done || rounds > 50 {
                break;
            }
        }
        let ids = lib.photo_ids()?;
        let in_zone = ids
            .iter()
            .filter(|id| lib.photo_zone(id).ok().flatten().as_deref() == Some(place.zone.as_str()))
            .count();
        println!(
            "{} ({}): {in_zone} photo(s) in {rounds} round(s), token kept: {}",
            &place.zone[..place.zone.len().min(20)],
            if place.owned() { "owned" } else { "joined" },
            lib.sync_token(&quince_icloud::photos::service_of(Some(&place.zone)))?
                .is_some()
        );
        for id in ids.iter().take(3) {
            if let Some(ph) = lib.photo(id)? {
                println!(
                    "  {:?} {}x{} taken {}",
                    ph.kind, ph.width, ph.height, ph.taken_at
                );
            }
        }
    }
    Ok(())
}
