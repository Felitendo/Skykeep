//! Live, throwaway list only: make a list, colour it purple, read it back, delete it.
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
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let guid = quince_icloud::reminders::create_list_cloudkit(
        &http,
        &root,
        p.clone(),
        "Quince colour test",
    )
    .await?;
    println!("made {guid}");
    quince_icloud::reminders::set_list_colour_cloudkit(&http, &root, p.clone(), &guid, "purple")
        .await?;
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let r = zc
        .lookup_record("Reminders", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    println!("Color = {}", r["fields"]["Color"]["value"]);
    let rtm: serde_json::Value = serde_json::from_str(
        r["fields"]["ResolutionTokenMap"]["value"]
            .as_str()
            .unwrap_or("{}"),
    )?;
    println!("color token = {}", rtm["map"]["color"]);
    quince_icloud::reminders::delete_list_cloudkit(&http, &root, p, &guid).await?;
    println!("deleted");
    Ok(())
}
