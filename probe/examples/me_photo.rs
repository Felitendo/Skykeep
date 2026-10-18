//! Read-only: the person's own card's photo — its pixel size, the crop Apple keeps for it,
//! and its EXIF orientation — to place the crop as Apple does.
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
        .service_url("contacts")
        .ok_or_else(|| anyhow!("no contacts"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let book = quince_icloud::contacts::fetch(&http, &root, p.clone()).await?;
    let me = book.me.clone().ok_or_else(|| anyhow!("no me card"))?;
    let card = book
        .contacts
        .iter()
        .find(|c| c.guid == me)
        .ok_or_else(|| anyhow!("me card not found"))?;
    println!("crop: {:?}", card.photo_crop);
    let url = card.photo_url.clone().ok_or_else(|| anyhow!("no photo"))?;
    let bytes = http.get(&url).send().await?.bytes().await?;
    let img = image::load_from_memory(&bytes)?;
    println!("decoded: {}x{}", img.width(), img.height());
    // EXIF orientation tag (0x0112), found by hand in the APP1 segment.
    let pos = bytes.windows(4).position(|w| w == b"Exif");
    println!("has exif: {}", pos.is_some());
    if let Some(i) = pos {
        let t = &bytes[i + 6..];
        let le = &t[0..2] == b"II";
        let rd16 = |o: usize| {
            if le {
                u16::from_le_bytes([t[o], t[o + 1]])
            } else {
                u16::from_be_bytes([t[o], t[o + 1]])
            }
        };
        let rd32 = |o: usize| if le { u32::from_le_bytes([t[o], t[o+1], t[o+2], t[o+3]]) } else { u32::from_be_bytes([t[o], t[o+1], t[o+2], t[o+3]]) } as usize;
        let ifd = rd32(4);
        let n = rd16(ifd) as usize;
        for k in 0..n {
            let e = ifd + 2 + k * 12;
            if rd16(e) == 0x0112 {
                println!("orientation: {}", rd16(e + 8));
            }
        }
    }
    let img2 = image::load_from_memory(&bytes)?;
    std::fs::write(std::env::temp_dir().join("me-photo-raw.jpg"), &bytes)?;
    let _ = img2;
    Ok(())
}
