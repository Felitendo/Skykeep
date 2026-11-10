//! Test cards for the Android contacts sync, only ever cards whose first name starts with
//! "Skykeep Test" (never another):
//!
//!   cargo run -p quince-probe --example contacts_test_cards -- peek
//!       each test card: id, names, numbers/emails with their labels, notes
//!   cargo run -p quince-probe --example contacts_test_cards -- notes <id> <text>
//!       that test card's notes set to <text>, as an edit made on another device
//!   cargo run -p quince-probe --example contacts_test_cards -- delete <id>
//!       that test card deleted
use anyhow::{anyhow, bail, Result};
use quince_icloud::contacts as c;

const PREFIX: &str = "Skykeep Test";

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
        .ok_or_else(|| anyhow!("no contacts service"))?;
    let (http, params) = (client.http().clone(), client.service_params());
    let (book, _) = c::fetch_book_with_tokens(&http, &root, params.clone()).await?;
    let tests: Vec<_> = book
        .contacts
        .iter()
        .filter(|p| {
            p.first_name
                .as_deref()
                .is_some_and(|f| f.starts_with(PREFIX))
        })
        .collect();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let test = |id: &str| {
        tests
            .iter()
            .find(|p| p.guid == id)
            .copied()
            .ok_or_else(|| anyhow!("{id} is not a test card: refused"))
    };
    match args.first().map(String::as_str) {
        Some("peek") => {
            for p in &tests {
                println!(
                    "{} | {} {} | notes {:?}",
                    p.guid,
                    p.first_name.as_deref().unwrap_or(""),
                    p.last_name.as_deref().unwrap_or(""),
                    p.notes
                );
                for m in &p.methods {
                    println!("    {} {:?} {}", m.kind, m.label, m.value);
                }
            }
            println!("{} test cards", tests.len());
        }
        Some("notes") => {
            let id = args.get(1).ok_or_else(|| anyhow!("which id?"))?;
            let text = args.get(2).ok_or_else(|| anyhow!("what notes?"))?;
            let base = test(id)?.clone();
            let mut edited = base.clone();
            edited.notes = Some(text.clone());
            let ledger = c::Ledger::new();
            let current = c::card(&http, &root, params.clone(), &ledger, id).await?;
            c::update_from(
                &http,
                &root,
                params,
                &ledger,
                &edited,
                current.as_ref(),
                Some(&base),
            )
            .await?;
            println!("notes set on {id}");
        }
        Some("delete") => {
            let id = args.get(1).ok_or_else(|| anyhow!("which id?"))?;
            let card = test(id)?.clone();
            c::delete(&http, &root, params, &c::Ledger::new(), &card).await?;
            println!("deleted {id}");
        }
        _ => bail!("peek, notes <id> <text>, or delete <id>"),
    }
    Ok(())
}
