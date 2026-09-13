//! Throwaway: the whole contact-picture chain against the real account.
use anyhow::{anyhow, Result};

fn kind(b: &[u8]) -> &'static str {
    match b {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, 0xff, ..] => "jpeg",
        _ => "UNDRAWABLE",
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let probe_dir =
        std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let lib_dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow!("pass a library dir"))?,
    );

    let mut client = quince_auth::Client::new(&probe_dir)?;
    if !client.resume().await? {
        return Err(anyhow!("the probe session is gone"));
    }
    let root = client
        .session()
        .service_url("contacts")
        .ok_or_else(|| anyhow!("no contacts service"))?;
    let http = client.http().clone();
    let params = client.service_params();

    let mut library = quince_core::Library::open(&lib_dir)?;
    println!(
        "synced {} contacts",
        quince_icloud::contacts::sync(&http, &root, params.clone(), &mut library).await?
    );

    let people = library.contacts(None)?;
    let want = people.iter().filter(|c| c.photo_url.is_some()).count();
    println!("with a picture: {want} of {}\n", people.len());

    let (mut ok, mut before, mut after) = (0usize, 0usize, 0usize);
    for person in people.iter().filter(|c| c.photo_url.is_some()) {
        let photo = library.contact_photo_source(&person.guid)?.ok_or_else(|| {
            anyhow!(
                "the handler's own lookup found nothing for {}",
                person.display_name()
            )
        })?;
        let r = http.get(&photo.url).query(&params).send().await?;
        let status = r.status();
        let raw = r.bytes().await?.to_vec();
        let crop = photo.crop.as_deref().and_then(quince_media::Crop::parse);
        let drawn = quince_media::avatar(&raw, crop, 256)
            .map_err(|e| anyhow!("{}: {e}", person.display_name()))?;
        let square = image_dims(&drawn);
        println!(
            "  {status} {:>6}b -> {:>6}b  {:<5} {square}  crop {:<20} {}",
            raw.len(),
            drawn.len(),
            kind(&drawn),
            photo.crop.clone().unwrap_or_else(|| "none".into()),
            person.display_name()
        );
        before += raw.len();
        after += drawn.len();
        if status.is_success() && kind(&drawn) != "UNDRAWABLE" && square == "256x256" {
            ok += 1;
        }
    }
    println!(
        "\n{ok} of {want} drawable.  {} KB from Apple -> {} KB served.",
        before / 1024,
        after / 1024
    );
    if ok != want {
        return Err(anyhow!("some pictures did not survive the chain"));
    }
    println!("OK — every picture on the account came through.");
    Ok(())
}

fn image_dims(b: &[u8]) -> String {
    image_size(b).map_or_else(|| "?".into(), |(w, h)| format!("{w}x{h}"))
}
fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.starts_with(b"\x89PNG") && b.len() > 24 {
        return Some((
            u32::from_be_bytes(b[16..20].try_into().ok()?),
            u32::from_be_bytes(b[20..24].try_into().ok()?),
        ));
    }
    // JPEG: walk the segments to the frame header.
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
            return Some((
                u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32,
                u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32,
            ));
        }
        i += 2 + len;
    }
    None
}
