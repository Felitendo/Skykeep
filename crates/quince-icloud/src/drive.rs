//! iCloud Drive's web service.
//!
//! Drive is fetched a folder at a time from `retrieveItemDetailsInFolders`, which takes a
//! list of folder identifiers and answers with their contents.

use crate::{Error, Result};
use quince_core::drive::DriveItem;
use serde_json::{json, Value};

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Apple sends dates as milliseconds in some fields and ISO strings in others.
fn seconds(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64().map(|ms| ms / 1000),
        Value::String(s) => jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%S%z", s)
            .ok()
            .map(|t| t.as_second())
            .or_else(|| s.parse::<jiff::Timestamp>().ok().map(|t| t.as_second())),
        _ => None,
    }
}

fn item_from(v: &Value) -> Option<DriveItem> {
    Some(DriveItem {
        drivewsid: string(v, "drivewsid")?,
        parent_id: string(v, "parentId"),
        name: string(v, "name").unwrap_or_else(|| "Untitled".into()),
        extension: string(v, "extension"),
        kind: string(v, "type").unwrap_or_else(|| "FILE".into()),
        // Apple sends -1 for things that have no size, which is not a size.
        size: v.get("size").and_then(Value::as_i64).filter(|n| *n >= 0),
        child_count: v.get("directChildrenCount").and_then(Value::as_i64),
        created_at: seconds(v, "dateCreated"),
        modified_at: seconds(v, "dateModified").or_else(|| seconds(v, "dateChanged")),
        etag: string(v, "etag"),
    })
}

/// Read the contents of one folder from a `retrieveItemDetailsInFolders` response.
#[must_use]
pub fn parse_folder(body: &Value) -> Vec<DriveItem> {
    body.as_array()
        .and_then(|folders| folders.first())
        .and_then(|folder| folder.get("items"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(item_from).collect())
        .unwrap_or_default()
}

/// Fetch one folder's contents and store them.
pub async fn sync_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
    folder_id: &str,
) -> Result<usize> {
    let items = fetch_folder(http, root, params, folder_id).await?;
    store_folder(library, folder_id, &items)
}

/// Store a folder's contents fetched by [`fetch_folder`].
pub fn store_folder(
    library: &mut quince_core::Library,
    folder_id: &str,
    items: &[DriveItem],
) -> Result<usize> {
    library.put_drive_children(folder_id, items)?;
    library.mark_synced("drive", None)?;
    Ok(items.len())
}

/// Ask Apple for one folder's contents. Touches no local state, so it can run without
/// holding the library every screen reads.
pub async fn fetch_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
) -> Result<Vec<DriveItem>> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));

    let url = format!(
        "{}/retrieveItemDetailsInFolders",
        root.trim_end_matches('/')
    );
    let body = json!([{ "drivewsid": folder_id, "partialData": false }]);

    let response = http.post(url).query(&query).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Drive returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }

    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    Ok(parse_folder(&parsed))
}

/// Bare docwsid: `FILE::com.apple.CloudDocs::<uuid>` carries it after the last `::`.
fn docwsid_of(drivewsid: &str) -> &str {
    drivewsid.rsplit("::").next().unwrap_or(drivewsid)
}

/// Where to fetch a file's bytes from — a signed, time-limited URL Apple hands out per
/// request.
///
/// Downloads are served from the `docws` host, not `drivews`: a different subdomain from
/// everything else in this module, measured against the real service rather than assumed.
pub async fn download_url(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<String> {
    // The zone in the path is the file's own. An app library — Keynote's, Shortcuts', a
    // third party's — is a zone of its own, and asking for one of its files under the
    // ordinary Drive zone gets "Could not find document" rather than the file. Every file
    // in every app library was undownloadable while this said com.apple.CloudDocs.
    let url = format!(
        "{}/ws/{}/download/by_id",
        docws_root.trim_end_matches('/'),
        zone_of(&item.drivewsid).unwrap_or(DOCUMENTS_ZONE)
    );
    let mut query = params;
    query.push((
        "document_id".into(),
        docwsid_of(&item.drivewsid).to_string(),
    ));

    let response = http.get(url).query(&query).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "couldn't prepare a download ({status}): {}",
            text.chars().take(160).collect::<String>()
        )));
    }

    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("download reply was not JSON: {e}")))?;
    body.pointer("/data_token/url")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| Error::Protocol("Apple's reply carried no download URL".into()))
}

/// Download a file's bytes to `destination`, streaming rather than holding the whole file
/// in memory — Drive is exactly where a large file is likely.
pub async fn download_to(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    destination: &std::path::Path,
) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let url = download_url(http, docws_root, params, item).await?;
    let response = http.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(Error::Protocol(format!(
            "the download itself failed ({})",
            response.status()
        )));
    }

    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut file = tokio::fs::File::create(destination).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

/// What a `createFolders` reply says happened.
///
/// Apple answers 200 either way. A folder it made comes back described, with `status: "OK"`;
/// one it declined comes back as the client id and a status alone — `UNKNOWN` for a name
/// already used in that folder, which is what a second submit of the same name produced. That
/// used to surface as "Apple created the folder but didn't describe it", which was false.
fn created_folder(body: &Value, name: &str) -> Result<DriveItem> {
    let folder = body
        .pointer("/folders/0")
        .ok_or_else(|| Error::Protocol("Drive's answer named no folder at all".into()))?;
    let status = folder.get("status").and_then(Value::as_str).unwrap_or("OK");
    if status != "OK" {
        return Err(Error::Protocol(format!(
            "iCloud Drive didn't create \u{201c}{name}\u{201d} ({status}). \
             Something with that name may already be here."
        )));
    }
    item_from(folder)
        .ok_or_else(|| Error::Protocol("Drive's answer described the folder without an id".into()))
}

