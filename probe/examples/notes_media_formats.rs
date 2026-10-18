//! Read-only: for every non-table attachment, the format of its file and its preview as read
//! from their first bytes — what the window will be asked to draw.
use anyhow::{anyhow, Result};
use quince_icloud::note_media::{self, Part};
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    let sniff = |b: &[u8]| match b {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, 0xff, ..] => "jpeg",
        [b'%', b'P', b'D', b'F', ..] => "pdf",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "ftyp(heic/mp4/m4a)",
        [b'P', b'K', ..] => "zip",
        _ => "other",
    };
    for a in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment")
    {
        let id = a["recordName"].as_str().unwrap_or("");
        let info = note_media::info(&http, &root, p.clone(), id).await?;
        if info.uti.contains("table") {
            continue;
        }
        let file = note_media::bytes(&http, &root, p.clone(), id, Part::File)
            .await
            .map(|b| sniff(&b).to_string())
            .unwrap_or_else(|e| format!("ERR {e}"));
        let prev = if info.has_preview {
            note_media::bytes(&http, &root, p.clone(), id, Part::Preview)
                .await
                .map(|b| sniff(&b).to_string())
                .unwrap_or_else(|e| format!("ERR {e}"))
        } else {
            "-".into()
        };
        println!(
            "{:10} {:40} file={file:20} preview={prev}  name? {}",
            info.kind,
            info.uti,
            info.filename.is_some()
        );
    }
    Ok(())
}
