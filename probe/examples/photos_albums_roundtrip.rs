//! Live, on a throwaway album: Quince's album writes (quince_icloud::photos) end to end. Makes
//! "Quince probe album", puts two wallpapers in, takes one out, renames it, deletes it; each
//! step read back from Apple. The photos themselves are never touched.
use anyhow::{anyhow, Result};
use base64::Engine;
use quince_icloud::photos::{self, CONTAINER, ZONE};

#[tokio::main]
async fn main() -> Result<()> {
    let assets: Vec<String> = std::env::args().skip(1).collect();
    if assets.len() != 2 {
        return Err(anyhow!("two asset ids"));
    }
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
    let name_of = |r: &serde_json::Value| {
        r.pointer("/fields/albumNameEnc/value")
            .and_then(|v| v.as_str())
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    };

    let album = photos::create_album(&http, &ck, p.clone(), "Quince probe album", None).await?;
    let made = zc
        .lookup_record(ZONE, &album.id)
        .await?
        .ok_or_else(|| anyhow!("album not there"))?;
    println!(
        "created {} name={:?} position={:?}",
        album.id,
        name_of(&made),
        made.pointer("/fields/position/value")
    );

    let added = photos::add_to_album(&http, &ck, p.clone(), &album.id, &assets, None).await?;
    println!("added {added:?}");
    for a in &assets {
        let r = zc
            .lookup_record(ZONE, &format!("{a}-IN-{}", album.id))
            .await?;
        println!(
            "  relation {a}: {}",
            r.map_or("MISSING".into(), |r| r["fields"].to_string())
        );
    }

    photos::remove_from_album(&http, &ck, p.clone(), &album.id, &assets[..1]).await?;
    let gone = zc
        .lookup_record(ZONE, &format!("{}-IN-{}", assets[0], album.id))
        .await?;
    println!(
        "removed first: relation now {}",
        if gone.is_none() {
            "gone"
        } else {
            "STILL THERE"
        }
    );
    let photo = zc.lookup_record(ZONE, &assets[0]).await?;
    println!(
        "  the photo itself: {}",
        photo.map_or("MISSING".into(), |r| format!(
            "isDeleted={:?}",
            r.pointer("/fields/isDeleted/value")
        ))
    );

    let tag = photos::rename_album(
        &http,
        &ck,
        p.clone(),
        &album.id,
        album.change_tag.as_deref(),
        "Quince probe — renamed ✓",
    )
    .await?;
    let renamed = zc
        .lookup_record(ZONE, &album.id)
        .await?
        .ok_or_else(|| anyhow!("album gone"))?;
    println!("renamed: {:?}", name_of(&renamed));

    photos::delete_album(&http, &ck, p.clone(), &album.id, tag.as_deref()).await?;
    let deleted = zc.lookup_record(ZONE, &album.id).await?;
    println!(
        "deleted: isDeleted={:?}",
        deleted.and_then(|r| r.pointer("/fields/isDeleted/value").cloned())
    );
    // What the album's one remaining photo relation becomes once the album is deleted.
    let left = zc
        .lookup_record(ZONE, &format!("{}-IN-{}", assets[1], album.id))
        .await?;
    println!(
        "  remaining relation after delete: {}",
        left.map_or("gone".into(), |r| r["fields"].to_string())
    );
    Ok(())
}