/// Create a folder inside `parent_id`. Returns the item Apple stored.
pub async fn create_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    parent_id: &str,
    name: &str,
) -> Result<DriveItem> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Protocol("a folder needs a name".into()));
    }

    let url = format!("{}/createFolders", root.trim_end_matches('/'));
    let body = json!({
        "destinationDrivewsId": parent_id,
        "folders": [{ "name": name, "clientId": uuid::Uuid::new_v4().to_string() }],
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Drive refused the folder ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    created_folder(&body, name)
}

/// The parent's own listing, straight from Apple — for finding an item's *current* etag
/// and for confirming afterward that a write actually landed.
///
/// `retrieveItemDetailsInFolders` is the only place Drive's web service will hand out an
/// etag, so this is called before a write (to get one that has not gone stale) and after
/// one (to confirm the change is what Apple now has, not just what it answered 200 to).
async fn folder_listing(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    folder_id: &str,
) -> Result<Vec<DriveItem>> {
    let url = format!(
        "{}/retrieveItemDetailsInFolders",
        root.trim_end_matches('/')
    );
    let body = json!([{ "drivewsid": folder_id, "partialData": false }]);
    let response = http.post(url).query(params).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Drive returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    Ok(parse_folder(&parsed))
}

/// The parent a write on `item` should be verified against: its own folder if it has one,
/// the Drive root otherwise.
fn parent_of(item: &DriveItem) -> &str {
    item.parent_id
        .as_deref()
        .unwrap_or(quince_core::drive::ROOT)
}

/// Rename a file or folder in place.
///
/// The etag on `item` may already be stale by the time this runs — nothing enforces that a
/// caller re-synced first — and CloudKit-style services answer a stale etag by silently
/// keeping the old version while still returning 200. So this fetches the etag Apple
/// currently holds immediately before writing, and afterward re-reads the parent to confirm
/// the name actually changed rather than trusting the HTTP status alone.
pub async fn rename(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    name: &str,
) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Protocol("a name can't be empty".into()));
    }

    let parent = parent_of(item);
    let current = folder_listing(http, root, &params, parent).await?;
    let etag = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .or_else(|| item.etag.clone());

    let url = format!("{}/renameItems", root.trim_end_matches('/'));
    let body = json!({
        "items": [{ "drivewsid": item.drivewsid, "name": name, "etag": etag }],
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Protocol(format!(
            "Drive refused the rename ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let after = folder_listing(http, root, &params, parent).await?;
    match after.iter().find(|i| i.drivewsid == item.drivewsid) {
        Some(found) if found.name == name => Ok(()),
        Some(found) => Err(Error::Protocol(format!(
            "Apple answered success but the name is still {:?}, not {name:?}",
            found.name
        ))),
        None => Err(Error::Protocol(
            "Apple answered success but the item is no longer where it should be".into(),
        )),
    }
}

/// Move an item to Recently Deleted. Apple keeps it there for a time before it is really
/// gone, the same as a Mac's trash.
///
/// Same discipline as [`rename`]: a stale etag is refreshed before the write, and the
/// parent is re-read afterward to confirm the item is genuinely gone from it rather than
/// trusting a 200 that turned out, once, to have silently done nothing.
pub async fn delete(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    let parent = parent_of(item);
    let current = folder_listing(http, root, &params, parent).await?;
    let etag = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .or_else(|| item.etag.clone());

    let url = format!("{}/moveItemsToTrash", root.trim_end_matches('/'));
    let body = json!({
        "items": [{ "drivewsid": item.drivewsid, "etag": etag }],
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Protocol(format!(
            "Drive refused the delete ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let after = folder_listing(http, root, &params, parent).await?;
    if after.iter().any(|i| i.drivewsid == item.drivewsid) {
        return Err(Error::Protocol(
            "Apple answered success but the item is still in the folder".into(),
        ));
    }
    Ok(())
}

/// Duplicate a file as icloud.com does: `POST {docws}/v1/item/copy/{item_id}` with
/// `{info_to_update:{}}`. Apple names the copy as Finder would — "Plan 2.pdf" — in the same
/// folder. `item_id` is not the drivewsid's tail (that answers 404): it is a field of the
/// item in its folder's listing, so the listing is read first. Measured on a real account.
pub async fn duplicate(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    let parent = parent_of(item);
    let url = format!(
        "{}/retrieveItemDetailsInFolders",
        drivews_root.trim_end_matches('/')
    );
    let body = json!([{ "drivewsid": parent, "partialData": false }]);
    let response = http.post(url).query(&params).json(&body).send().await?;
    if response.status().as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    let listing: Value = response
        .json()
        .await
        .map_err(|e| Error::Protocol(format!("Drive listing: {e}")))?;
    let item_id = listing
        .pointer("/0/items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|i| {
                i.get("drivewsid").and_then(Value::as_str) == Some(item.drivewsid.as_str())
            })
        })
        .and_then(|i| i.get("item_id").and_then(Value::as_str))
        .ok_or_else(|| {
            Error::Refused("That item is no longer in its folder, so it was not duplicated.".into())
        })?
        .to_string();
    let url = format!(
        "{}/v1/item/copy/{item_id}",
        docws_root.trim_end_matches('/')
    );
    let response = http
        .post(url)
        .query(&params)
        .json(&json!({ "info_to_update": {} }))
        .send()
        .await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Protocol(format!(
            "Drive refused the copy ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    Ok(())
}

/// Drive's Recently Deleted, as icloud.com lists it: a folder Apple names `TRASH_ROOT`.
pub const TRASH: &str = "TRASH_ROOT";

/// What is in Recently Deleted now. Read-only.
pub async fn trash(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<DriveItem>> {
    folder_listing(http, root, &params, TRASH).await
}

/// Put an item back where it was deleted from (`putBackItemsFromTrash`), or delete it for
/// good (`deleteItems`), as icloud.com's Recover and Delete Now do. Either way the trash is
/// re-read afterwards and the item must have left it: a 200 alone has been seen to do nothing.
async fn out_of_trash(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    endpoint: &str,
    what: &str,
) -> Result<()> {
    let current = folder_listing(http, root, &params, TRASH).await?;
    let Some(etag) = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .map(|i| i.etag.clone().or_else(|| item.etag.clone()))
    else {
        return Err(Error::Refused(format!(
            "That item is no longer in Recently Deleted, so it could not be {what}."
        )));
    };
    let url = format!("{}/{endpoint}", root.trim_end_matches('/'));
    let body = json!({ "items": [{ "drivewsid": item.drivewsid, "etag": etag }] });
    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Protocol(format!(
            "Drive refused that ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    let after = folder_listing(http, root, &params, TRASH).await?;
    if after.iter().any(|i| i.drivewsid == item.drivewsid) {
        return Err(Error::Refused(format!(
            "iCloud answered, but the item is still in Recently Deleted, so it was not {what}."
        )));
    }
    Ok(())
}

/// Recover: back to the folder it was deleted from.
pub async fn put_back(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    out_of_trash(
        http,
        root,
        params,
        item,
        "putBackItemsFromTrash",
        "recovered",
    )
    .await
}

/// Delete Now: gone for good. Only ever from Recently Deleted, and only when asked.
pub async fn delete_forever(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    out_of_trash(http, root, params, item, "deleteItems", "deleted").await
}

/// Move an item into a different folder.
///
/// Verified the same way: a fresh etag before the write, and the destination re-read
/// afterward to confirm the item actually arrived there.
pub async fn move_to(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    destination_id: &str,
) -> Result<()> {
    let parent = parent_of(item);
    let current = folder_listing(http, root, &params, parent).await?;
    let etag = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .or_else(|| item.etag.clone());

    // Unlike rename and delete, moveItems 400s outright without a clientId on the item —
    // measured against the real service, not documented anywhere.
    let url = format!("{}/moveItems", root.trim_end_matches('/'));
    let body = json!({
        "items": [{
            "drivewsid": item.drivewsid,
            "etag": etag,
            "clientId": uuid::Uuid::new_v4().to_string(),
        }],
        "destinationDrivewsId": destination_id,
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Protocol(format!(
            "Drive refused the move ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let arrived = folder_listing(http, root, &params, destination_id).await?;
    if !arrived.iter().any(|i| i.drivewsid == item.drivewsid) {
        return Err(Error::Protocol(
            "Apple answered success but the item never arrived at the destination".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod checksum_tests {
    use super::*;
    use serde_json::json;

    fn pattern(size: usize) -> Vec<u8> {
        (0..size)
            .map(|i| u8::try_from(i * 31 % 251).unwrap())
            .collect()
    }

    /// Apple's own answers for these exact bytes, measured on real uploads.
    #[test]
    fn the_checksum_is_apples() {
        for (size, apple) in [
            (37, "AcbV5Q1Z72Y4XYc0UObScQwypTml"),
            (3 * 1024 * 1024, "AbnSbKOQ813hyTYHFLl6Dh/oYsl2"),
        ] {
            let bytes = pattern(size);
            let mut whole = AppleChecksum::default();
            whole.update(&bytes);
            assert_eq!(whole.finish(), apple, "{size} bytes");
            // Fed in pieces as a stream feeds it, the answer is the same.
            let mut pieces = AppleChecksum::default();
            for chunk in bytes.chunks(256 * 1024 + 7) {
                pieces.update(chunk);
            }
            assert_eq!(pieces.finish(), apple, "{size} bytes in pieces");
        }
    }

    fn filed(signature: &str, size: u64, code: i64) -> Value {
        json!({ "results": [{ "document": {
            "document_id": "06502863-6910-440B-A1B9-29536C125463",
            "name": "photo 2.jpg", "size": size,
            "data": { "signature": signature },
            "status": { "status_code": code, "error_message": "OK" }
        }}]})
    }

    /// Found by the id Apple gave it, whatever it was renamed to.
    #[test]
    fn a_filed_document_is_accepted_only_when_it_is_the_file_sent() {
        let sig = "AcbV5Q1Z72Y4XYc0UObScQwypTml";
        assert_eq!(
            filed_document(&filed(sig, 37, 0), sig, 37, "photo.jpg").unwrap(),
            "FILE::com.apple.CloudDocs::06502863-6910-440B-A1B9-29536C125463"
        );
        assert!(filed_document(&filed("Aother", 37, 0), sig, 37, "photo.jpg").is_err());
        assert!(filed_document(&filed(sig, 36, 0), sig, 37, "photo.jpg").is_err());
        assert!(filed_document(&filed(sig, 37, 1), sig, 37, "photo.jpg").is_err());
        assert!(filed_document(&json!({}), sig, 37, "photo.jpg").is_err());
    }
}

#[cfg(test)]
mod created_folder_tests {
    use super::*;
    use serde_json::json;

    /// The two replies Apple gives, both measured: a made folder, and a second create of the
    /// same name.
    #[test]
    fn a_declined_folder_is_reported_as_declined() {
        let made = json!({ "folders": [{ "drivewsid": "FOLDER::com.apple.CloudDocs::A",
            "name": "Wallp", "type": "FOLDER", "status": "OK", "etag": "2y8" }] });
        assert_eq!(created_folder(&made, "Wallp").unwrap().name, "Wallp");

        let declined = json!({ "folders": [{ "clientId": "7222bf20", "status": "UNKNOWN" }] });
        let message = created_folder(&declined, "Wallp").unwrap_err().to_string();
        assert!(message.contains("didn't create"), "{message}");
        assert!(!message.contains("created the folder"), "{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Value {
        json!([{
            "drivewsid": "FOLDER::com.apple.CloudDocs::root",
            "items": [
                { "drivewsid": "FOLDER::com.apple.CloudDocs::abc", "parentId": "FOLDER::com.apple.CloudDocs::root",
                  "name": "Downloads", "type": "FOLDER", "directChildrenCount": 3,
                  "dateCreated": "2026-08-01T10:00:00Z", "etag": "e1", "size": -1 },
                { "drivewsid": "FILE::com.apple.CloudDocs::def", "parentId": "FOLDER::com.apple.CloudDocs::root",
                  "name": "report", "extension": "pdf", "type": "FILE", "size": 20480,
                  "dateModified": 1757000000000_i64, "etag": "e2" },
                { "drivewsid": "APPLIBRARY::com.apple.Keynote::x", "name": "Keynote",
                  "type": "APP_LIBRARY", "size": -1 }
            ]
        }])
    }

    #[test]
    fn a_folder_listing_is_read() {
        let items = parse_folder(&response());
        assert_eq!(items.len(), 3);

        let folder = &items[0];
        assert_eq!(folder.name, "Downloads");
        assert!(folder.is_container());
        assert_eq!(folder.child_count, Some(3));
        assert!(folder.size.is_none(), "-1 is not a size");
        assert!(folder.created_at.is_some(), "an ISO date should parse");

        let file = &items[1];
        assert_eq!(file.display_name(), "report.pdf");
        assert_eq!(file.size, Some(20480));
        assert_eq!(
            file.modified_at,
            Some(1_757_000_000),
            "milliseconds become seconds"
        );

        assert!(
            items[2].is_container(),
            "an app library opens like a folder"
        );
    }

    #[test]
    fn an_item_without_an_identifier_is_skipped() {
        let body = json!([{ "items": [{ "name": "ghost", "type": "FILE" }] }]);
        assert!(parse_folder(&body).is_empty());
    }

    #[test]
    fn an_empty_or_odd_response_is_an_empty_folder() {
        assert!(parse_folder(&json!([])).is_empty());
        assert!(parse_folder(&json!({})).is_empty());
        assert!(parse_folder(&json!([{ "drivewsid": "x" }])).is_empty());
    }

    #[tokio::test]
    async fn syncing_a_folder_stores_its_contents() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/retrieveItemDetailsInFolders$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response()))
            .mount(&server)
            .await;

        let mut library = quince_core::Library::in_memory().unwrap();
        let stored = sync_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            quince_core::drive::ROOT,
        )
        .await
        .unwrap();

        assert_eq!(stored, 3);
        let children = library.drive_children(quince_core::drive::ROOT).unwrap();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].name, "Downloads", "folders sort first");
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn file() -> DriveItem {
        DriveItem {
            drivewsid: "FILE::com.apple.CloudDocs::ABC-123".into(),
            parent_id: Some("FOLDER::com.apple.CloudDocs::root".into()),
            name: "report".into(),
            extension: Some("pdf".into()),
            kind: "FILE".into(),
            size: Some(12),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: Some("e1".into()),
        }
    }

    async fn sent(server: &MockServer, matching: &str) -> Value {
        let request = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path().ends_with(matching))
            .unwrap_or_else(|| panic!("no request to {matching}"));
        serde_json::from_slice(&request.body).unwrap()
    }

    #[test]
    fn a_bare_docwsid_is_the_last_segment() {
        assert_eq!(docwsid_of("FILE::com.apple.CloudDocs::ABC-123"), "ABC-123");
        assert_eq!(docwsid_of("no-separator"), "no-separator");
    }

    #[tokio::test]
    async fn a_download_url_comes_from_the_docws_host_not_drivews() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": "https://cvws.icloud-content.com/x" }
            })))
            .mount(&server)
            .await;

        let url = download_url(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap();
        assert_eq!(url, "https://cvws.icloud-content.com/x");

        let request: Request = server.received_requests().await.unwrap().remove(0);
        let query: std::collections::HashMap<_, _> = request.url.query_pairs().collect();
        assert_eq!(
            query["document_id"], "ABC-123",
            "the bare id, not the full drivewsid"
        );
    }

    #[tokio::test]
    async fn a_reply_with_no_url_is_a_clear_error_not_a_panic() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        assert!(
            download_url(&reqwest::Client::new(), &server.uri(), vec![], &file())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_empty_folder_name_is_refused_before_anything_is_sent() {
        let server = MockServer::start().await;
        assert!(create_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            "   ",
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_created_folder_is_returned_as_a_real_item() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/createFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "folders": [{
                    "drivewsid": "FOLDER::com.apple.CloudDocs::NEW",
                    "parentId": "FOLDER::com.apple.CloudDocs::root",
                    "name": "Scans",
                    "type": "FOLDER",
                    "etag": "e9"
                }]
            })))
            .mount(&server)
            .await;

        let created = create_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            "Scans",
        )
        .await
        .unwrap();
        assert_eq!(created.name, "Scans");
        assert_eq!(created.kind, "FOLDER");

        let body = sent(&server, "/createFolders").await;
        assert_eq!(
            body["destinationDrivewsId"],
            "FOLDER::com.apple.CloudDocs::root"
        );
        assert_eq!(body["folders"][0]["name"], "Scans");
    }

    /// A response shaped like a real `retrieveItemDetailsInFolders` reply, holding `item`
    /// (possibly under a different, current etag) as the sole child of its folder.
    fn listing_of(folder_id: &str, item: &DriveItem, etag: &str) -> serde_json::Value {
        serde_json::json!([{
            "drivewsid": folder_id,
            "items": [{
                "drivewsid": item.drivewsid,
                "parentId": folder_id,
                "name": item.name,
                "extension": item.extension,
                "type": item.kind,
                "etag": etag,
            }]
        }])
    }

    fn empty_listing_of(folder_id: &str) -> serde_json::Value {
        serde_json::json!([{ "drivewsid": folder_id, "items": [] }])
    }

    #[tokio::test]
    async fn renaming_uses_the_etag_apple_currently_holds_not_a_stale_one() {
        let server = MockServer::start().await;
        // The caller's copy says "e1"; Apple has moved on to "e2" since — a rename sent
        // with "e1" is exactly the write that used to silently do nothing.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e2",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/renameItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        // The confirming re-read after the write: the rename really landed.
        let mut renamed = file();
        renamed.name = "final report".into();
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &renamed,
                "e3",
            )))
            .mount(&server)
            .await;

        rename(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "final report",
        )
        .await
        .unwrap();

        let body = sent(&server, "/renameItems").await;
        assert_eq!(body["items"][0]["name"], "final report");
        assert_eq!(
            body["items"][0]["etag"], "e2",
            "the fresh etag, not the stale e1"
        );
    }

    /// The whole point of this rewrite: a 200 that did not actually change anything must
    /// surface as a failure, not as success.
    #[tokio::test]
    async fn a_rename_that_silently_did_nothing_is_reported_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/renameItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;

        let err = rename(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "final report",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("still"), "{err}");
    }

    #[tokio::test]
    async fn deleting_moves_to_trash_rather_than_destroying() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItemsToTrash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        // Confirmed gone: the parent no longer lists it.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(empty_listing_of("FOLDER::com.apple.CloudDocs::root")),
            )
            .mount(&server)
            .await;

        delete(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap();

        let body = sent(&server, "/moveItemsToTrash").await;
        assert_eq!(
            body["items"][0]["drivewsid"],
            "FILE::com.apple.CloudDocs::ABC-123"
        );
    }

    /// Recover sends the item, with the etag Recently Deleted holds now, and is only a success
    /// once the item has left Recently Deleted.
    #[tokio::test]
    async fn recovering_puts_back_and_checks_it_left_the_trash() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                TRASH,
                &file(),
                "fresh",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/putBackItemsFromTrash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_listing_of(TRASH)))
            .mount(&server)
            .await;

        put_back(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap();
        let body = sent(&server, "/putBackItemsFromTrash").await;
        assert_eq!(
            body["items"][0]["drivewsid"],
            "FILE::com.apple.CloudDocs::ABC-123"
        );
        assert_eq!(body["items"][0]["etag"], "fresh");
    }

    /// Delete Now that left the item in Recently Deleted is a failure, said so.
    #[tokio::test]
    async fn a_delete_now_that_did_nothing_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                TRASH,
                &file(),
                "e1",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/deleteItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        let err = delete_forever(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("still in Recently Deleted"),
            "{err}"
        );
    }

    /// This is the bug measured against the real account: `moveItemsToTrash` answered 200
    /// while the item was still sitting in the folder, because the etag sent was stale.
    #[tokio::test]
    async fn a_delete_that_left_the_item_in_place_is_reported_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItemsToTrash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;

        let err = delete(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("still in the folder"), "{err}");
    }

    #[tokio::test]
    async fn moving_names_the_destination_folder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        // Confirmed arrived: the destination now lists it.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::DEST",
                &file(),
                "e2",
            )))
            .mount(&server)
            .await;

        move_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "FOLDER::com.apple.CloudDocs::DEST",
        )
        .await
        .unwrap();

        let body = sent(&server, "/moveItems").await;
        assert_eq!(
            body["destinationDrivewsId"],
            "FOLDER::com.apple.CloudDocs::DEST"
        );
        // Measured against the real service: moveItems 400s outright with no clientId on
        // the item, unlike rename or delete, which need none.
        assert!(
            body["items"][0]["clientId"]
                .as_str()
                .is_some_and(|c| !c.is_empty()),
            "moveItems requires a clientId per item"
        );
    }

    #[tokio::test]
    async fn a_move_that_never_arrived_is_reported_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_listing_of("x")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;

        let err = move_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "FOLDER::com.apple.CloudDocs::DEST",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("never arrived"), "{err}");
    }

    /// A 450 is Apple asking for a fresh sign-in — the same rule every write path in this
    /// app follows, so it must not be reported as a plain protocol error.
    #[tokio::test]
    async fn a_450_anywhere_means_reauth_not_a_generic_failure() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*"))
            .respond_with(ResponseTemplate::new(450))
            .mount(&server)
            .await;
        assert!(matches!(
            rename(&reqwest::Client::new(), &server.uri(), vec![], &file(), "x").await,
            Err(Error::ReauthRequired)
        ));
    }

    #[tokio::test]
    async fn downloading_to_disk_writes_the_exact_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": format!("{}/bytes", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bytes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello drive".to_vec()))
            .mount(&server)
            .await;

        let dir = tempdir();
        let destination = dir.join("report.pdf");
        download_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            &destination,
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"hello drive");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("quince-drive-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

/// A guess at what kind of file this is, from its name.
///
/// Apple stores whatever it is told here and hands it back on download, so a wrong guess shows
/// up later as a file the browser refuses to preview. The list covers what people actually put
/// in a Drive; anything else is declared as bytes, which is honest rather than wrong.
fn content_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("html" | "htm") => "text/html",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("zip") => "application/zip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("pages") => "application/x-iwork-pages-sffpages",
        Some("numbers") => "application/x-iwork-numbers-sffnumbers",
        Some("key") => "application/x-iwork-keynote-sffkey",
        _ => "application/octet-stream",
    }
}

