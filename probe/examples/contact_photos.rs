//! Read-only: how many contacts carry a photo, and whether fetching one works as the app
//! fetches it (session cookies and the service query). Prints counts and statuses only.
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
        .service_url("contacts")
        .ok_or_else(|| anyhow!("no contacts"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let people = quince_icloud::contacts::fetch(&http, &root, p.clone())
        .await?
        .contacts;
    let with: Vec<_> = people.iter().filter(|c| c.photo_url.is_some()).collect();
    println!("{} contacts, {} with a photo", people.len(), with.len());
    for c in with.iter().take(3) {
        let url = c.photo_url.as_deref().unwrap();
        println!("  url host: {:?}", url.split('/').nth(2));
        let r = http.get(url).query(&p).send().await?;
        let st = r.status();
        let ct = r.headers().get("content-type").cloned();
        let n = r.bytes().await.map(|b| b.len()).unwrap_or(0);
        println!("  with query: {st} {ct:?} {n} bytes");
        let r = http.get(url).send().await?;
        println!(
            "  bare: {} {} bytes",
            r.status(),
            r.bytes().await.map(|b| b.len()).unwrap_or(0)
        );
    }
    Ok(())
}
