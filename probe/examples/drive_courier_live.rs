//! Live: does today's push socket (quince_icloud::courier) say when iCloud Drive changes? The
//! older channel this was first asked on is retired. One socket listens to the candidate Drive
//! containers; a throwaway folder ("Quince mount test …") is made in Drive's top level and
//! deleted, and every push is printed with its delay. The throwaway is cleared from Recently
//! Deleted at the end. Prints container names and timings only.
use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let s = client.session();
    let dsid = s.dsid.clone().ok_or_else(|| anyhow!("no dsid"))?;
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let containers = [
        "com.apple.clouddocs",
        "com.apple.CloudDocs",
        "com.apple.icloud.drive",
        "com.apple.notes",
    ];
    let mut id = quince_icloud::courier::Identity::new();
    let mut courier = quince_icloud::courier::Courier::connect(
        quince_icloud::courier::SERVER,
        &mut id,
        &containers,
    )
    .await
    .map_err(|e| anyhow!("connect: {e}"))?;
    courier
        .register(&http, quince_icloud::courier::REGISTRY, &id, &dsid)
        .await
        .map_err(|e| anyhow!("register: {e}"))?;
    println!("listening");
    let listen = |secs| Duration::from_secs(secs);
    async fn drain(c: &mut quince_icloud::courier::Courier, t: Instant, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            match c.next(end - Instant::now()).await {
                Ok(Some(e)) => println!("{:>6.1}s push: {e:?}", t.elapsed().as_secs_f32()),
                Ok(None) => {}
                Err(e) => {
                    println!("lost: {e}");
                    return;
                }
            }
        }
    }
    let name = format!("Quince mount test {}", std::process::id());
    let t = Instant::now();
    let made = quince_icloud::drive::create_folder(
        &http,
        &drivews,
        p.clone(),
        "FOLDER::com.apple.CloudDocs::root",
        &name,
    )
    .await?;
    println!("folder made");
    drain(&mut courier, t, listen(30)).await;
    let t = Instant::now();
    quince_icloud::drive::delete(&http, &drivews, p.clone(), &made).await?;
    println!("folder deleted");
    drain(&mut courier, t, listen(30)).await;
    let trash = quince_icloud::drive::trash(&http, &drivews, p.clone()).await?;
    let mine: Vec<_> = trash
        .into_iter()
        .filter(|i| i.name.starts_with("Quince mount test"))
        .collect();
    println!("throwaways in Recently Deleted: {}", mine.len());
    if !mine.is_empty() {
        quince_icloud::drive::delete_forever_many(&http, &drivews, p, &mine).await?;
    }
    courier.close().await;
    Ok(())
}
