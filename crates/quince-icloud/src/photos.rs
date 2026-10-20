//! iCloud Photos: the library mirrored through CloudKit's change feed.
//!
//! Container `com.apple.photos.cloud`, zone `PrimarySync`. A photo is two records: a
//! `CPLAsset` (what the person sees: the date, the flags, edits) pointing by `masterRef` at a
//! `CPLMaster` (the original file and its renditions). Measured on his library: a full walk is
//! 273 records in eight seconds, and a pass from the token with nothing new is under half a
//! second. See docs/icloud-web/photos.md.

use crate::cloudkit::{field, ZoneClient};
use crate::Result;
use base64::Engine;
use quince_core::photos::{Album, Kind, Photo};
use serde_json::Value;
use std::collections::HashMap;

pub const CONTAINER: &str = "com.apple.photos.cloud";
pub const ZONE: &str = "PrimarySync";
/// The library's sync token, in the library's sync state.
pub const SERVICE: &str = "photos";

/// What a walk of the zone brought.
pub struct PhotosFetch {
    pub records: Vec<Value>,
    pub token: Option<String>,
    /// From a token: only what changed. Without one: everything, and anything not in it is gone.
    pub incremental: bool,
    pub complete: bool,
}

/// The change feed from `since` (or from the start).
pub async fn fetch(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    since: Option<&str>,
) -> Result<PhotosFetch> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let walk = client.all_records(ZONE, since).await?;
    Ok(PhotosFetch {
        records: walk.records,
        token: walk.token,
        incremental: since.is_some(),
        complete: walk.complete,
    })
}

/// Where each of a photo's pictures can be downloaded now. They expire: never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    pub thumb: Option<String>,
    pub medium: Option<String>,
    pub original: Option<String>,
    /// The whole picture as a JPEG, for zooming in: Apple's full-size rendition, or the
    /// original itself when that is already a JPEG or PNG (a HEIC original shows nowhere here).
    pub full: Option<String>,
    /// A video's playable rendition, or a Live Photo's moving part.
    pub video: Option<String>,
}

fn link(fields: &Value, key: &str, name: &str) -> Option<String> {
    fields
        .pointer(&format!("/{key}/value/downloadURL"))
        .and_then(Value::as_str)
        .map(|u| u.replace("${f}", name))
}

/// A master's download links, as icloud.com takes them (`${f}` is the file name it asks for).
#[must_use]
pub fn links_of(master: &Value) -> Links {
    let f = &master["fields"];
    Links {
        thumb: link(f, "resJPEGThumbRes", "thumb.jpg")
            .or_else(|| link(f, "resJPEGMedRes", "med.jpg")),
        medium: link(f, "resJPEGMedRes", "med.jpg")
            .or_else(|| link(f, "resJPEGFullRes", "full.jpg")),
        original: link(f, "resOriginalRes", "original"),
        full: link(f, "resJPEGFullRes", "full.jpg").or_else(|| {
            let kind = f.pointer("/itemType/value").and_then(Value::as_str);
            matches!(kind, Some("public.jpeg" | "public.png"))
                .then(|| link(f, "resOriginalRes", "original"))
                .flatten()
        }),
        video: link(f, "resVidMedRes", "video.mp4")
            .or_else(|| link(f, "resVidSmallRes", "video.mp4"))
            .or_else(|| link(f, "resOriginalVidComplRes", "live.mov")),
    }
}

