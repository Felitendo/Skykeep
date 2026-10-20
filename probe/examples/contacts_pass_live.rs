//! Live: a Contacts pass reads the book only when it changed. Unchanged, it is answered from
//! /co/changeset alone; a throwaway card made, the next pass reads the book with it; deleted,
//! the next pass reads it without. Prints counts and timings only.
use anyhow::{anyhow, Result};
use quince_core::contacts::{Contact, ContactMethod};
use quince_icloud::contacts::{self, Pass, Tokens};
use std::time::Instant;

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
    let run = |since: Option<Tokens>| {
        let (http, root, p) = (http.clone(), root.clone(), p.clone());
        async move {
            let t = Instant::now();
            let r = contacts::pass(&http, &root, p, since).await?;
            Ok::<_, anyhow::Error>((r, t.elapsed().as_millis()))
        }
    };
    let say = |what: &str, r: &Pass, ms: u128| match r {
        Pass::Read(b, _) => println!("{what}: read the book, {} cards, {ms} ms", b.contacts.len()),
        Pass::Unchanged(_) => println!("{what}: unchanged, nothing read, {ms} ms"),
    };
    let (first, ms) = run(None).await?;
    say("first pass", &first, ms);
    let Pass::Read(book, tokens) = first else {
        return Err(anyhow!("first pass must read"));
    };
    let before = book.contacts.len();
    let (again, ms) = run(Some(tokens.clone())).await?;
    say("nothing changed", &again, ms);
    println!(
        "{} skipped when unchanged",
        if matches!(again, Pass::Unchanged(_)) {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let tokens = match again {
        Pass::Unchanged(t) | Pass::Read(_, t) => t,
    };

    let throwaway = Contact {
        guid: String::new(),
        first_name: Some("Quince mount test".into()),
        middle_name: None,
        last_name: Some(format!("{}", std::process::id())),
        company: None,
        is_company: false,
        birthday: None,
        notes: None,
        photo_url: None,
        photo_signature: None,
        photo_crop: None,
        etag: None,
        methods: vec![ContactMethod {
            kind: "email".into(),
            label: None,
            value: "quince-test@example.com".into(),
        }],
    };
    let guid = contacts::create(&http, &root, p.clone(), &tokens, &throwaway).await?;
    let (after, ms) = run(Some(tokens.clone())).await?;
    say("after a card was made", &after, ms);
    let ok = matches!(&after, Pass::Read(b, _) if b.contacts.len() == before + 1 && b.contacts.iter().any(|c| c.guid == guid));
    println!(
        "{} read again, with the new card",
        if ok { "PASS" } else { "FAIL" }
    );
    let (tokens, made) = match after {
        Pass::Read(b, t) => (t, b.contacts.into_iter().find(|c| c.guid == guid)),
        Pass::Unchanged(t) => (t, None),
    };
    let made = made
        .ok_or_else(|| anyhow!("throwaway not read back — delete it by hand: Quince mount test"))?;
    contacts::delete(&http, &root, p.clone(), &tokens, &made).await?;
    let (gone, ms) = run(Some(tokens)).await?;
    say("after it was deleted", &gone, ms);
    let ok = matches!(&gone, Pass::Read(b, _) if b.contacts.len() == before && !b.contacts.iter().any(|c| c.guid == guid));
    println!(
        "{} read again, without it",
        if ok { "PASS" } else { "FAIL" }
    );
    Ok(())
}