/// Where a document is filed, as `update/documents` wants it: the bare id, or `root`.
fn starting_document_id(folder_id: &str) -> &str {
    if folder_id == quince_core::drive::ROOT {
        "root"
    } else {
        docwsid_of(folder_id)
    }
}

/// The container an item belongs to: the middle of `<TYPE>::<zone>::<id>`.
///
/// Most of a Drive is `com.apple.CloudDocs`, but an app library is its own zone — Keynote's
/// is `com.apple.Keynote`, and a third party's is whatever they registered, such as
/// `iCloud.dk.simonbs.Scriptable`. Everyone has a few of these whether they use the apps or
/// not.
fn zone_of(drivewsid: &str) -> Option<&str> {
    let mut parts = drivewsid.split("::");
    let _kind = parts.next()?;
    let zone = parts.next()?;
    (!zone.is_empty()).then_some(zone)
}

/// The one zone a file uploaded from this machine belongs in.
const DOCUMENTS_ZONE: &str = "com.apple.CloudDocs";

/// Apple's checksum of a file's bytes, as its storage service reports it back.
///
/// Measured against real uploads of 37 bytes and 3 MiB: the receipt's `fileChecksum` is a
/// `0x01` byte and then SHA-1 of the bytes behind a fixed salt, base64. Knowing it means an
/// upload can be proven intact — the bytes Apple holds are the ones on disk, not merely the
/// same number of them.
pub struct AppleChecksum(sha1::Sha1);

