//! What changed in iCloud Drive, item by item, from CloudKit's own change feed.
//!
//! Apple's push socket says nothing when Drive changes (measured 2026-10-29), and Drive's web
//! service only lists a folder at a time, so a change on the phone could only be found by
//! listing folders again and again. CloudKit holds Drive too, in the `com.apple.clouddocs`
//! container (measured live, 2026-10-29):
//!
//! - its `changes/database` token moves within half a second of a change anywhere in Drive;
//! - the `com.apple.CloudDocs` zone's `changes/zone` feed then names each record that changed:
//!   `directory/<id>` for a folder, `documentStructure/<id>` and `documentContent/<id>` for a
//!   file, `<id>` being the same one Drive's own `FOLDER::com.apple.CloudDocs::<id>` carries;
//! - a structure record's `parent` field is a reference to its folder's `directory/<id>`; a
//!   deletion is a move into Recently Deleted's directory, a purge a `deleted` record.
//!
//! Names are encrypted there, so the feed is read for *which* folders changed — never what
//! is in them: those folders are then listed again through Drive's web service, as before.

use crate::{Error, Result};
use serde_json::{json, Value};

/// Drive's container and its main zone.
pub const CONTAINER: &str = "com.apple.clouddocs";
const ZONE: &str = "com.apple.CloudDocs";

/// One record the feed named, as Drive knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// Drive's own identifier for the item.
    pub drivewsid: String,
    /// The folder it is in now, when the record says (a structure record does).
    pub parent: Option<String>,
    /// Purged: no longer anywhere.
    pub gone: bool,
}

/// One page of the feed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Page {
    pub changed: Vec<Changed>,
    /// Where to read from next time.
    pub token: Option<String>,
    /// More waits behind this page.
    pub more: bool,
}

/// Drive's identifier for a record of the main zone, or `None` for anything else it holds.
pub fn drivewsid_of(record_name: &str) -> Option<String> {
    let (kind, id) = record_name.split_once('/')?;
    if id.is_empty() {
        return None;
    }
    let kind = match kind {
        "directory" => "FOLDER",
        "documentStructure" | "documentContent" => "FILE",
        _ => return None,
    };
    Some(format!("{kind}::{ZONE}::{id}"))
}

/// A page of `changes/zone`, as Drive knows its items. The same item named twice (its
/// structure and its content) is told once, with what the structure said of its folder.
pub fn parse_page(body: &Value) -> Result<Page> {
    let zone = body["zones"]
        .get(0)
        .ok_or_else(|| Error::Protocol("changes/zone answered no zone".into()))?;
    if let Some(code) = zone["serverErrorCode"].as_str() {
        return Err(match code {
            "CHANGE_TOKEN_EXPIRED" | "ZONE_NOT_FOUND" | "BAD_REQUEST" => Error::StaleCursor,
            "AUTHENTICATION_REQUIRED" => Error::ReauthRequired,
            other => Error::Protocol(format!("changes/zone answered {other}")),
        });
    }
    let mut changed: Vec<Changed> = Vec::new();
    for record in zone["records"].as_array().into_iter().flatten() {
        let Some(drivewsid) = record["recordName"].as_str().and_then(drivewsid_of) else {
            continue;
        };
        let parent = record["fields"]["parent"]["value"]["recordName"]
            .as_str()
            .and_then(drivewsid_of);
        let gone = record["deleted"].as_bool().unwrap_or(false);
        match changed.iter_mut().find(|c| c.drivewsid == drivewsid) {
            Some(seen) => {
                seen.parent = seen.parent.take().or(parent);
                seen.gone |= gone;
            }
            None => changed.push(Changed {
                drivewsid,
                parent,
                gone,
            }),
        }
    }
    Ok(Page {
        changed,
        token: zone["syncToken"].as_str().map(str::to_string),
        more: zone["moreComing"].as_bool().unwrap_or(false),
    })
}

/// One page of Drive's change feed from `token` (from the start with none). Read-only; asks
/// only for the `parent` field, so a page is small whatever the files hold.
pub async fn page(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: &[(String, String)],
    token: Option<&str>,
) -> Result<Page> {
    let url = format!(
        "{}/database/1/{CONTAINER}/production/private/changes/zone",
        ckdatabasews_root.trim_end_matches('/')
    );
    let mut zone = json!({
        "zoneID": { "zoneName": ZONE, "zoneType": "REGULAR_CUSTOM_ZONE" },
        "resultsLimit": 200,
        "desiredKeys": ["parent"],
    });
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        zone["syncToken"] = Value::String(t.to_string());
    }
    // As CloudKit JS asks, the way every other CloudKit read here does.
    let mut query = params.to_vec();
    query.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
    query.push(("ckjsVersion".into(), "2.6.1".into()));
    let response = http
        .post(url)
        .query(&query)
        .json(&json!({ "zones": [zone] }))
        .send()
        .await?;
    let status = response.status();
    if matches!(status.as_u16(), 401 | 421 | 450) {
        return Err(Error::ReauthRequired);
    }
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        if text.contains("CHANGE_TOKEN_EXPIRED") || status.as_u16() == 400 && token.is_some() {
            return Err(Error::StaleCursor);
        }
        return Err(Error::Protocol(format!("changes/zone answered {status}")));
    }
    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("changes/zone sent invalid JSON: {e}")))?;
    parse_page(&body)
}

