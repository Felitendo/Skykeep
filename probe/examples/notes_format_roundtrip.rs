//! Live, throwaway note only: make a note, turn its second line into a checklist item, tick
//! it, make the third a dashed list item — reading the runs back from Apple after each — then
//! leave it in Recently Deleted.
use anyhow::{anyhow, Result};
use quince_icloud::ckdocument::{runs_from_base64, ParagraphChange};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? { return Err(anyhow!("no session")); }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let body = "Quince format test\nBuy milk\nCall the bank";
    let guid = quince_icloud::notes::create_cloudkit(&http, &root, p.clone(), None, body, None).await?;
    let note = quince_core::notes::Note { guid: guid.clone(), folder_guid: None, title: "Quince format test".into(), snippet: None, body: None, locked: false, pinned: false, created_at: None, modified_at: None, editable: true, etag: None, runs: None };
    let show = |r: &serde_json::Value| {
        runs_from_base64(r["fields"]["TextDataEncrypted"]["value"].as_str().unwrap_or(""))
            .iter().map(|x| format!("{}:{:?}{}", x.length, x.style, x.checked.map(|c| if c { "✓" } else { "○" }).unwrap_or(""))).collect::<Vec<_>>().join(" ")
    };
    println!("made:      {}", show(&zc.lookup_record("Notes", &guid).await?.unwrap()));
    quince_icloud::notes::set_paragraph_cloudkit(&http, &root, p.clone(), &note, 19, ParagraphChange::Style(103)).await?;
    println!("checklist: {}", show(&zc.lookup_record("Notes", &guid).await?.unwrap()));
    quince_icloud::notes::set_paragraph_cloudkit(&http, &root, p.clone(), &note, 19, ParagraphChange::Checked(true)).await?;
    println!("ticked:    {}", show(&zc.lookup_record("Notes", &guid).await?.unwrap()));
    quince_icloud::notes::set_paragraph_cloudkit(&http, &root, p.clone(), &note, 28, ParagraphChange::Style(101)).await?;
    let last = zc.lookup_record("Notes", &guid).await?.unwrap();
    println!("dashed:    {}", show(&last));
    println!("text kept: {:?}", quince_icloud::ckdocument::text_from_base64(last["fields"]["TextDataEncrypted"]["value"].as_str().unwrap_or("")));
    println!("note {guid} left in place for a look on the phone");
    Ok(())
}
