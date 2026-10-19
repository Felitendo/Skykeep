//! Live, on one wallpaper he named as throwaway (2026-10-19): favorite then unfavorite, delete
//! (to Recently Deleted) then recover, each read back from Apple. It ends as it began.
use anyhow::{anyhow, Result};
use quince_icloud::photos::Flag;
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
    let mut lib = quince_core::Library::in_memory()?;
    quince_icloud::photos::apply(
        &mut lib,
        quince_icloud::photos::fetch(&http, &root, p.clone(), None).await?,
    )?;
    let photo = lib
        .photos(quince_core::photos::View::Library)?
        .into_iter()
        .find(|x| {
            x.filename
                .as_deref()
                .is_some_and(|n| n.starts_with("wildflower-meadows-wallpaper"))
        })
        .ok_or_else(|| anyhow!("the wallpaper is not in the library"))?;
    println!(
        "throwaway: {:?} {} favorite={} deleted={}",
        photo.filename, photo.id, photo.favorite, photo.deleted
    );
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.photos.cloud", p.clone());
    let read = |name: &'static str| {
        let (zc, id) = (&zc, photo.id.clone());
        async move {
            let r = zc
                .lookup_record("PrimarySync", &id)
                .await?
                .ok_or_else(|| anyhow!("gone"))?;
            anyhow::Ok((
                quince_icloud::cloudkit::field::int(&r, name).unwrap_or(0),
                r["recordChangeTag"].as_str().map(str::to_string),
            ))
        }
    };
    let steps = [
        (Flag::Favorite, true, "isFavorite"),
        (Flag::Favorite, false, "isFavorite"),
        (Flag::Deleted, true, "isDeleted"),
        (Flag::Deleted, false, "isDeleted"),
    ];
    for (flag, on, name) in steps {
        let (_, tag) = read(name).await?;
        quince_icloud::photos::set_flag(
            &http,
            &root,
            p.clone(),
            &photo.id,
            tag.as_deref(),
            flag,
            on,
        )
        .await?;
        let (now, _) = read(name).await?;
        println!(
            "{flag:?} → {on}: Apple holds {name}={now} {}",
            if (now == 1) == on {
                "✓"
            } else {
                "✗ MISMATCH"
            }
        );
    }
    Ok(())
}
