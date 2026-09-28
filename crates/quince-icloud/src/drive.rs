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
    let items = parse_folder(&parsed);
    library.put_drive_children(folder_id, &items)?;
    library.mark_synced("drive", None)?;
    Ok(items.len())
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

/// Put a file from this machine into a Drive folder.
///
/// Three separate calls, because that is how Apple does it and skipping any of them leaves a
/// half-made file: ask `upload/web` for somewhere to put the bytes, post the bytes to the URL
/// it hands back, then tell `update/documents` to file the result under a name. Only the third
/// call makes the file exist; the first two leave nothing visible behind if they fail.
///
/// The bytes are streamed off disk rather than read into memory, so putting a film in Drive
/// costs the same as putting a note in it.
///
/// # Errors
/// When the file cannot be read, when Apple refuses any of the three steps, or when the file
/// does not afterwards appear in the folder it was meant for.
pub async fn upload(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
    path: &std::path::Path,
) -> Result<DriveItem> {
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
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not read {name}: {e}")))?
        .len();
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

    // 2. The bytes themselves, straight off disk.
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not open {name}: {e}")))?;
    let part = reqwest::multipart::Part::stream_with_length(reqwest::Body::from(file), size)
        .file_name(name.clone())
        .mime_str(content_type(&name))
        .map_err(|e| Error::Protocol(format!("{name} has a content type Apple refused: {e}")))?;
    let sent = http
        .post(url)
        .multipart(reqwest::multipart::Form::new().part("files", part))
        .send()
        .await?;
    let status = sent.status();
    let text = sent.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Apple would not take the file ({status}): {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    let receipt: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Apple's upload reply was not JSON: {e}")))?;
    let single = receipt
        .get("singleFile")
        .ok_or_else(|| Error::Protocol("Apple took the bytes but described nothing".into()))?;
    let of = |key: &str| single.get(key).cloned().unwrap_or(Value::Null);

    // 3. File it under a name. Until this, the bytes belong to nothing.
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    )
    .unwrap_or(0);
    post_json(
        http,
        &format!("{docws}/ws/com.apple.CloudDocs/update/documents"),
        &params,
        &json!({
            // Apple renames rather than overwrites when the name is taken, which is what a
            // file manager does and what a person expects.
            "allow_conflict": true,
            "btime": now,
            "mtime": now,
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

    // Apple answers 200 to writes it did not make. The folder is the only witness that counts.
    let after = folder_listing(http, drivews_root, &params, folder_id).await?;
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
    after
        .into_iter()
        .find(|i| i.name == name || i.name == stem)
        .ok_or_else(|| {
            Error::Protocol(format!(
                "Apple accepted {name} but the folder does not hold it"
            ))
        })
}

/// A JSON POST to Drive that fails loudly rather than returning something unusable.
async fn post_json(
    http: &reqwest::Client,
    url: &str,
    params: &[(String, String)],
    body: &Value,
) -> Result<Value> {
    let response = http
        .post(url)
        .query(params)
        .header("Referer", "https://www.icloud.com/")
        .json(body)
        .send()
        .await?;
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
    serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))
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
