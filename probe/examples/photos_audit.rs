//! Read-only audit of the Photos sync on his real library: the whole zone walked and applied
//! to an in-memory library with Quince's own code, every view's count set beside Apple's own
//! count of it (HyperionIndexCountLookup), and how many photos came through with each kind of
//! detail. Nothing is written anywhere.
use anyhow::{anyhow, Result};
use quince_core::photos::View;
use serde_json::{json, Value};

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
    let fetched = quince_icloud::photos::fetch(&http, &root, p.clone(), None).await?;
    let mut lib = quince_core::Library::in_memory()?;
    quince_icloud::photos::apply(&mut lib, fetched)?;

    // Apple's own counts, as icloud.com asks for them.
    let mut q = p.clone();
    q.push(("remapEnums".into(), "true".into()));
    let url = format!(
        "{}/database/1/com.apple.photos.cloud/production/private/records/query",
        root.trim_end_matches('/')
    );
    let ids = [
        "CPLAssetByAssetDateWithoutHiddenOrDeleted",
        "CPLAssetInSmartAlbumByAssetDate:FAVORITE",
        "CPLAssetInSmartAlbumByAssetDate:VIDEO",
        "CPLAssetInSmartAlbumByAssetDate:SCREENSHOT",
        "CPLAssetInSmartAlbumByAssetDate:PANORAMA",
        "CPLAssetHiddenByAssetDate",
        "CPLAssetDeletedByExpungedDate",
    ];
    let body = json!({"query":{"recordType":"HyperionIndexCountLookup","filterBy":[
        {"fieldName":"indexCountID","comparator":"IN","fieldValue":{"value":ids,"type":"STRING_LIST"}}]},
        "zoneID":{"zoneName":"PrimarySync"}});
    let v: Value = http
        .post(&url)
        .query(&q)
        .header("content-type", "text/plain")
        .body(body.to_string())
        .send()
        .await?
        .json()
        .await?;
    let apple = |id: &str| -> Option<i64> {
        v["records"].as_array()?.iter().find_map(|r| {
            (r["recordName"].as_str()? == id).then(|| r["fields"]["itemCount"]["value"].as_i64())?
        })
    };
    // Smart albums (Favorites, Media Types) are counted by walking them as icloud.com pages them;
    // their count-lookup ids are not the ones in the notes.
    let walk = |album: &'static str| {
        let (http, url, q) = (http.clone(), url.clone(), q.clone());
        async move {
            let mut rank = 0i64;
            loop {
                let body = json!({"query":{"recordType":"CPLAssetAndMasterInSmartAlbumByAssetDate","filterBy":[
                    {"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":"ASCENDING","type":"STRING"}},
                    {"fieldName":"smartAlbum","comparator":"EQUALS","fieldValue":{"value":album,"type":"STRING"}},
                    {"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":rank,"type":"INT64"}}]},
                    "zoneID":{"zoneName":"PrimarySync"},"resultsLimit":100});
                let v: Value = http
                    .post(&url)
                    .query(&q)
                    .header("content-type", "text/plain")
                    .body(body.to_string())
                    .send()
                    .await?
                    .json()
                    .await?;
                let n = v["records"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|r| r["recordType"] == "CPLAsset")
                    .count() as i64;
                if n == 0 || rank > 100_000 {
                    return anyhow::Ok(rank);
                }
                rank += n;
            }
        }
    };
    let walked = [
        walk("FAVORITE").await?,
        walk("VIDEO").await?,
        walk("SCREENSHOT").await?,
        walk("PANORAMA").await?,
    ];
    let views = [
        (View::Library, ids[0]),
        (View::Favorites, ids[1]),
        (View::Videos, ids[2]),
        (View::Screenshots, ids[3]),
        (View::Panoramas, ids[4]),
        (View::Hidden, ids[5]),
        (View::RecentlyDeleted, ids[6]),
    ];
    let mut all_match = true;
    for (view, id) in views {
        let ours = lib.photos(view)?.len() as i64;
        let theirs = match view {
            View::Favorites => Some(walked[0]),
            View::Videos => Some(walked[1]),
            View::Screenshots => Some(walked[2]),
            View::Panoramas => Some(walked[3]),
            _ => apple(id),
        };
        let same = theirs == Some(ours);
        all_match &= same;
        println!(
            "{view:?}: Quince {ours}, Apple {theirs:?} {}",
            if same { "✓" } else { "✗" }
        );
    }
    let photos = lib.photos(View::Library)?;
    let count =
        |f: &dyn Fn(&quince_core::photos::Photo) -> bool| photos.iter().filter(|p| f(p)).count();
    println!(
        "details: titles {}, captions {}, rated {}, keywords {}, places {}, favorites {}",
        count(&|p| p.title.is_some()),
        count(&|p| p.caption.is_some()),
        count(&|p| p.rating > 0),
        count(&|p| !p.keywords.is_empty()),
        count(&|p| p.latitude.is_some()),
        count(&|p| p.favorite)
    );
    println!("Rated: {}", lib.photos(View::Rated)?.len());
    println!(
        "{}",
        if all_match {
            "ALL VIEWS MATCH APPLE"
        } else {
            "MISMATCH"
        }
    );
    Ok(())
}