const CHECKSUM_SALT: &[u8] = b"com.apple.XattrObjectSalt\0com.apple.DataObjectSalt\0";

impl Default for AppleChecksum {
    fn default() -> Self {
        use sha1::Digest;
        let mut hash = sha1::Sha1::new();
        hash.update(CHECKSUM_SALT);
        Self(hash)
    }
}

impl AppleChecksum {
    pub fn update(&mut self, bytes: &[u8]) {
        use sha1::Digest;
        self.0.update(bytes);
    }

    #[must_use]
    pub fn finish(self) -> String {
        use base64::Engine;
        use sha1::Digest;
        let mut out = vec![0x01u8];
        out.extend_from_slice(&self.0.finalize());
        base64::engine::general_purpose::STANDARD.encode(out)
    }
}

/// How far one upload has got, for a progress bar.
pub type Progress = std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Milliseconds since the epoch for a file time, when the filesystem keeps one.
fn millis(time: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    time.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

/// Upload one file into a folder, and prove it arrived whole.
///
/// Three steps, as iCloud.com does it: ask for somewhere to put the bytes, send them, then
/// file them under a name. Every one is checked rather than trusted — the receipt must carry
/// the checksum of exactly the bytes that were sent, and the filed document must carry it
/// again at the same size — and the folder is read back for the document by its id. A file
/// renamed on the way in because the name was taken ("photo 2.jpg") is found by id too, where
/// matching by name reported it missing, and an older file of the same name could stand in
/// for a failed one.
pub async fn upload(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
    path: &std::path::Path,
    progress: Progress,
) -> Result<DriveItem> {
    // A busy Drive at any step is met again from the start — a new slot, the bytes again —
    // so a file half sent is never filed, and a moment's overload costs nothing.
    let mut attempt = 0;
    loop {
        attempt += 1;
        let result = upload_once(
            http,
            docws_root,
            drivews_root,
            params.clone(),
            folder_id,
            path,
            progress.clone(),
        )
        .await;
        match result {
            Err(Error::Refused(ref m)) if m == BUSY && attempt < 3 => {
                tokio::time::sleep(std::time::Duration::from_secs(3 * attempt)).await;
            }
            other => return other,
        }
    }
}

/// What a busy Drive is said to be, in one place so a retry can know it.
const BUSY: &str = "iCloud Drive is busy right now — try this one again in a minute.";

async fn upload_once(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
    path: &std::path::Path,
    progress: Progress,
) -> Result<DriveItem> {
    use tokio::io::AsyncReadExt;

    if folder_id.trim().is_empty() {
        return Err(Error::Protocol("no folder was named to put this in".into()));
    }
    // An app library belongs to the app that made it, and `update/documents` names the folder
    // by a bare id with no zone beside it — so a file meant for one would be filed by an id
    // that means something different in the documents zone, or nothing at all. Refusing is the
    // honest answer; quietly putting someone's file somewhere else is not.
    if zone_of(folder_id).is_some_and(|zone| zone != DOCUMENTS_ZONE) {
        return Err(Error::Protocol(
            "that folder belongs to an app, and only the app can put things in it".into(),
        ));
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Protocol("that file has no name to give it".into()))?
        .to_string();
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not read {name}: {e}")))?;
    let size = meta.len();
    let docws = docws_root.trim_end_matches('/');

    // 1. Somewhere to put the bytes.
    let offered: Value = post_json(
        http,
        &format!("{docws}/ws/com.apple.CloudDocs/upload/web"),
        &params,
        &json!({
            "filename": name,
            "type": "FILE",
            "content_type": content_type(&name),
            "size": size,
        }),
    )
    .await?;
    let slot = offered
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| Error::Protocol("Apple offered nowhere to put the file".into()))?;
    let url = slot
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("Apple's upload slot carried no address".into()))?;
    let document_id = slot
        .get("document_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("Apple's upload slot carried no document id".into()))?;

    // 2. The bytes, straight off disk and never all in memory, checksummed and counted on the
    //    way past so what was sent is known exactly.
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not open {name}: {e}")))?;
    let checksum = std::sync::Arc::new(std::sync::Mutex::new(Some(AppleChecksum::default())));
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stream = futures_util::stream::try_unfold(
        (file, checksum.clone(), sent.clone(), progress.clone()),
        move |(mut file, checksum, sent, progress)| async move {
            let mut buf = vec![0u8; 256 * 1024];
            let n = file.read(&mut buf).await?;
            if n == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            buf.truncate(n);
            if let Some(hash) = checksum.lock().ok().as_mut().and_then(|h| h.as_mut()) {
                hash.update(&buf);
            }
            let so_far = sent.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed) + n as u64;
            progress(so_far, size);
            Ok(Some((buf, (file, checksum, sent, progress))))
        },
    );
    let part =
        reqwest::multipart::Part::stream_with_length(reqwest::Body::wrap_stream(stream), size)
            .file_name(name.clone())
            .mime_str(content_type(&name))
            .map_err(|e| {
                Error::Protocol(format!("{name} has a content type Apple refused: {e}"))
            })?;
    let response = http
        .post(url)
        .multipart(reqwest::multipart::Form::new().part("files", part))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if matches!(status.as_u16(), 429 | 500..=599) {
        return Err(Error::Refused(BUSY.into()));
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Apple would not take the file ({status}): {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    let local = checksum
        .lock()
        .ok()
        .and_then(|mut h| h.take())
        .map(AppleChecksum::finish)
        .ok_or_else(|| Error::Protocol(format!("{name} could not be checksummed")))?;
    if sent.load(std::sync::atomic::Ordering::Relaxed) != size {
        return Err(Error::Protocol(format!(
            "{name} changed on disk while it was being sent, so nothing was filed"
        )));
    }
    let receipt: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Apple's upload reply was not JSON: {e}")))?;
    let single = receipt
        .get("singleFile")
        .ok_or_else(|| Error::Protocol("Apple took the bytes but described nothing".into()))?;
    if single.get("fileChecksum").and_then(Value::as_str) != Some(local.as_str()) {
        return Err(Error::Protocol(format!(
            "the bytes Apple received for {name} are not the file's, so it was not filed"
        )));
    }
    let of = |key: &str| single.get(key).cloned().unwrap_or(Value::Null);

    // 3. File it under a name, with the file's own dates — as Finder keeps them — rather than
    //    the moment it happened to be uploaded.
    let now = millis(Ok(std::time::SystemTime::now())).unwrap_or(0);
    let modified = millis(meta.modified()).unwrap_or(now);
    let created = millis(meta.created()).unwrap_or(modified);
    let filed = post_json(
        http,
        &format!("{docws}/ws/com.apple.CloudDocs/update/documents"),
        &params,
        &json!({
            // Apple renames rather than overwrites when the name is taken, which is what a
            // file manager does and what a person expects: nothing already there is lost.
            "allow_conflict": true,
            "btime": created,
            "mtime": modified,
            "command": "add_file",
            "create_short_guid": true,
            "document_id": document_id,
            "file_flags": { "is_executable": false, "is_hidden": false, "is_writable": true },
            "path": { "starting_document_id": starting_document_id(folder_id), "path": name },
            "data": {
                "receipt": of("receipt"),
                "reference_signature": of("referenceChecksum"),
                "signature": of("fileChecksum"),
                "wrapping_key": of("wrappingKey"),
                "size": of("size"),
            },
        }),
    )
    .await?;
    let stored = filed_document(&filed, &local, size, &name)?;

    // Apple answers 200 to writes it did not make. The folder is the only witness that counts.
    let after = folder_listing(http, drivews_root, &params, folder_id).await?;
    after
        .into_iter()
        .find(|i| i.drivewsid == stored)
        .ok_or_else(|| {
            Error::Protocol(format!(
                "Apple accepted {name} but the folder does not hold it"
            ))
        })
}

/// The id of the document an `add_file` reply describes, once it is shown to be the file sent.
fn filed_document(reply: &Value, checksum: &str, size: u64, name: &str) -> Result<String> {
    let document = reply
        .pointer("/results/0/document")
        .ok_or_else(|| Error::Protocol(format!("Apple filed {name} without describing it")))?;
    let ok = document
        .pointer("/status/status_code")
        .and_then(Value::as_i64)
        == Some(0);
    let same_bytes = document.pointer("/data/signature").and_then(Value::as_str) == Some(checksum);
    let same_size = document.get("size").and_then(Value::as_u64) == Some(size);
    if !(ok && same_bytes && same_size) {
        return Err(Error::Protocol(format!(
            "what Apple filed as {name} is not the file that was sent"
        )));
    }
    let id = document
        .get("document_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol(format!("Apple filed {name} without an id")))?;
    Ok(format!("FILE::{DOCUMENTS_ZONE}::{id}"))
}

/// A JSON POST to Drive that fails loudly rather than returning something unusable.
async fn post_json(
    http: &reqwest::Client,
    url: &str,
    params: &[(String, String)],
    body: &Value,
) -> Result<Value> {
    // A busy Drive (503, 429) asks to be asked again shortly. Ninety files going up three at
    // a time met exactly that, and every one that met it was reported as not uploaded. It is
    // waited out, as the server asks, up to three times; only one still busy is reported.
    let mut tries: u64 = 0;
    let (status, text) = loop {
        tries += 1;
        let response = http
            .post(url)
            .query(params)
            .header("Referer", "https://www.icloud.com/")
            .json(body)
            .send()
            .await?;
        let status = response.status();
        let wait = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let text = response.text().await.unwrap_or_default();
        if !matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) || tries >= 3 {
            break (status, text);
        }
        let asked = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("retryAfter").and_then(Value::as_u64))
            .or(wait)
            .unwrap_or(2 * tries);
        tokio::time::sleep(std::time::Duration::from_secs(asked.clamp(1, 30))).await;
    };
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        eprintln!(
            "drive: {url} answered {status}: {}",
            text.chars().take(300).collect::<String>()
        );
        return Err(Error::Refused(match status.as_u16() {
            429 | 500..=599 => {
                "iCloud Drive is busy right now — try this one again in a minute.".into()
            }
            409 => "Something with that name is already there.".into(),
            403 => "iCloud Drive won't let this be changed.".into(),
            code => format!("iCloud Drive turned this down (status {code})."),
        }));
    }
    serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))
}