/// The folders to list again for what changed: where each item is now, and where it was —
/// `was_in` answers from what this computer stored — of those this computer has seen
/// (`known`). A folder never seen here is listed when someone opens it, so it is left be.
pub fn folders_to_list(
    changed: &[Changed],
    was_in: impl Fn(&str) -> Option<String>,
    known: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut folders: Vec<String> = Vec::new();
    for c in changed {
        for folder in [c.parent.clone(), was_in(&c.drivewsid)]
            .into_iter()
            .flatten()
        {
            if !folders.contains(&folder) && known(&folder) {
                folders.push(folder);
            }
        }
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

    fn record(name: &str, parent: Option<&str>, deleted: bool) -> Value {
        let mut r = json!({ "recordName": name, "recordType": "structure", "deleted": deleted });
        if let Some(p) = parent {
            r["fields"] = json!({ "parent": { "value": {
                "recordName": p, "action": "VALIDATE",
                "zoneID": { "zoneName": "com.apple.CloudDocs" } } } });
        }
        r
    }

    #[test]
    fn records_are_read_as_drive_knows_them() {
        assert_eq!(drivewsid_of("directory/root").as_deref(), Some(ROOT));
        assert_eq!(
            drivewsid_of("directory/12121DA4").as_deref(),
            Some("FOLDER::com.apple.CloudDocs::12121DA4")
        );
        assert_eq!(
            drivewsid_of("documentStructure/7D85").as_deref(),
            Some("FILE::com.apple.CloudDocs::7D85")
        );
        assert_eq!(
            drivewsid_of("documentContent/7D85").as_deref(),
            Some("FILE::com.apple.CloudDocs::7D85")
        );
        assert_eq!(drivewsid_of("AppContainer/x"), None);
        assert_eq!(drivewsid_of("directory/"), None);
        assert_eq!(drivewsid_of("nothing"), None);
    }

    /// The shape measured live when a folder was made in the top level and then deleted.
    #[test]
    fn a_page_names_each_item_once_with_its_folder() {
        let body = json!({ "zones": [{
            "syncToken": "T2", "moreComing": false,
            "records": [
                record("directory/NEW", Some("directory/root"), false),
                record("directory/root", None, false),
                record("documentContent/F", None, false),
                record("documentStructure/F", Some("directory/NEW"), false),
                record("documentStructure/P", None, true),
                { "recordName": "AppContainer/x", "recordType": "AppContainer" },
            ] }] });
        let page = parse_page(&body).unwrap();
        assert_eq!(page.token.as_deref(), Some("T2"));
        assert!(!page.more);
        assert_eq!(
            page.changed,
            vec![
                Changed {
                    drivewsid: "FOLDER::com.apple.CloudDocs::NEW".into(),
                    parent: Some(ROOT.into()),
                    gone: false
                },
                Changed {
                    drivewsid: ROOT.into(),
                    parent: None,
                    gone: false
                },
                Changed {
                    drivewsid: "FILE::com.apple.CloudDocs::F".into(),
                    parent: Some("FOLDER::com.apple.CloudDocs::NEW".into()),
                    gone: false
                },
                Changed {
                    drivewsid: "FILE::com.apple.CloudDocs::P".into(),
                    parent: None,
                    gone: true
                },
            ]
        );
    }

    #[test]
    fn a_token_apple_no_longer_knows_starts_over() {
        let body = json!({ "zones": [{ "serverErrorCode": "CHANGE_TOKEN_EXPIRED" }] });
        assert!(matches!(parse_page(&body), Err(Error::StaleCursor)));
        assert!(matches!(
            parse_page(&json!({ "zones": [] })),
            Err(Error::Protocol(_))
        ));
    }

    /// A file moved from one folder to another: both are listed again, once each; a folder
    /// this computer never saw is left for when it is opened.
    #[test]
    fn the_folders_listed_again_are_where_it_went_and_where_it_was() {
        let a = "FOLDER::com.apple.CloudDocs::A";
        let b = "FOLDER::com.apple.CloudDocs::B";
        let unseen = "FOLDER::com.apple.CloudDocs::U";
        let changed = vec![
            Changed {
                drivewsid: "FILE::com.apple.CloudDocs::F".into(),
                parent: Some(b.into()),
                gone: false,
            },
            Changed {
                drivewsid: "FILE::com.apple.CloudDocs::G".into(),
                parent: Some(unseen.into()),
                gone: false,
            },
            Changed {
                drivewsid: "FILE::com.apple.CloudDocs::P".into(),
                parent: None,
                gone: true,
            },
        ];
        let was_in = |id: &str| match id {
            "FILE::com.apple.CloudDocs::F" | "FILE::com.apple.CloudDocs::P" => Some(a.to_string()),
            _ => None,
        };
        let known = |id: &str| id == a || id == b;
        assert_eq!(folders_to_list(&changed, was_in, known), vec![b, a]);
    }
}
