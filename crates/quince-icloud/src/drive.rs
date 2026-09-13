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
