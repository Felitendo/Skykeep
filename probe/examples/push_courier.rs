//! Read-only: the live push channel as icloud.com opens it today (quince_icloud::courier):
//! the web push socket, a token, registration with the push registry, then pushes as they
//! come. Prints statuses and container names only — never a token, cookie or JWT.
//! Usage: push_courier [seconds]
use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};
#[tokio::main]
async fn main() -> Result<()> {
    let wait: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let dsid = client
        .session()
        .dsid
        .clone()
        .ok_or_else(|| anyhow!("no dsid"))?;
    let http = client.http().clone();
    let containers = [
        "com.apple.notes",
        "com.apple.reminders",
        "com.apple.photos.cloud",
    ];
    let mut id = quince_icloud::courier::Identity::new();
    let t = Instant::now();
    let mut courier = quince_icloud::courier::Courier::connect(
        quince_icloud::courier::SERVER,
        "https://www.icloud.com",
        &mut id,
        &containers,
    )
    .await
    .map_err(|e| anyhow!("connect: {e}"))?;
    println!("connected in {:?}", t.elapsed());
    courier
        .register(&http, quince_icloud::courier::REGISTRY, &id, &dsid)
        .await
        .map_err(|e| anyhow!("register: {e}"))?;
    println!(
        "registered in {:?}; waiting {wait}s for pushes…",
        t.elapsed()
    );
    let end = Instant::now() + Duration::from_secs(wait);
    while Instant::now() < end {
        match courier.next(end - Instant::now()).await {
            Ok(Some(e)) => println!("{:>7.1}s push: {e:?}", t.elapsed().as_secs_f32()),
            Ok(None) => {}
            Err(e) => {
                println!("lost: {e}");
                break;
            }
        }
    }
    courier.close().await;
    Ok(())
}
