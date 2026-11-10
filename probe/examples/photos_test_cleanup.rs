//! Live cleanup of one test photo: the asset `<record name>` (as Apple named it when the phone's
//! backup test uploaded it) sent to Recently Deleted and then deleted for good. Refused unless the
//! photo's master is a file named "skykeep-backup-test…": nothing else is ever touched.
//!
//!   cargo run -p quince-probe --example photos_test_cleanup -- <asset record name>
use anyhow::{anyhow, bail, Result};
use quince_icloud::cloudkit::field;
use quince_icloud::photos::Flag;

#[tokio::main]
async fn main() -> Result<()> {
    let asset = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("which asset?"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &ck,
        quince_icloud::photos::CONTAINER,
        p.clone(),
    );
    let zone = quince_icloud::photos::library_zone(&zc).await;
    let a = zc
        .lookup_record(&zone, &asset)
        .await?
        .ok_or_else(|| anyhow!("no such photo"))?;
    let master = a
        .pointer("/fields/masterRef/value/recordName")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("no master"))?;
    let m = zc
        .lookup_record(&zone, master)
        .await?
        .ok_or_else(|| anyhow!("no master record"))?;
    // The file's name as Apple keeps it: base64 in `filenameEnc`.
    let name = m
        .pointer("/fields/filenameEnc/value")
        .and_then(serde_json::Value::as_str)
        .and_then(|b| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(b).ok()
        })
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();
    if !name.starts_with("skykeep-backup-test") {
        bail!("{name:?} is not a test photo: refused");
    }
    for flag in [Flag::Deleted, Flag::Expunged] {
        // Each change on the record as it is now.
        let now = zc
            .lookup_record(&zone, &asset)
            .await?
            .ok_or_else(|| anyhow!("gone"))?;
        let tag = now["recordChangeTag"].as_str().map(str::to_string);
        quince_icloud::photos::set_flag(
            &http,
            &ck,
            p.clone(),
            None,
            &asset,
            tag.as_deref(),
            flag,
            true,
        )
        .await?;
    }
    println!(
        "test photo {name} deleted for good ({})",
        field::record_name(&a).unwrap_or_default()
    );
    Ok(())
}
