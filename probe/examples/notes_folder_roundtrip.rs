//! Live, on the probe account: make a folder, rename it, delete it — each read back from Apple.
//! Leaves nothing behind.
use anyhow::{anyhow, Result};
use serde_json::Value;

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
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let http = client.http().clone();
    let params = client.service_params();
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", params.clone());
    let show = |r: &Option<Value>| {
        r.as_ref()
            .map(|r| r["fields"].to_string())
            .unwrap_or("NONE".into())
    };

    let guid = quince_icloud::notes::create_folder_cloudkit(
        &http,
        &root,
        params.clone(),
        "Quince test",
        None,
    )
    .await?;
    println!(
        "created {guid}: {}",
        show(&zc.lookup_record("Notes", &guid).await?)
    );
    quince_icloud::notes::rename_folder_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        "Quince test renamed",
    )
    .await?;
    println!(
        "renamed: {}",
        show(&zc.lookup_record("Notes", &guid).await?)
    );
    quince_icloud::notes::delete_folder_cloudkit(&http, &root, params.clone(), &[], &[], &guid)
        .await?;
    println!(
        "deleted: {}",
        show(&zc.lookup_record("Notes", &guid).await?)
    );
    Ok(())
}
