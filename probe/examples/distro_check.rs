//! What an installed Skykeep needs from the distribution it is on, checked through the app's
//! own code, with no account and no network (.github/workflows/linux-compat.yml):
//!
//! - `distro_check keyring`: a made-up session saved through `quince_auth::Store::keyring`
//!   (the path sign-in takes) into whatever Secret Service is on the session bus, read back,
//!   then forgotten. Its directory is a fresh temporary one, so its keyring entry is never the
//!   app's own;
//! - `distro_check heic <file>`: the file decoded to a JPEG by `quince_media::heic_to_jpeg`
//!   through the system's libheif, as a HEIC photo is shown — or, run from inside an AppImage's
//!   AppDir (APPDIR set), through the libheif and HEVC decoder it carries, as the app finds them;
//! - `distro_check video <file>`: the file's poster read by `quince_media::poster` (decodebin,
//!   videoconvert, videoflip, appsink), then played to its end through playbin3 — the window's
//!   player — with every frame decoded. Through whichever GStreamer the loader finds: the
//!   system's, or the AppImage's own with its environment (its AppRun hook) set.
use anyhow::{anyhow, bail, Result};
use quince_auth::session::Session;

fn main() -> Result<()> {
    // As the app's main does, before any thread starts.
    quince_media::appimage::use_bundled_heif_plugins();
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("keyring") => keyring(),
        Some("heic") => heic(args.get(2).ok_or_else(|| anyhow!("heic <file>"))?),
        Some("video") => video(args.get(2).ok_or_else(|| anyhow!("video <file>"))?),
        _ => bail!("usage: distro_check keyring | heic <file> | video <file>"),
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

fn video(path: &str) -> Result<()> {
    use gstreamer as gst;
    use gstreamer::prelude::*;
    let file = std::path::Path::new(path).canonicalize()?;
    let poster = quince_media::poster::first_seen_frame(&file)
        .map_err(|e| anyhow!("poster: {e}"))?
        .ok_or_else(|| anyhow!("poster: no frame that shows something"))?;
    let picture = image::load_from_memory_with_format(&poster, image::ImageFormat::Jpeg)?;

    // playbin3 to the end, every frame decoded and handed to a sink that only counts them.
    let uri = gst::glib::filename_to_uri(&file, None).map_err(|e| anyhow!("uri: {e}"))?;
    let frames = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", false)
        .property("signal-handoffs", true)
        .build()
        .map_err(|e| anyhow!("fakesink: {e}"))?;
    let counted = frames.clone();
    sink.connect("handoff", false, move |_| {
        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    });
    let player = gst::ElementFactory::make("playbin3")
        .property("uri", uri.as_str())
        .property("video-sink", &sink)
        .property(
            "audio-sink",
            gst::ElementFactory::make("fakesink")
                .build()
                .map_err(|e| anyhow!("fakesink: {e}"))?,
        )
        .build()
        .map_err(|e| anyhow!("playbin3: {e}"))?;
    player
        .set_state(gst::State::Playing)
        .map_err(|e| anyhow!("play: {e}"))?;
    let bus = player.bus().ok_or_else(|| anyhow!("no bus"))?;
    let ended = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(20),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    let _ = player.set_state(gst::State::Null);
    match ended.as_ref().map(|m| m.view()) {
        Some(gst::MessageView::Eos(_)) => {}
        Some(gst::MessageView::Error(e)) => bail!("playbin3: {} ({:?})", e.error(), e.debug()),
        _ => bail!("playbin3: not finished in 20 s"),
    }
    let frames = frames.load(std::sync::atomic::Ordering::Relaxed);
    if frames == 0 {
        bail!("playbin3: reached the end without a frame");
    }
    println!(
        "video: poster {}x{}, played to the end through playbin3 ({frames} frames)",
        picture.width(),
        picture.height()
    );
    Ok(())
}
