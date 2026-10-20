//! One-off repair of his own address book, done with his say-so (2026-10-20): a vCard export
//! of it was imported back at 07:07 UTC that day, adding a copy of every contact. A contact is
//! deleted here only when all three hold — Apple's import gave it its id (base64, 48 long),
//! Apple made it inside that twelve-second burst, and an original (a UUID id) with exactly the
//! same names, birthday, notes and methods is still in the book. One is deleted first and the
//! book read again to see the original kept; then the rest. Pass `--go` to delete; without it,
//! it only counts.
use anyhow::{anyhow, Result};
use quince_core::contacts::Contact;

fn made_at(c: &Contact) -> Option<i64> {
    i64::from_str_radix(c.etag.as_deref()?, 36).ok()
}

fn same(a: &Contact, b: &Contact) -> bool {
    let methods = |c: &Contact| {
        let mut m: Vec<_> = c
            .methods
            .iter()
            .map(|m| {
                (
                    m.kind.clone(),
                    m.label.clone().unwrap_or_default().to_lowercase(),
                    m.value.clone(),
                )
            })
            .collect();
        m.sort();
        m
    };
    let t = |s: &Option<String>| s.clone().unwrap_or_default();
    t(&a.first_name) == t(&b.first_name)
        && t(&a.middle_name) == t(&b.middle_name)
        && t(&a.last_name) == t(&b.last_name)
        && t(&a.company) == t(&b.company)
        && t(&a.birthday) == t(&b.birthday)
        && t(&a.notes) == t(&b.notes)
        && methods(a) == methods(b)
}

#[tokio::main]
async fn main() -> Result<()> {
    let go = std::env::args().any(|a| a == "--go");
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
    // 2026-10-20 07:07:00 – 07:07:30 UTC, in ms.
    let (from, to) = (1_792_480_020_000_i64, 1_792_480_050_000_i64);
    let pick = |book: &[Contact]| -> Vec<(Contact, String)> {
        book.iter()
            .filter(|x| x.guid.len() == 48 && made_at(x).is_some_and(|t| (from..=to).contains(&t)))
            .filter_map(|x| {
                book.iter()
                    .find(|o| o.guid.len() == 36 && same(o, x))
                    .map(|o| (x.clone(), o.guid.clone()))
            })
            .collect()
    };
    let (book, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let copies = pick(&book);
    println!(
        "book {}; copies with an exact original: {}",
        book.len(),
        copies.len()
    );
    if !go || copies.is_empty() {
        return Ok(());
    }
    let (first, original) = &copies[0];
    c::delete(&http, &root, p.clone(), &tokens, first).await?;
    let (book2, mut tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let kept = book2.iter().any(|x| &x.guid == original);
    let gone = !book2.iter().any(|x| x.guid == first.guid);
    println!("first: copy gone {gone}, original kept {kept}");
    if !(kept && gone) {
        return Err(anyhow!(
            "stopped: the first delete did not do what it should"
        ));
    }
    let mut done = 1;
    for (copy, original) in pick(&book2) {
        if !book2.iter().any(|x| x.guid == original) {
            continue;
        }
        match c::delete(&http, &root, p.clone(), &tokens, &copy).await {
            // Each delete moves the book's sync token on: read it before the next.
            Ok(()) => {
                done += 1;
                tokens = c::fetch_with_tokens(&http, &root, p.clone()).await?.1;
            }
            Err(e) => {
                eprintln!("{}: {e}; reading the book again", copy.guid);
                tokens = c::fetch_with_tokens(&http, &root, p.clone()).await?.1;
            }
        }
    }
    let (book3, _) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    println!(
        "deleted {done}; book now {}; copies left {}",
        book3.len(),
        pick(&book3).len()
    );
    Ok(())
}
