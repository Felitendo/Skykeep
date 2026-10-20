//! Read-only: for a few videos, what Apple keeps — the original and its streaming copies — with
//! their sizes, codecs and whether each is upright (its matrix), and how long the streaming copy
//! takes to fetch — whole, and up to its header and turn, where the player can start (the
//! viewer streams it as it lands). Prints sizes and kinds only.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let ids: Vec<String> = std::env::args().skip(1).collect();
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &ck, "com.apple.photos.cloud", p);
    let tmp = std::env::temp_dir().join(format!("vr-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    for (n, id) in ids.iter().enumerate() {
        let a = zc
            .lookup_record("PrimarySync", id)
            .await?
            .ok_or_else(|| anyhow!("gone"))?;
        let master = a["fields"]["masterRef"]["value"]["recordName"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let m = zc
            .lookup_record("PrimarySync", &master)
            .await?
            .ok_or_else(|| anyhow!("no master"))?;
        let f = &m["fields"];
        println!(
            "video {n}: original {} bytes ({})",
            f["resOriginalFileSize"]["value"], f["itemType"]["value"]
        );
        for key in [
            "resVidMedRes",
            "resVidSmallRes",
            "resVidHDRMedRes",
            "resVidLargeRes",
        ] {
            let Some(url) = f[key]["value"]["downloadURL"].as_str() else {
                continue;
            };
            use futures_util::StreamExt;
            use std::io::Write;
            let t = std::time::Instant::now();
            let path = tmp.join(format!("{n}-{key}"));
            let mut out = std::fs::File::create(&path)?;
            let mut body = http
                .get(url.replace("${f}", "v.mp4"))
                .send()
                .await?
                .bytes_stream();
            let (mut got, mut header) = (0usize, None);
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                out.write_all(&chunk)?;
                out.flush()?;
                got += chunk.len();
                if header.is_none() && quince_media::video_matrix(&path).ok().flatten().is_some() {
                    header = Some((t.elapsed().as_millis(), got));
                }
            }
            let took = t.elapsed().as_millis();
            if let Some((ms, at)) = header {
                println!("   {key}: header and turn in {ms} ms ({at} bytes in)");
            }
            let bytes = std::fs::read(&path)?;
            let codec = ["avc1", "hvc1", "hev1"]
                .into_iter()
                .find(|c| bytes.windows(4).any(|w| w == c.as_bytes()))
                .unwrap_or("?");
            let moov_at = bytes.windows(4).position(|w| w == b"moov");
            let matrix = quince_media::video_matrix(&path)
                .ok()
                .flatten()
                .map(|(_, m)| m);
            println!("   {key}: {} bytes in {took} ms, codec {codec}, moov at {moov_at:?} of {}, matrix {matrix:?}", bytes.len(), bytes.len());
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}
