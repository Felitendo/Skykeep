//! What an installed Skykeep needs from the distribution it is on, checked through the app's
//! own code, with no account and no network (.github/workflows/linux-compat.yml):
//!
//! - `distro_check keyring`: a made-up session saved through `quince_auth::Store::keyring`
//!   (the path sign-in takes) into whatever Secret Service is on the session bus, read back,
//!   then forgotten. Its directory is a fresh temporary one, so its keyring entry is never the
//!   app's own;
//! - `distro_check heic <file>`: the file decoded to a JPEG by `quince_media::heic_to_jpeg`
//!   through the system's libheif, as a HEIC photo is shown.
use anyhow::{anyhow, bail, Result};
use quince_auth::session::Session;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("keyring") => keyring(),
        Some("heic") => heic(args.get(2).ok_or_else(|| anyhow!("heic <file>"))?),
        _ => bail!("usage: distro_check keyring | heic <file>"),
    }
}

fn keyring() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("skykeep-distro-check-{}", std::process::id()));
    let store = quince_auth::Store::keyring(&dir)?;
    let session = Session {
        client_id: Some("distro-check".into()),
        trust_token: Some("distro-check-not-a-real-token".into()),
        ..Session::default()
    };
    store.save(&session).map_err(|e| anyhow!("save: {e}"))?;
    let back = store.try_load().map_err(|e| anyhow!("load: {e}"))?;
    if back.trust_token != session.trust_token {
        bail!("the keyring gave back {:?}", back.trust_token);
    }
    store.clear().map_err(|e| anyhow!("clear: {e}"))?;
    let gone = store
        .try_load()
        .map_err(|e| anyhow!("load after clear: {e}"))?;
    if gone.trust_token.is_some() {
        bail!("the keyring kept the session after it was cleared");
    }
    std::fs::remove_dir_all(&dir).ok();
    println!("keyring: saved, read back and cleared through the Secret Service");
    Ok(())
}

fn heic(path: &str) -> Result<()> {
    let bytes = std::fs::read(path)?;
    if !quince_media::is_heif(&bytes) {
        bail!("{path} is not a HEIF file");
    }
    let jpeg = quince_media::heic_to_jpeg(&bytes).map_err(|e| anyhow!("decode: {e}"))?;
    let picture = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)?;
    println!(
        "heic: decoded to a {}x{} JPEG ({} bytes)",
        picture.width(),
        picture.height(),
        jpeg.len()
    );
    Ok(())
}
