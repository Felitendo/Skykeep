//! Live: the app's push code (quince_icloud::push) parked on Notes + Reminders, printing the
//! moment each push lands, so a change made elsewhere can be timed end to end.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdeviceservice")
        .ok_or_else(|| anyhow!("no ckdevice"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let ch = quince_icloud::push::open(
        &http,
        &root,
        &p,
        "PROBE",
        &["com.apple.notes", "com.apple.reminders"],
    )
    .await?;
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    };
    println!("parked at {:.3}", now());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        let got = quince_icloud::push::wait(&http, &ch, end - std::time::Instant::now()).await?;
        println!("push at {:.3}: {got:?}", now());
    }
    Ok(())
}
