//! Shared Albums as Apple keeps them since iOS 27: "shared collections", each a CloudKit zone
//! of its own in Photos' container (`SharedCollection-<UUID>`), holding the same photo records
//! as the library (a `CPLAsset` and its `CPLMaster` per photo) and a `cloudkit.share` naming
//! the album — its title, when it expires, how it may be viewed. An album this account made is
//! in its private database; one it was invited to is in the shared database, in a zone the
//! other account owns. The older `sharedstreams` service (`shared_albums`) knows none of them:
//! it answers an empty list for an album icloud.com shows. Mapped live 2026-11-05 (see
//! docs/icloud-web/photos.md).
//!
//! An album is small (Apple caps one at a few thousand photos), so it is read whole whenever
//! its change feed says it changed: what Apple has now is what the album holds, nothing kept
//! from an older read can linger, and no half-read album is ever shown.

use crate::cloudkit::{field, Database, ZoneClient};
use crate::photos::{links_with_edit, photo_of, Links, CONTAINER};
use crate::Result;
use quince_core::photos::Photo;
use serde_json::Value;
use std::collections::HashMap;

/// The zones of the new Shared Albums are named so.
pub const ZONE_PREFIX: &str = "SharedCollection-";

/// Where one shared album lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub database: Database,
    pub zone: String,
    /// The account that owns it, for one shared with this account.
    pub owner: Option<String>,
}

impl Place {
    /// Made by this account (in its own database), rather than joined.
    #[must_use]
    pub fn owned(&self) -> bool {
        self.database == Database::Private
    }
}

/// A shared album, as its `cloudkit.share` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct About {
    pub title: Option<String>,
    /// When Apple ends the album, unix seconds ("Expires November 19").
    pub expires_at: Option<i64>,
}

/// One whole read of a shared album.
pub struct Read {
    pub about: About,
    pub photos: Vec<Photo>,
    /// Every photo's picture links, by master, as the library keeps them for its own photos.
    pub links: HashMap<String, Links>,
    /// Where the album's change feed stands now: asked next time whether anything changed.
    pub token: Option<String>,
}

fn client<'a>(
    http: &'a reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: &Place,
) -> ZoneClient<'a> {
    ZoneClient::new(http, ckdatabasews_root, CONTAINER, params)
        .in_database(place.database)
        .owned_by(place.owner.clone())
}

/// Every shared album this account made or was invited to.
pub async fn places(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<Place>> {
    let mut out = Vec::new();
    for database in [Database::Private, Database::Shared] {
        let ids = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params.clone())
            .in_database(database)
            .zone_ids()
            .await?;
        out.extend(
            ids.into_iter()
                .filter(|(name, _)| name.starts_with(ZONE_PREFIX))
                .map(|(zone, owner)| Place {
                    database,
                    zone,
                    owner,
                }),
        );
    }
    Ok(out)
}

/// Whether the album changed since `token`, and the token to keep. With no token held, or one
/// Apple no longer takes, it has (it is to be read whole).
pub async fn changed(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: &Place,
    token: Option<&str>,
) -> Result<bool> {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Ok(true);
    };
    match client(http, ckdatabasews_root, params, place)
        .changes(&place.zone, Some(token))
        .await
    {
        Ok(page) => Ok(!page.records.is_empty() || page.more_coming),
        Err(crate::Error::ReauthRequired) => Err(crate::Error::ReauthRequired),
        // A cursor Apple refuses (expired) or anything odd about it: read whole, which is
        // always right.
        Err(_) => Ok(true),
    }
}

/// The album read whole: what it is called, every photo in it, and their links.
pub async fn read(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: &Place,
) -> Result<Read> {
    let walk = client(http, ckdatabasews_root, params, place)
        .all_records(&place.zone, None)
        .await?;
    let (about, photos, links) = parse(&walk.records);
    Ok(Read {
        about,
        photos,
        links,
        token: walk.token,
    })
}

/// One shared photo's links, read from its album now: for a picture whose kept link expired.
pub async fn links(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: &Place,
    master: &str,
    asset: &str,
) -> Result<Option<Links>> {
    let client = client(http, ckdatabasews_root, params, place);
    let Some(m) = client.lookup_record(&place.zone, master).await? else {
        return Ok(None);
    };
    let a = client
        .lookup_record(&place.zone, asset)
        .await
        .ok()
        .flatten();
    Ok(Some(links_with_edit(a.as_ref(), &m)))
}

