//! Read only: the library's own counts (CPLLibraryInfo) against what Quince shows once bursts
//! are stacked — every field of that record printed, then Quince's Library count.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let fetched = quince_icloud::photos::fetch(&http, &root, p, None).await?;
    for r in &fetched.records {
        if r["recordType"] == "CPLLibraryInfo" {
            for (k, v) in r["fields"].as_object().unwrap() {
                println!("LibraryInfo.{k} = {}", v["value"]);
            }
        }
    }
    let mut lib = quince_core::Library::in_memory()?;
    quince_icloud::photos::apply(&mut lib, fetched)?;
    for (v, n) in lib.photo_counts()? {
        println!("quince {v:?}: {n}");
    }
    let lib_view = lib.photos(quince_core::photos::View::Library)?;
    let (videos, photos) = lib_view.iter().fold((0, 0), |(v, p), x| {
        if x.kind == quince_core::photos::Kind::Video {
            (v + 1, p)
        } else {
            (v, p + 1)
        }
    });
    let bursts: Vec<_> = lib_view
        .iter()
        .filter(|x| x.burst.is_some())
        .map(|x| x.burst_count)
        .collect();
    println!(
        "quince Library: {photos} photos, {videos} videos; bursts shown as one each: {bursts:?}"
    );
    Ok(())
}
