//! Read-only: whether the account's name and address come back from sign-in (printed
//! masked — enough to see they are there, not the address itself).
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let s = client.session();
    let mask = |v: &Option<String>| {
        v.as_deref().map(|t| {
            format!(
                "{}… ({} chars)",
                t.chars().take(2).collect::<String>(),
                t.chars().count()
            )
        })
    };
    println!(
        "name: {:?}\nemail: {:?}",
        mask(&s.account_name),
        mask(&s.account_email)
    );
    Ok(())
}
