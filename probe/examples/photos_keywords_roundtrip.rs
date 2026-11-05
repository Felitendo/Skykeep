//! Live, on a wallpaper he named as a throwaway: keywords set through Quince's own
//! set_keywords, read back from Apple, then cleared. First checks that the binary plist Quince
//! writes for ["loupe"] is byte for byte the one Apple wrote on his photo.
use anyhow::{anyhow, Result};
use base64::Engine;
use quince_icloud::photos::{self, CONTAINER};

#[tokio::main]
async fn main() -> Result<()> {
    let apples = "YnBsaXN0MDChAVVsb3VwZQgKAAAAAAAAAQEAAAAAAAAAAgAAAAAAAAAAAAAAAAAAABA=";
    let mut ours = Vec::new();
    plist::to_writer_binary(
        &mut ours,
        &plist::Value::Array(vec![plist::Value::String("loupe".into())]),
    )?;
    println!(
        "same bytes as Apple's: {}",
        base64::engine::general_purpose::STANDARD.encode(&ours) == apples
    );
    let asset = std::env::args().nth(1).ok_or_else(|| anyhow!("asset id"))?;
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
    let words = |r: &serde_json::Value| -> Option<plist::Value> {
        let v = r.pointer("/fields/keywordsEnc/value")?.as_str()?;
        plist::from_bytes(&base64::engine::general_purpose::STANDARD.decode(v).ok()?).ok()
    };
    let before = zc
        .lookup_record(&zone, &asset)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("before: {:?}", words(&before));
    let wanted: Vec<String> = vec!["spring".into(), "quince test ✿".into()];
    photos::set_keywords(&http, &ck, p.clone(), &asset, &[], &wanted).await?;
    let now = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!("set:    {:?}", words(&now));
    photos::set_keywords(&http, &ck, p.clone(), &asset, &wanted, &[]).await?;
    let back = zc.lookup_record(&zone, &asset).await?.unwrap();
    println!("clear:  {:?}", words(&back));
    Ok(())
}
