//! Live, throwaway list only: a Contacts list made with one person, renamed, a second added,
//! then deleted — read back from Apple after each. No contact is changed.
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
    use quince_icloud::contacts as c;
    let (people, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    let (a, b) = (people[0].guid.clone(), people[1].guid.clone());
    let show = |label: &str, lists: &[c::ContactList]| {
        let mine: Vec<_> = lists
            .iter()
            .filter(|l| l.name.starts_with("Quince "))
            .map(|l| (l.name.clone(), l.members.len()))
            .collect();
        println!("{label}: {mine:?}");
    };
    let guid = c::create_list(
        &http,
        &root,
        p.clone(),
        &tokens,
        "Quince list test",
        std::slice::from_ref(&a),
    )
    .await?;
    let book = c::fetch(&http, &root, p.clone()).await?;
    show("made", &book.lists);
    let mut list = book
        .lists
        .iter()
        .find(|l| l.guid.eq_ignore_ascii_case(&guid))
        .cloned()
        .ok_or_else(|| anyhow!("not listed"))?;
    let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    list.name = "Quince list test, renamed".into();
    list.members.push(b);
    c::update_list(&http, &root, p.clone(), &tokens, &list).await?;
    show(
        "renamed + added",
        &c::fetch(&http, &root, p.clone()).await?.lists,
    );
    let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    c::delete_list(&http, &root, p.clone(), &tokens, &list.guid).await?;
    show("deleted", &c::fetch(&http, &root, p.clone()).await?.lists);
    Ok(())
}