#[cfg(test)]
mod busy_tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Ninety files three at a time met a busy Drive, and each was reported as not uploaded.
    #[tokio::test]
    async fn a_busy_drive_is_waited_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })),
            )
            .mount(&server)
            .await;
        let v = post_json(
            &reqwest::Client::new(),
            &server.uri(),
            &[],
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        assert_eq!(v["ok"], true);
    }

    #[tokio::test]
    async fn a_drive_that_stays_busy_says_so_plainly() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
            .mount(&server)
            .await;
        let e = post_json(
            &reqwest::Client::new(),
            &server.uri(),
            &[],
            &serde_json::json!({}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "iCloud Drive is busy right now — try this one again in a minute."
        );
    }
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    #[test]
    fn a_file_is_declared_as_what_it_is() {
        assert_eq!(content_type("holiday.JPG"), "image/jpeg");
        assert_eq!(content_type("notes.md"), "text/plain");
        assert_eq!(
            content_type("budget.numbers"),
            "application/x-iwork-numbers-sffnumbers"
        );
        assert_eq!(content_type("archive.tar.gz"), "application/octet-stream");
        assert_eq!(content_type("noextension"), "application/octet-stream");
        assert_eq!(content_type(""), "application/octet-stream");
    }

    #[test]
    fn an_item_names_the_container_it_belongs_to() {
        assert_eq!(
            zone_of("FOLDER::com.apple.CloudDocs::root"),
            Some("com.apple.CloudDocs")
        );
        assert_eq!(
            zone_of("FOLDER::iCloud.dk.simonbs.Scriptable::documents"),
            Some("iCloud.dk.simonbs.Scriptable"),
            "a third party's library is a zone like any other"
        );
        assert_eq!(
            zone_of("FOLDER::iCloud.is.workflow.my.workflows::documents"),
            Some("iCloud.is.workflow.my.workflows")
        );
        assert_eq!(zone_of("nonsense"), None);
        assert_eq!(zone_of("FOLDER::"), None);
    }

    /// The root has a name of its own in the listing and a different one in a document path.
    #[test]
    fn the_root_is_called_root_when_filing_a_document() {
        assert_eq!(starting_document_id(quince_core::drive::ROOT), "root");
        assert_eq!(
            starting_document_id("FOLDER::com.apple.CloudDocs::7B3F-ABCD"),
            "7B3F-ABCD"
        );
    }
}

