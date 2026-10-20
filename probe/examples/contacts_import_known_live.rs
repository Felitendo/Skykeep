//! Live, throwaway contact only: "Quince dup test" imported once; the same card offered again
//! is known as already in the book (contacts::already_in, as the app's import now checks) and
//! so never sent; a card with a new number is not. Then the throwaway is deleted.
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
    let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:test;Quince dup;;;\r\nFN:Quince dup test\r\nTEL;type=CELL:+1 555 0199\r\nEND:VCARD\r\n";
    let (book, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    println!("before: known {}", c::already_in(card, &book));
    let (ids, _) = c::import_vcards(&http, &root, p.clone(), &tokens, card).await?;
    let id = ids
        .first()
        .ok_or_else(|| anyhow!("nothing imported"))?
        .clone();
    let (book, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    println!("after import: known {}", c::already_in(card, &book));
    let other = card.replace("0199", "0123");
    println!(
        "same name, new number: known {}",
        c::already_in(&other, &book)
    );
    let mine = book
        .iter()
        .find(|x| x.guid == id)
        .ok_or_else(|| anyhow!("not in the book"))?;
    c::delete(&http, &root, p.clone(), &tokens, mine).await?;
    let (book, _) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    println!(
        "throwaway deleted: {}; book {}",
        !book.iter().any(|x| x.guid == id),
        book.len()
    );
    Ok(())
}
