//! Live, throwaways only: a folder made through Quince's own create_album (albumType 3), an
//! album made inside it (parentId), each read back; the folder renamed; then the folder deleted
//! as icloud.com deletes one (isDeleted on it alone) — and what that does to the album inside,
//! read back. Whatever is left is deleted. No photo is touched.
use anyhow::{anyhow, Result};
use base64::Engine;
use quince_icloud::photos::{self, CONTAINER};

#[tokio::main]
async fn main() -> Result<()> {
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
    let show = |r: &serde_json::Value| {
        let name = r
            .pointer("/fields/albumNameEnc/value")
            .and_then(|v| v.as_str())
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        format!(
            "name={name:?} type={:?} parent={:?} deleted={:?}",
            r.pointer("/fields/albumType/value"),
            r.pointer("/fields/parentId/value"),
            r.pointer("/fields/isDeleted/value")
        )
    };
    let folder = photos::create_album(
        &http,
        &ck,
        p.clone(),
        "Quince probe folder",
        None,
        None,
        true,
    )
    .await?;
    println!(
        "folder: {}",
        show(&zc.lookup_record(&zone, &folder.id).await?.unwrap())
    );
    let inner = photos::create_album(
        &http,
        &ck,
        p.clone(),
        "Quince probe inner",
        None,
        Some(&folder.id),
        false,
    )
    .await?;
    println!(
        "inner:  {}",
        show(&zc.lookup_record(&zone, &inner.id).await?.unwrap())
    );
    let tag = photos::rename_album(
        &http,
        &ck,
        p.clone(),
        &folder.id,
        folder.change_tag.as_deref(),
        "Quince probe folder ✓",
    )
    .await?;
    println!(
        "renamed: {}",
        show(&zc.lookup_record(&zone, &folder.id).await?.unwrap())
    );
    photos::delete_album(&http, &ck, p.clone(), &folder.id, tag.as_deref()).await?;
    println!(
        "folder after delete: {}",
        show(&zc.lookup_record(&zone, &folder.id).await?.unwrap())
    );
    let inner_now = zc.lookup_record(&zone, &inner.id).await?.unwrap();
    println!("inner after the folder's delete: {}", show(&inner_now));
    if inner_now
        .pointer("/fields/isDeleted/value")
        .and_then(|v| v.as_i64())
        != Some(1)
    {
        let t = inner_now.get("recordChangeTag").and_then(|x| x.as_str());
        photos::delete_album(&http, &ck, p.clone(), &inner.id, t).await?;
        println!(
            "inner deleted too: {}",
            show(&zc.lookup_record(&zone, &inner.id).await?.unwrap())
        );
    }
    Ok(())
}