#[cfg(test)]
mod app_library_tests {
    use super::*;

    /// Everyone has app libraries — Keynote, Shortcuts, whatever third-party apps they use —
    /// and a file meant for one would be filed by a bare id that means something else in the
    /// documents zone. Refusing beats putting someone's file somewhere they will not find it.
    #[tokio::test]
    async fn a_file_is_not_uploaded_into_an_app_library() {
        let file = std::env::temp_dir().join("quince-zone-test.txt");
        std::fs::write(&file, b"x").unwrap();
        for folder in [
            "FOLDER::com.apple.Keynote::documents",
            "FOLDER::iCloud.is.workflow.my.workflows::documents",
            "FOLDER::iCloud.dk.simonbs.Scriptable::documents",
        ] {
            let err = upload(
                &reqwest::Client::new(),
                "http://127.0.0.1:1",
                "http://127.0.0.1:1",
                vec![],
                folder,
                &file,
                std::sync::Arc::new(|_, _| {}),
            )
            .await
            .expect_err("an app's own library is not somewhere to put a file");
            assert!(
                err.to_string().contains("only the app"),
                "the reason has to be the zone, not a network failure: {err}"
            );
        }
        std::fs::remove_file(&file).ok();
    }

    /// A busy Drive while the bytes go up: the file is tried again from a fresh slot, up to
    /// three times, and only then reported busy — not dropped at the first refusal.
    #[tokio::test]
    async fn a_file_refused_as_busy_is_sent_again() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/upload/web$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "url": format!("{}/content", server.uri()), "document_id": "D1" }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let file = std::env::temp_dir().join("quince-busy-test.txt");
        std::fs::write(&file, b"hello").unwrap();
        let err = upload(
            &reqwest::Client::new(),
            &server.uri(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            &file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .unwrap_err();
        let slots = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/upload/web"))
            .count();
        assert_eq!(slots, 3, "three tries, each from a fresh slot");
        assert_eq!(err.to_string(), BUSY);
        std::fs::remove_file(&file).ok();
    }

    /// The documents zone is everything else, and must still take a file.
    #[tokio::test]
    async fn the_ordinary_drive_is_not_refused() {
        let file = std::env::temp_dir().join("quince-zone-ok.txt");
        std::fs::write(&file, b"x").unwrap();
        let err = upload(
            &reqwest::Client::new(),
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            vec![],
            quince_core::drive::ROOT,
            &file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .expect_err("nothing is listening, so this fails on the network");
        assert!(
            !err.to_string().contains("only the app"),
            "the documents zone must get as far as trying: {err}"
        );
        std::fs::remove_file(&file).ok();
    }
}

#[cfg(test)]
mod download_zone_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn file(drivewsid: &str) -> DriveItem {
        DriveItem {
            drivewsid: drivewsid.into(),
            parent_id: None,
            name: "Morning".into(),
            extension: Some("shortcut".into()),
            kind: "FILE".into(),
            size: Some(10),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
        }
    }

    /// A file in an app library is asked for under that library's zone. Under the ordinary
    /// Drive zone Apple answers "Could not find document" — measured against a Shortcuts file.
    #[tokio::test]
    async fn a_download_asks_for_the_file_under_its_own_zone() {
        let server = MockServer::start().await;
        for zone in ["iCloud.is.workflow.my.workflows", "com.apple.CloudDocs"] {
            Mock::given(method("GET"))
                .and(path(format!("/ws/{zone}/download/by_id")))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "data_token": { "url": format!("https://x/{zone}") } }),
                ))
                .mount(&server)
                .await;
        }
        let http = reqwest::Client::new();
        let got = download_url(
            &http,
            &server.uri(),
            vec![],
            &file("FILE::iCloud.is.workflow.my.workflows::ABC"),
        )
        .await
        .unwrap();
        assert!(got.ends_with("iCloud.is.workflow.my.workflows"), "{got}");

        let ordinary = download_url(
            &http,
            &server.uri(),
            vec![],
            &file("FILE::com.apple.CloudDocs::XYZ"),
        )
        .await
        .unwrap();
        assert!(ordinary.ends_with("com.apple.CloudDocs"), "{ordinary}");
    }
}

