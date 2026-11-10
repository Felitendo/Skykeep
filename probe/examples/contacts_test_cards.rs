//! Test cards for the Android contacts sync, only ever cards whose first name starts with
//! "Skykeep Test" (never another):
//!
//!   cargo run -p quince-probe --example contacts_test_cards -- peek
//!       each test card: id, names, numbers/emails with their labels, notes
//!   cargo run -p quince-probe --example contacts_test_cards -- notes <id> <text>
//!       that test card's notes set to <text>, as an edit made on another device
//!   cargo run -p quince-probe --example contacts_test_cards -- delete <id>
//!       that test card deleted
//!   cargo run -p quince-probe --example contacts_test_cards -- create
//!       a new test card, "Skykeep Test Photo", with a picture (a blue square); prints its id
//!   cargo run -p quince-probe --example contacts_test_cards -- photo <id> <red|green|blue>
//!       that test card's picture replaced with a square of that colour, as another device would
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
                    "{} | {} {} | notes {:?} | picture {:?}",
                    p.guid,
                    p.first_name.as_deref().unwrap_or(""),
                    p.last_name.as_deref().unwrap_or(""),
                    p.notes,
                    p.photo_url
                        .as_deref()
                        .map(|u| &u[u.len().saturating_sub(16)..]),
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
        Some("create") => {
            let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
            let card = quince_core::contacts::Contact {
                guid: guid.clone(),
                first_name: Some(format!("{PREFIX} Photo")),
                middle_name: None,
                photo_url: Some(square("blue")?),
                photo_signature: None,
                photo_crop: None,
                last_name: None,
                company: None,
                is_company: false,
                birthday: None,
                notes: None,
                etag: None,
                methods: Vec::new(),
            };
            c::create_named(&http, &root, params, &c::Ledger::new(), &guid, &card).await?;
            println!("created {guid}");
        }
        Some("photo") => {
            let id = args.get(1).ok_or_else(|| anyhow!("which id?"))?;
            let colour = args.get(2).ok_or_else(|| anyhow!("which colour?"))?;
            let base = test(id)?.clone();
            let mut edited = base.clone();
            edited.photo_url = Some(square(colour)?);
            edited.photo_crop = None;
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
            println!("picture of {id} set to {colour}");
        }
        _ => bail!("peek, notes <id> <text>, delete <id>, create, or photo <id> <colour>"),
    }
    Ok(())
}

/// A 400px square of one colour, as a picture is sent to Apple (a JPEG `data:` URL).
fn square(colour: &str) -> Result<String> {
    let rgb = match colour {
        "red" => [220, 40, 40],
        "green" => [40, 170, 70],
        "blue" => [40, 90, 220],
        other => bail!("{other}: red, green or blue"),
    };
    let image = image::RgbImage::from_pixel(400, 400, image::Rgb(rgb));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    let (url, _) = quince_media::photo_for_apple(png.get_ref(), quince_media::APPLE_PHOTO_LIMIT)
        .ok_or_else(|| anyhow!("the picture could not be made"))?;
    Ok(url)
}
