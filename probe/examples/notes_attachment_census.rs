//! Read-only: what each attachment of one note is, as Apple records it — its type and which
//! of the renderings Notes keeps (preview images, a fallback image or PDF) it has.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    for id in std::env::args().skip(1) {
        let Some(r) = zc.lookup_record("Notes", &id).await? else {
            println!("{id}: not found");
            continue;
        };
        let mut keys: Vec<String> = r["fields"]
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        let uti = r["fields"]["UTIEncrypted"]["value"]
            .as_str()
            .and_then(|v| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, v).ok()
            })
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .or_else(|| r["fields"]["UTI"]["value"].as_str().map(String::from));
        println!("{id}: type {} uti {uti:?}", r["recordType"]);
        for k in [
            "PreviewAppearances",
            "PreviewWidths",
            "PreviewHeights",
            "PreviewScales",
            "PreviewScaleWhenDrawings",
            "Width",
            "Height",
        ] {
            if let Some(v) = r["fields"][k].get("value") {
                println!("   {k} = {v}");
            }
        }
        if let Some(n) = r["fields"]["PreviewImages"]["value"].as_array() {
            println!(
                "   PreviewImages: {} of sizes {:?}",
                n.len(),
                n.iter().map(|x| x["size"].clone()).collect::<Vec<_>>()
            );
        }
        if let Some(f) = r["fields"]["FallbackImage"]["value"].as_object() {
            println!("   FallbackImage size {:?}", f.get("size"));
        }
    }
    Ok(())
}