fn decoded(record: &Value, name: &str) -> Option<String> {
    let v = record.pointer(&format!("/fields/{name}/value"))?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(v).ok()?;
    String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

fn millis(record: &Value, name: &str) -> Option<i64> {
    record
        .pointer(&format!("/fields/{name}/value"))
        .and_then(Value::as_i64)
        .map(|ms| ms / 1000)
}

/// A photo from its asset and the master it shows.
#[must_use]
pub fn photo_of(asset: &Value, master: &Value) -> Option<Photo> {
    let id = field::record_name(asset)?;
    let master_id = field::record_name(master)?;
    let m = &master["fields"];
    let uti = m
        .pointer("/itemType/value")
        .and_then(Value::as_str)
        .map(str::to_string);
    let video = uti
        .as_deref()
        .is_some_and(|t| t.contains("movie") || t.contains("mpeg") || t.contains("video"));
    let kind = if video {
        Kind::Video
    } else if m.get("resOriginalVidComplRes").is_some() {
        Kind::Live
    } else {
        Kind::Image
    };
    let size = |k: &str| m.pointer(&format!("/{k}/value")).and_then(Value::as_i64);
    let (mut width, mut height) = (
        size("resOriginalWidth")
            .or(size("resJPEGMedWidth"))
            .unwrap_or(0),
        size("resOriginalHeight")
            .or(size("resJPEGMedHeight"))
            .unwrap_or(0),
    );
    // EXIF orientations 5 to 8 are turned a quarter: the picture is as tall as it is wide.
    let orientation =
        field::int(asset, "orientation").or(field::int(master, "originalOrientation"));
    if matches!(orientation, Some(5..=8)) {
        std::mem::swap(&mut width, &mut height);
    }
    Some(Photo {
        id,
        master: master_id,
        taken_at: millis(asset, "assetDate")
            .or(millis(master, "originalCreationDate"))
            .unwrap_or(0),
        added_at: millis(asset, "addedDate"),
        kind,
        width,
        height,
        duration: asset
            .pointer("/fields/duration/value")
            .and_then(Value::as_f64)
            .filter(|d| *d > 0.0),
        filename: decoded(master, "filenameEnc"),
        size: size("resOriginalFileSize"),
        uti,
        favorite: field::int(asset, "isFavorite") == Some(1),
        hidden: field::int(asset, "isHidden") == Some(1),
        deleted: field::int(asset, "isDeleted") == Some(1),
        deleted_at: field::int(asset, "isDeleted")
            .filter(|&d| d == 1)
            .and(millis(asset, "recordModificationDate")),
        subtype: field::int(asset, "assetSubtypeV2").unwrap_or(0),
        expunge_at: (field::int(asset, "isDeleted") == Some(1))
            .then(|| millis(asset, "dateExpunged"))
            .flatten(),
        change_tag: asset
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// An asset's own fields laid over the photo it already is: what an asset-only change (a
/// favourite, a delete) brings, when its master did not change.
fn with_asset(mut p: Photo, asset: &Value) -> Photo {
    if let Some(t) = millis(asset, "assetDate") {
        p.taken_at = t;
    }
    p.added_at = millis(asset, "addedDate").or(p.added_at);
    p.favorite = field::int(asset, "isFavorite") == Some(1);
    p.hidden = field::int(asset, "isHidden") == Some(1);
    p.deleted = field::int(asset, "isDeleted") == Some(1);
    p.deleted_at = if p.deleted {
        millis(asset, "recordModificationDate").or(p.deleted_at)
    } else {
        None
    };
    p.expunge_at = if p.deleted {
        millis(asset, "dateExpunged").or(p.expunge_at)
    } else {
        None
    };
    p.change_tag = asset
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .map(str::to_string);
    p
}

/// A pass applied to the library: everything replaced on a full pass, merged on an incremental
/// one. Answers how many photos changed, and every master's links seen, for the app to keep.
pub fn apply(
    library: &mut quince_core::Library,
    fetched: PhotosFetch,
) -> Result<(usize, HashMap<String, Links>)> {
    let PhotosFetch {
        records,
        token,
        incremental,
        complete,
    } = fetched;
    let mut masters: HashMap<String, &Value> = HashMap::new();
    let mut assets: Vec<&Value> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    let mut albums: Vec<Album> = Vec::new();
    let mut albums_seen = false;
    // Every album this pass speaks of: one deleted elsewhere arrives marked deleted, and has
    // to leave the list rather than stay as it was.
    let mut albums_touched: Vec<String> = Vec::new();
    // Which photo is in which album: a relation record each, named `<asset>-IN-<album>`.
    let mut in_albums: Vec<(String, String, i64)> = Vec::new();
    let mut out_of_albums: Vec<(String, String)> = Vec::new();
    for r in &records {
        if r.get("deleted").and_then(Value::as_bool) == Some(true) {
            if let Some(name) = field::record_name(r) {
                match name.split_once("-IN-") {
                    Some((asset, album)) => {
                        out_of_albums.push((album.to_string(), asset.to_string()))
                    }
                    None => gone.push(name),
                }
            }
            continue;
        }
        match field::record_type(r).as_deref() {
            Some("CPLMaster") => {
                if let Some(name) = field::record_name(r) {
                    masters.insert(name, r);
                }
            }
            // Deleted for good stays a record at Apple, marked expunged: it is gone here.
            Some("CPLAsset") if field::int(r, "isExpunged") == Some(1) => {
                if let Some(name) = field::record_name(r) {
                    gone.push(name);
                }
            }
            Some("CPLAsset") => assets.push(r),
            Some("CPLContainerRelation") => {
                let album = field::string(r, "containerId");
                let asset = field::string(r, "itemId");
                if let (Some(album), Some(asset)) = (album, asset) {
                    if field::int(r, "isDeleted") == Some(1) {
                        out_of_albums.push((album, asset));
                    } else {
                        in_albums.push((album, asset, field::int(r, "position").unwrap_or(0)));
                    }
                }
            }
            Some("CPLAlbum") => {
                albums_seen = true;
                if let Some(id) = field::record_name(r) {
                    albums_touched.push(id);
                }
                // Folders and smart albums are not albums a photo is put in.
                if field::int(r, "isDeleted") != Some(1)
                    && field::int(r, "albumType").unwrap_or(0) == 0
                {
                    if let (Some(id), Some(name)) =
                        (field::record_name(r), decoded(r, "albumNameEnc"))
                    {
                        albums.push(Album {
                            id,
                            name,
                            position: field::int(r, "position").unwrap_or(0),
                            change_tag: r
                                .get("recordChangeTag")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    let links: HashMap<String, Links> = masters
        .iter()
        .map(|(k, m)| (k.clone(), links_of(m)))
        .collect();
    let mut photos = Vec::new();
    for a in assets {
        let master = a
            .pointer("/fields/masterRef/value/recordName")
            .and_then(Value::as_str);
        let built = match master.and_then(|m| masters.get(m)) {
            Some(m) => photo_of(a, m),
            // Only the asset changed: its flags laid over the photo already known.
            None => field::record_name(a)
                .and_then(|id| library.photo(&id).ok().flatten())
                .map(|p| with_asset(p, a)),
        };
        if let Some(p) = built {
            photos.push(p);
        }
    }
    let changed = photos.len() + gone.len();
    if incremental {
        library.put_photos(&photos)?;
        library.forget_photos(&gone)?;
        library.put_album_items(&in_albums)?;
        library.forget_album_items(&out_of_albums)?;
        if albums_seen {
            let mut all = library.albums()?;
            all.retain(|a| !albums_touched.contains(&a.id) && !gone.contains(&a.id));
            for id in albums_touched.iter().chain(&gone) {
                if !albums.iter().any(|b| &b.id == id) {
                    library.forget_album(id)?;
                }
            }
            all.extend(albums);
            library.replace_albums(&all)?;
        }
    } else if complete {
        // A full pass that finished is the whole truth; one that stopped early is not, and
        // pruning on it would drop photos that are fine.
        library.replace_photos(&photos)?;
        library.replace_albums(&albums)?;
        library.replace_album_items(&in_albums)?;
    } else {
        library.put_photos(&photos)?;
        library.put_album_items(&in_albums)?;
    }
    if complete {
        library.mark_synced(SERVICE, token.as_deref())?;
    }
    Ok((changed, links))
}

/// A photo's flag set, as icloud.com sets it: its `CPLAsset` record updated with the one
/// field, against the asset's change tag. Favorite is `isFavorite`; Delete (to Recently
/// Deleted, never for good) is `isDeleted: 1`; Recover is `isDeleted: 0`; Hide is `isHidden`.
/// Answers the asset as Apple now holds it.
pub async fn set_flag(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    asset: &str,
    change_tag: Option<&str>,
    flag: Flag,
    on: bool,
) -> Result<Value> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let name = match flag {
        Flag::Favorite => "isFavorite",
        Flag::Deleted => "isDeleted",
        Flag::Hidden => "isHidden",
        Flag::Expunged => "isExpunged",
    };
    client
        .update_record(
            ZONE,
            asset,
            "CPLAsset",
            change_tag,
            serde_json::json!({ name: { "value": i64::from(on) } }),
        )
        .await
}

/// An album's name as Photos writes it: base64 of its UTF-8 bytes, untyped.
fn album_name(name: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(name.as_bytes())
}

/// Highest position an album or an album's item may take (`MAX_SAFE_INTEGER - 1024`).
pub const MAX_POSITION: i64 = 9_007_199_254_740_991 - 1024;
/// Gap between items added to an album, and the first item's position.
const ITEM_SPACING: i64 = 1024;

/// A new album, made as icloud.com makes one: CloudKit names it, it sorts first (two below
/// the first album there is), and its photos sort by date. Answers the album.
pub async fn create_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    name: &str,
    first_position: Option<i64>,
) -> Result<Album> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let position = first_position.map_or(MAX_POSITION, |p| (p - 2).max(0));
    let made = client
        .create_record_unnamed(
            ZONE,
            "CPLAlbum",
            serde_json::json!({
                "albumNameEnc": { "value": album_name(name) },
                "albumType": { "value": 0 },
                "isDeleted": { "value": 0 },
                "isExpunged": { "value": 0 },
                "position": { "value": position },
                "sortAscending": { "value": 1 },
                "sortType": { "value": 1 },
            }),
        )
        .await?;
    Ok(Album {
        id: field::record_name(&made).ok_or_else(|| {
            crate::Error::Protocol("CloudKit made the album without a name".into())
        })?,
        name: name.to_string(),
        position,
        change_tag: made
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// An album renamed, against its change tag. Answers the new change tag.
pub async fn rename_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    change_tag: Option<&str>,
    name: &str,
) -> Result<Option<String>> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let now = jiff::Timestamp::now().as_millisecond();
    let answer = client
        .update_record(
            ZONE,
            album,
            "CPLAlbum",
            change_tag,
            serde_json::json!({
                "albumNameEnc": { "value": album_name(name) },
                "userModificationDate": { "value": now },
            }),
        )
        .await?;
    Ok(answer
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// An album deleted, as Photos deletes one: marked deleted. Its photos stay in the library.
pub async fn delete_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    change_tag: Option<&str>,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    client
        .update_record(
            ZONE,
            album,
            "CPLAlbum",
            change_tag,
            serde_json::json!({ "isDeleted": { "value": 1 } }),
        )
        .await?;
    Ok(())
}

/// Photos put in an album, after what is there: a `CPLContainerRelation` each, named
/// `<asset>-IN-<album>`, 1024 apart. Answers each photo with the position it took.
pub async fn add_to_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    assets: &[String],
    last_position: Option<i64>,
) -> Result<Vec<(String, i64)>> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let mut at = last_position.unwrap_or(ITEM_SPACING);
    let mut added = Vec::new();
    for asset in assets {
        at = (at + ITEM_SPACING).min(MAX_POSITION);
        client
            .create_record(
                ZONE,
                &format!("{asset}-IN-{album}"),
                "CPLContainerRelation",
                serde_json::json!({
                    "itemId": { "value": asset },
                    "position": { "value": at },
                    "containerId": { "value": album },
                }),
            )
            .await?;
        added.push((asset.clone(), at));
    }
    Ok(added)
}

/// Photos taken out of an album, as icloud.com takes them: their relation records removed
/// outright. The photos stay in the library.
pub async fn remove_from_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    assets: &[String],
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    for asset in assets {
        client
            .delete_record(ZONE, &format!("{asset}-IN-{album}"))
            .await?;
    }
    Ok(())
}

/// The flags a photo carries that this app sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flag {
    Favorite,
    Deleted,
    Hidden,
    /// Deleted for good, from Recently Deleted: `isExpunged: 1`. It cannot be undone.
    Expunged,
}

/// One master's links, read from Apple now: for a picture whose cached link expired, or one
/// never seen in this run.
pub async fn links(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    master: &str,
) -> Result<Option<Links>> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    Ok(client
        .lookup_record(ZONE, master)
        .await?
        .map(|m| links_of(&m)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn master() -> Value {
        json!({"recordName": "M1", "recordType": "CPLMaster", "fields": {
            "itemType": {"value": "public.jpeg"},
            "filenameEnc": {"value": base64::engine::general_purpose::STANDARD.encode("IMG_0001.JPG")},
            "resOriginalWidth": {"value": 4032}, "resOriginalHeight": {"value": 3024},
            "resOriginalFileSize": {"value": 2_400_000},
            "resJPEGThumbRes": {"value": {"downloadURL": "https://cvws.example/thumb/${f}"}},
            "resOriginalRes": {"value": {"downloadURL": "https://cvws.example/orig/${f}"}}
        }})
    }

    fn asset(flags: Value) -> Value {
        let mut fields = json!({
            "masterRef": {"value": {"recordName": "M1"}},
            "assetDate": {"value": 1_789_640_832_217_i64}, "addedDate": {"value": 1_792_337_452_485_i64},
            "orientation": {"value": 6}
        });
        for (k, v) in flags.as_object().unwrap() {
            fields[k] = json!({"value": v});
        }
        json!({"recordName": "A1", "recordType": "CPLAsset", "recordChangeTag": "t1", "fields": fields})
    }

    /// An asset and its master read as one photo: its date, size (turned for orientation 6),
    /// name, flags and download links.
    #[test]
    fn an_asset_and_its_master_are_one_photo() {
        let p = photo_of(&asset(json!({"isFavorite": 1})), &master()).unwrap();
        assert_eq!((p.id.as_str(), p.master.as_str()), ("A1", "M1"));
        assert_eq!(p.taken_at, 1_789_640_832);
        assert_eq!(
            (p.width, p.height),
            (3024, 4032),
            "orientation 6 is a portrait"
        );
        assert_eq!(p.filename.as_deref(), Some("IMG_0001.JPG"));
        assert!(p.favorite && !p.deleted);
        assert_eq!(p.kind, Kind::Image);
        let l = links_of(&master());
        assert_eq!(
            l.thumb.as_deref(),
            Some("https://cvws.example/thumb/thumb.jpg")
        );
        assert_eq!(
            l.original.as_deref(),
            Some("https://cvws.example/orig/original")
        );
    }

    /// A full pass replaces the library; a later asset-only change (deleted) is laid over the
    /// photo already known; an expunged asset goes.
    #[test]
    fn passes_build_and_keep_the_library() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let full = PhotosFetch {
            records: vec![
                master(),
                asset(json!({})),
                json!({"recordName": "A1-IN-AL", "recordType": "CPLContainerRelation", "fields": {"containerId": {"value": "AL"}, "itemId": {"value": "A1"}, "position": {"value": 1024}}}),
            ],
            token: Some("T1".into()),
            incremental: false,
            complete: true,
        };
        apply(&mut lib, full).unwrap();
        assert_eq!(
            lib.photos(quince_core::photos::View::Library)
                .unwrap()
                .len(),
            1
        );
        let change = PhotosFetch {
            records: vec![asset(json!({"isDeleted": 1}))],
            token: Some("T2".into()),
            incremental: true,
            complete: true,
        };
        apply(&mut lib, change).unwrap();
        assert!(lib
            .photos(quince_core::photos::View::Library)
            .unwrap()
            .is_empty());
        let deleted = lib
            .photos(quince_core::photos::View::RecentlyDeleted)
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].filename.as_deref(),
            Some("IMG_0001.JPG"),
            "the master's details are kept"
        );
        let expunged = PhotosFetch {
            records: vec![json!({"recordName": "A1", "deleted": true})],
            token: Some("T3".into()),
            incremental: true,
            complete: true,
        };
        apply(&mut lib, expunged).unwrap();
        assert!(lib.photo("A1").unwrap().is_none());
    }

    /// An album deleted on another device arrives marked deleted; it leaves the list, and
    /// its photos leave it with it (Apple leaves the relations standing), while a rename
    /// elsewhere is taken as it comes.
    #[test]
    fn an_album_deleted_elsewhere_goes() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let album = |name: &str, deleted: i64| {
            json!({"recordName": "AL", "recordType": "CPLAlbum", "fields": {
                "albumNameEnc": {"value": base64::engine::general_purpose::STANDARD.encode(name)},
                "albumType": {"value": 0}, "position": {"value": 5}, "isDeleted": {"value": deleted}}})
        };
        let relation = json!({"recordName": "A1-IN-AL", "recordType": "CPLContainerRelation", "fields": {"containerId": {"value": "AL"}, "itemId": {"value": "A1"}, "position": {"value": 1024}}});
        let pass = |records, incremental| PhotosFetch {
            records,
            token: Some("T".into()),
            incremental,
            complete: true,
        };
        apply(
            &mut lib,
            pass(
                vec![master(), asset(json!({})), album("Trips", 0), relation],
                false,
            ),
        )
        .unwrap();
        assert_eq!(lib.albums().unwrap()[0].name, "Trips");
        apply(&mut lib, pass(vec![album("Sea", 0)], true)).unwrap();
        assert_eq!(
            lib.albums().unwrap()[0].name,
            "Sea",
            "a rename elsewhere is taken"
        );
        apply(&mut lib, pass(vec![album("Sea", 1)], true)).unwrap();
        assert!(
            lib.albums().unwrap().is_empty(),
            "a deleted album leaves the list"
        );
        assert!(
            lib.album_photos("AL").unwrap().is_empty(),
            "and its photos with it"
        );
        assert_eq!(
            lib.photos(quince_core::photos::View::Library)
                .unwrap()
                .len(),
            1,
            "the photos stay in the library"
        );
    }

    /// A photo deleted for good stays a record at Apple, marked `isExpunged`; the library
    /// lets it go rather than keep it in Recently Deleted.
    #[test]
    fn an_expunged_photo_goes() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let pass = |records, incremental| PhotosFetch {
            records,
            token: Some("T".into()),
            incremental,
            complete: true,
        };
        apply(
            &mut lib,
            pass(vec![master(), asset(json!({"isDeleted": 1}))], false),
        )
        .unwrap();
        assert_eq!(
            lib.photos(quince_core::photos::View::RecentlyDeleted)
                .unwrap()
                .len(),
            1
        );
        apply(
            &mut lib,
            pass(vec![asset(json!({"isDeleted": 1, "isExpunged": 1}))], true),
        )
        .unwrap();
        assert!(lib.photo("A1").unwrap().is_none());
    }
}
