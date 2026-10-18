//! Live, throwaway note only: a checklist saved as the editor saves it, one line then ticked
//! as another device ticks it, then a save from an editor that never saw the tick — the tick
//! must survive — and the note moved to Recently Deleted.
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
    let body = "Quince tick test\nplain";
    let guid =
        quince_icloud::notes::create_cloudkit(&http, &root, p.clone(), None, body, None).await?;
    let note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title: "Quince tick test".into(),
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
    let text = "Quince tick test\nMilk\nEggs";
    let plain = |n: u64| Run {
        length: n,
        ..Run::default()
    };
    let item = |n: u64| Run {
        style: Some(103),
        checked: Some(false),
        ..plain(n)
    };
    let runs = vec![
        Run {
            style: Some(0),
            ..plain(17)
        },
        item(5),
        item(4),
    ];
    quince_icloud::notes::edit_rich_cloudkit_from(
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
    // Another device ticks Milk.
    quince_icloud::notes::set_paragraph_cloudkit(
        &http,
        &root,
        p.clone(),
        &note,
        17,
        quince_icloud::ckdocument::ParagraphChange::Checked(true),
        Some("Milk"),
    )
    .await?;
    // This window, which never saw the tick, adds a line.
    let typed = "Quince tick test\nMilk\nEggs\nJam";
    let mine = vec![
        Run {
            style: Some(0),
            ..plain(17)
        },
        item(5),
        item(5),
        item(3),
    ];
    quince_icloud::notes::edit_rich_cloudkit_from(
        &http,
        &root,
        p.clone(),
        &note,
        Some(text),
        typed,
        Some(&mine),
        Some(&runs),
    )
    .await?;
    let back = zc.lookup_record("Notes", &guid).await?.unwrap();
    let got = runs_from_base64(
        back["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or(""),
    );
    println!("runs back: {}", show(&back));
    let milk = got
        .iter()
        .scan(0u64, |at, r| {
            let s = *at;
            *at += r.length;
            Some((s, r.clone()))
        })
        .find(|(s, _)| *s == 17)
        .map(|(_, r)| r.checked);
    println!("Milk still ticked: {:?}", milk);
    quince_icloud::notes::delete_cloudkit(&http, &root, p.clone(), &note).await?;
    println!("moved to Recently Deleted");
    Ok(())
}