/// An album's records as its title, expiry, photos and their links. A photo's asset without
/// its master (or a master without an asset) is no photo yet, and is left out; deleted records
/// are simply absent from a whole read.
#[must_use]
pub fn parse(records: &[Value]) -> (About, Vec<Photo>, HashMap<String, Links>) {
    let live = || {
        records
            .iter()
            .filter(|r| r.get("deleted").and_then(Value::as_bool) != Some(true))
    };
    let masters: HashMap<String, &Value> = live()
        .filter(|r| field::record_type(r).as_deref() == Some("CPLMaster"))
        .filter_map(|r| Some((field::record_name(r)?, r)))
        .collect();
    let mut about = About::default();
    let mut photos = Vec::new();
    let mut links = HashMap::new();
    for r in live() {
        match field::record_type(r).as_deref() {
            Some("cloudkit.share") => {
                about.title = field::string(r, "cloudkit.title").filter(|t| !t.trim().is_empty());
                about.expires_at = field::timestamp_seconds(r, "expiryDate");
            }
            Some("CPLAsset") => {
                let Some(master) = field::reference(r, "masterRef").and_then(|m| masters.get(&m))
                else {
                    continue;
                };
                if let Some(photo) = photo_of(r, master) {
                    links.insert(photo.master.clone(), links_with_edit(Some(r), master));
                    photos.push(photo);
                }
            }
            _ => {}
        }
    }
    // Newest first, as an album shows them; ties by id so the order is the same every read.
    photos.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then(a.id.cmp(&b.id)));
    (about, photos, links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn master(name: &str) -> Value {
        json!({"recordName": name, "recordType": "CPLMaster", "fields": {
            "itemType": {"value": "public.jpeg"},
            "filenameEnc": {"value": base64::engine::general_purpose::STANDARD.encode(format!("{name}.JPG"))},
            "resOriginalWidth": {"value": 1200}, "resOriginalHeight": {"value": 2600},
            "resJPEGThumbRes": {"value": {"downloadURL": "https://cvws.example/thumb/${f}"}},
            "resOriginalRes": {"value": {"downloadURL": "https://cvws.example/orig/${f}"}}
        }})
    }

    fn asset(name: &str, master: &str, taken_ms: i64) -> Value {
        json!({"recordName": name, "recordType": "CPLAsset", "fields": {
            "masterRef": {"value": {"recordName": master}},
            "assetDate": {"value": taken_ms}
        }})
    }

    /// What Apple sent for his "Wallpapers" album, cut down: the share names it and says when
    /// it ends; each asset with its master is a photo with links; newest first.
    #[test]
    fn an_albums_records_are_its_title_expiry_and_photos() {
        let records = vec![
            json!({"recordName": "share", "recordType": "cloudkit.share", "fields": {
                "cloudkit.title": {"value": "Wallpapers"},
                "expiryDate": {"value": 1_795_498_444_000_i64}
            }}),
            master("M1"),
            asset("A1", "M1", 1_790_000_000_000),
            master("M2"),
            asset("A2", "M2", 1_791_000_000_000),
            json!({"recordName": "q", "recordType": "CPLSharedCollectionQuota", "fields": {}}),
        ];
        let (about, photos, links) = parse(&records);
        assert_eq!(about.title.as_deref(), Some("Wallpapers"));
        assert_eq!(about.expires_at, Some(1_795_498_444));
        let ids: Vec<&str> = photos.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["A2", "A1"], "newest first");
        assert!(links["M1"].thumb.is_some() && links["M1"].original.is_some());
    }

    /// An asset whose master hasn't come (or was deleted), a deleted asset, and a master with
    /// no asset are no photos: an album never shows a photo it can't draw.
    #[test]
    fn only_whole_photos_are_shown() {
        let mut gone = asset("A3", "M3", 1);
        gone["deleted"] = json!(true);
        let records = vec![
            asset("A1", "M-missing", 1),
            master("M3"),
            gone,
            master("M4"),
        ];
        let (about, photos, _) = parse(&records);
        assert!(photos.is_empty(), "{photos:?}");
        assert_eq!(about, About::default());
    }
}
