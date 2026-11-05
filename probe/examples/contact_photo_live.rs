//! Live, on a throwaway card only: a picture added, its square moved, and the picture taken
//! away, each read back from Apple; the card is deleted at the end. `contact_photo_live <jpeg>`.
//! Prints PASS/FAIL lines only.
use anyhow::{anyhow, Result};
use base64::Engine;
use quince_core::contacts::Contact;
use quince_icloud::contacts as c;

#[tokio::main]
async fn main() -> Result<()> {
    let jpeg = std::fs::read(std::env::args().nth(1).ok_or_else(|| anyhow!("a jpeg"))?)?;
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

    let read = |guid: String| {
        let (http, root, p) = (http.clone(), root.clone(), p.clone());
        async move {
            let card = c::card(&http, &root, p, &c::Ledger::new(), &guid).await?;
            let contact = card.as_ref().and_then(|v| {
                c::parse_cards(&serde_json::json!({ "contacts": [v] }))
                    .pop()
                    .map(|c| c.contact)
            });
            Ok::<_, anyhow::Error>((card, contact))
        }
    };
    let say = |ok: bool, what: &str| println!("{} {what}", if ok { "PASS" } else { "FAIL" });

    let throwaway = Contact {
        guid: String::new(),
        first_name: Some("Alex".into()),
        middle_name: None,
        last_name: Some(format!("Rivera Photo {}", std::process::id())),
        company: None,
        is_company: false,
        birthday: None,
        notes: None,
        photo_url: None,
        photo_signature: None,
        photo_crop: None,
        etag: None,
        methods: vec![],
    };
    let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
    let tokens = c::Ledger::holding(tokens);
    let guid = c::create(&http, &root, p.clone(), &tokens, &throwaway).await?;
    let result = async {
        // A picture added, as the form sends one: the JPEG as a data: URL with its square.
        let (card, base) = read(guid.clone()).await?;
        let base = base.ok_or_else(|| anyhow!("the new card is not at Apple"))?;
        let mut with = base.clone();
        with.photo_url = Some(format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&jpeg)
        ));
        with.photo_crop = Some("0,0,600,600".into());
        let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
        let tokens = c::Ledger::holding(tokens);
        c::update_from(
            &http,
            &root,
            p.clone(),
            &tokens,
            &with,
            card.as_ref(),
            Some(&base),
        )
        .await?;
        let (card, now) = read(guid.clone()).await?;
        let now = now.ok_or_else(|| anyhow!("card gone after the photo"))?;
        let url = now.photo_url.clone().unwrap_or_default();
        say(
            url.starts_with("https://"),
            "photo added: Apple holds it at its own URL",
        );
        say(
            now.photo_crop.as_deref() == Some("0,0,600,600"),
            "photo added: square kept",
        );
        let r = http.get(&url).query(&p).send().await?;
        let ok = r.status().is_success();
        let bytes = r.bytes().await?;
        let drawn = quince_media::avatar(
            &bytes,
            now.photo_crop
                .as_deref()
                .and_then(quince_media::Crop::parse),
            128,
        );
        say(
            ok && drawn.is_ok(),
            &format!("photo added: fetched {} bytes and drawn", bytes.len()),
        );

        // The square moved on the same picture: the rest of Apple's entry stays.
        let mut moved = now.clone();
        moved.photo_crop = Some("100,100,400,400".into());
        let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
        let tokens = c::Ledger::holding(tokens);
        c::update_from(
            &http,
            &root,
            p.clone(),
            &tokens,
            &moved,
            card.as_ref(),
            Some(&now),
        )
        .await?;
        let (card, after) = read(guid.clone()).await?;
        let after = after.ok_or_else(|| anyhow!("card gone after the crop"))?;
        say(
            after.photo_crop.as_deref() == Some("100,100,400,400"),
            "square moved",
        );
        say(
            after.photo_url.is_some(),
            "square moved: picture still there",
        );

        // The picture taken away.
        let mut none = after.clone();
        none.photo_url = None;
        none.photo_crop = None;
        none.photo_signature = None;
        let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
        let tokens = c::Ledger::holding(tokens);
        c::update_from(
            &http,
            &root,
            p.clone(),
            &tokens,
            &none,
            card.as_ref(),
            Some(&after),
        )
        .await?;
        let (_, gone) = read(guid.clone()).await?;
        let gone = gone.ok_or_else(|| anyhow!("card gone after removing the photo"))?;
        say(gone.photo_url.is_none(), "photo removed");
        say(
            gone.first_name.as_deref() == Some("Alex"),
            "name untouched throughout",
        );
        Ok::<_, anyhow::Error>(gone)
    }
    .await;

    // The throwaway goes whatever happened above.
    let (_, last) = read(guid.clone()).await?;
    if let Some(last) = last {
        let (_, tokens) = c::fetch_with_tokens(&http, &root, p.clone()).await?;
        let tokens = c::Ledger::holding(tokens);
        c::delete(&http, &root, p.clone(), &tokens, &last).await?;
    }
    let (card, _) = read(guid).await?;
    say(card.is_none(), "throwaway deleted");
    result.map(|_| ())
}
