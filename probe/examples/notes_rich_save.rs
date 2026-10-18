//! Live, throwaway note only: a note saved as the editor saves it — text and formatting
//! together, a heading, a bold word, a ticked checklist line — read back from Apple, then
//! moved to Recently Deleted.
use anyhow::{anyhow, Result};
use quince_icloud::ckdocument::{runs_from_base64, Run};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let body = "Quince rich test\nplain";
    let guid =
        quince_icloud::notes::create_cloudkit(&http, &root, p.clone(), None, body, None).await?;
    let note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title: "Quince rich test".into(),
        snippet: None,
        body: None,
        locked: false,
        pinned: false,
        created_at: None,
        modified_at: None,
        editable: true,
        etag: None,
        runs: None,
    };
    let show = |r: &serde_json::Value| {
        runs_from_base64(
            r["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .unwrap_or(""),
        )
        .iter()
        .map(|x| {
            format!(
                "{}:{:?}{}",
                x.length,
                x.style,
                x.checked.map(|c| if c { "✓" } else { "○" }).unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
    };
    let text = "Quince rich test\nShopping\nBuy milk now\nCall the bank";
    let plain = |n: u64| Run {
        length: n,
        ..Run::default()
    };
    let runs = vec![
        Run {
            style: Some(0),
            ..plain(17)
        },
        Run {
            style: Some(1),
            ..plain(9)
        },
        Run {
            style: Some(103),
            checked: Some(true),
            ..plain(4)
        },
        Run {
            style: Some(103),
            checked: Some(true),
            bold: true,
            ..plain(4)
        },
        Run {
            style: Some(103),
            checked: Some(true),
            ..plain(5)
        },
        Run {
            style: Some(101),
            ..plain(13)
        },
    ];
    let saved = quince_icloud::notes::edit_rich_cloudkit_from(
        &http,
        &root,
        p.clone(),
        &note,
        Some(body),
        text,
        Some(&runs),
        None,
    )
    .await?;
    let back = zc.lookup_record("Notes", &guid).await?.unwrap();
    println!("saved text matches: {}", saved == text);
    println!("runs back: {}", show(&back));
    let got = runs_from_base64(
        back["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or(""),
    );
    let bold: Vec<u64> = got.iter().filter(|r| r.bold).map(|r| r.length).collect();
    println!("bold runs: {bold:?} (want [4])");
    println!(
        "text back: {:?}",
        quince_icloud::ckdocument::text_from_base64(
            back["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .unwrap_or("")
        )
    );
    quince_icloud::notes::delete_cloudkit(&http, &root, p.clone(), &note).await?;
    println!("moved to Recently Deleted");
    Ok(())
}