/// The account's iCloud storage, as icloud.com's "X GB available" reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// `POST {setup}/storageUsageInfo`, no body — what icloud.com's shell asks. Measured on a real
/// account: `storageUsageInfo.{usedStorageInBytes,totalStorageInBytes}`.
pub async fn storage(
    http: &reqwest::Client,
    setup_root: &str,
    params: Vec<(String, String)>,
) -> Result<Storage> {
    let url = format!("{}/storageUsageInfo", setup_root.trim_end_matches('/'));
    let response = http.post(url).query(&params).send().await?;
    let status = response.status();
    if status.as_u16() == 450 || status.as_u16() == 421 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!("storage returned {status}")));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| Error::Protocol(format!("storage: {e}")))?;
    let n = |k: &str| {
        body.pointer(&format!("/storageUsageInfo/{k}"))
            .and_then(Value::as_u64)
    };
    match (n("usedStorageInBytes"), n("totalStorageInBytes")) {
        (Some(used_bytes), Some(total_bytes)) => Ok(Storage {
            used_bytes,
            total_bytes,
        }),
        _ => Err(Error::Protocol("storage: no usage in the answer".into())),
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The shape measured on a real account.
    #[tokio::test]
    async fn storage_is_read_from_apples_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/storageUsageInfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "storageUsageInfo": { "compStorageInBytes": 0, "usedStorageInBytes": 182_196_857_u64,
                    "totalStorageInBytes": 5_368_709_120_u64, "commerceStorageInBytes": 0 },
                "quotaStatus": { "overQuota": false }
            })))
            .mount(&server)
            .await;
        let got = storage(&reqwest::Client::new(), &server.uri(), vec![])
            .await
            .unwrap();
        assert_eq!(
            got,
            Storage {
                used_bytes: 182_196_857,
                total_bytes: 5_368_709_120
            }
        );
    }
}
