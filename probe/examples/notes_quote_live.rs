//! Live, throwaway note only: Block Quote as the editor saves it — two quoted lines between
//! plain ones — read back from Apple as Apple stores a quote (paragraph field 8), then taken
//! off one line, then the note moved to Recently Deleted. Prints the record name to delete.
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let first = "Quince mount test quote\nplain";
    let guid =
        quince_icloud::notes::create_cloudkit(&http, &root, p.clone(), None, first, None).await?;
    let mut note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title: "Quince mount test quote".into(),
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
    let plain = |n: u64| Run {
        length: n,
        ..Run::default()
    };
    let quoted = |n: u64| Run {
        length: n,
        quote: true,
        ..Run::default()
    };
    // "Quince mount test quote\n" (24) "Said once\n" (10, quoted) "and again\n" (10, quoted) "after" (5)
    let text = "Quince mount test quote\nSaid once\nand again\nafter";
    let runs = vec![
        Run {
            style: Some(0),
            ..plain(24)
        },
        quoted(10),
        quoted(10),
        plain(5),
    ];
    let quotes = |r: &serde_json::Value| -> Vec<(u64, bool)> {
        runs_from_base64(
            r["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .unwrap_or(""),
        )
        .iter()
        .map(|x| (x.length, x.quote))
        .collect()
    };
    let saved = quince_icloud::notes::edit_rich_cloudkit_from(
        &http,
        &root,
        p.clone(),
        &note,
        Some(first),
        text,
        Some(&runs),
        None,
    )
    .await?;
    let back = zc
        .lookup_record("Notes", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let got = quotes(&back);
    let quoted_len: u64 = got.iter().filter(|(_, q)| *q).map(|(n, _)| n).sum();
    println!(
        "{} saved: quoted {} of the 20 quoted units, runs {got:?}",
        if saved == text && quoted_len == 20 {
            "PASS"
        } else {
            "FAIL"
        },
        quoted_len
    );
    // Off the second line, as Command-' on it would.
    note.etag = back["recordChangeTag"].as_str().map(String::from);
    let runs = vec![
        Run {
            style: Some(0),
            ..plain(24)
        },
        quoted(10),
        plain(10),
        plain(5),
    ];
    quince_icloud::notes::edit_rich_cloudkit_from(
        &http,
        &root,
        p.clone(),
        &note,
        Some(text),
        text,
        Some(&runs),
        None,
    )
    .await?;
    let back = zc
        .lookup_record("Notes", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let quoted_len: u64 = quotes(&back)
        .iter()
        .filter(|(_, q)| *q)
        .map(|(n, _)| n)
        .sum();
    println!(
        "{} taken off one line: quoted {quoted_len} (want 10)",
        if quoted_len == 10 { "PASS" } else { "FAIL" }
    );
    note.etag = back["recordChangeTag"].as_str().map(String::from);
    quince_icloud::notes::delete_cloudkit(&http, &root, p, &note).await?;
    println!("RECORD {guid}");
    Ok(())
}
