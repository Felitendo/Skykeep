//! Read-only: a drawing's preview images, one per appearance, saved to a folder to look at.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().ok_or_else(|| anyhow!("attachment id"))?;
    let out = std::path::PathBuf::from(args.next().ok_or_else(|| anyhow!("folder"))?);
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p);
    let r = zc
        .lookup_record("Notes", &id)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let looks = r["fields"]["PreviewAppearances"]["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut assets: Vec<(String, String)> = r["fields"]["PreviewImages"]["value"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            Some((
                format!("preview-appearance{}", looks.get(i)?),
                a["downloadURL"].as_str()?.to_string(),
            ))
        })
        .collect();
    if let Some(u) = r["fields"]["FallbackImage"]["value"]["downloadURL"].as_str() {
        assets.push(("fallback".into(), u.to_string()));
    }
    std::fs::create_dir_all(&out)?;
    for (name, url) in assets {
        let bytes = http
            .get(url.replace("${f}", "file"))
            .send()
            .await?
            .bytes()
            .await?;
        let img = image::load_from_memory(&bytes)?;
        let rgba = img.to_rgba8();
        let transparent =
            rgba.pixels().filter(|p| p.0[3] < 255).count() * 100 / rgba.pixels().count().max(1);
        let path = out.join(format!("{name}.png"));
        img.save(&path)?;
        println!(
            "{name}: {}x{}, {transparent}% transparent, kind {:?}",
            img.width(),
            img.height(),
            image::guess_format(&bytes).ok()
        );
    }
    Ok(())
}
