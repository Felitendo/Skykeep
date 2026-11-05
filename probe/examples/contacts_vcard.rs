//! Live, throwaway contact only: a vCard for "Quince vCard test" imported, exported back,
//! then deleted. The my-card id is read, never changed.
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
    let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:test;Quince vCard;;;\r\nFN:Quince vCard test\r\nTEL;type=CELL:+1 555 0100\r\nEND:VCARD\r\n";
    let (ids, failed) = c::import_vcards(&http, &root, p.clone(), &tokens, card).await?;
    println!("imported {ids:?}, {failed} failed");
    let id = ids
        .first()
        .ok_or_else(|| anyhow!("nothing imported"))?
        .clone();
    let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    let out = c::export_vcards(
        &http,
        &root,
        p.clone(),
        &tokens,
        std::slice::from_ref(&id),
        "Quince vCard test.vcf",
    )
    .await?;
    println!(
        "exported {} bytes; has name: {}; has phone: {}",
        out.len(),
        out.contains("Quince vCard"),
        out.contains("555")
    );
    let (people, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    let mine = people
        .iter()
        .find(|x| x.guid == id)
        .ok_or_else(|| anyhow!("not in the book"))?;
    c::delete(&http, &root, p.clone(), &tokens, mine).await?;
    let (people, _) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    println!("deleted: {}", !people.iter().any(|x| x.guid == id));
    Ok(())
}
