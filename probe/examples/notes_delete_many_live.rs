//! Live, throwaways only: three notes made, all three deleted at once (into Recently Deleted),
//! then all three deleted at once from there — as he did when Apple answered 409
//! TRY_AGAIN_LATER. Every delete should go through, the busy answers waited out.
use anyhow::{anyhow, Result};
use quince_icloud::notes as n;

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
    let mut made = Vec::new();
    for i in 1..=3 {
        made.push(
            n::create_cloudkit(
                &http,
                &root,
                p.clone(),
                None,
                &format!("Quince delete test {i}\nthrowaway"),
                None,
            )
            .await?,
        );
    }
    println!("made {}", made.len());
    let read = || async {
        let mut lib = quince_core::Library::in_memory()?;
        n::apply_cloudkit(
            &mut lib,
            n::fetch_cloudkit(&http, &root, p.clone(), None).await?,
        )?;
        anyhow::Ok(lib)
    };
    for round in ["to Recently Deleted", "for good"] {
        let lib = read().await?;
        let notes: Vec<_> = made
            .iter()
            .map(|g| lib.note(g))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        let all = futures_util::future::join_all(
            notes
                .iter()
                .map(|note| n::delete_cloudkit(&http, &root, p.clone(), note)),
        )
        .await;
        let failed: Vec<_> = all
            .iter()
            .filter_map(|r| r.as_ref().err())
            .map(ToString::to_string)
            .collect();
        println!(
            "{round}: {} of {} went through; {:?}",
            all.len() - failed.len(),
            all.len(),
            failed
        );
    }
    let lib = read().await?;
    let left = made
        .iter()
        .filter(|g| lib.note(g).ok().flatten().is_some())
        .count();
    println!("still listed afterwards: {left}");
    // Earlier runs' throwaways still in Recently Deleted, found by their own title only.
    let stray: Vec<_> = lib
        .notes_in(Some("TrashFolder-CloudKit"))?
        .into_iter()
        .filter(|x| x.title.starts_with("Quince delete test"))
        .collect();
    for note in &stray {
        n::delete_cloudkit(&http, &root, p.clone(), note).await?;
    }
    println!("earlier throwaways cleared: {}", stray.len());
    Ok(())
}
