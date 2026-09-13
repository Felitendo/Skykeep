//! Apple's reminders web service.
//!
//! `/rd/startup` returns the lists and their reminders in one call — unlike calendar, there
//! is no date window to chunk.

use crate::calendar::parse_apple_date;
use crate::{Error, Result};
use quince_core::reminders::{Reminder, ReminderList};
use serde_json::Value;

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Apple's date arrays appear here too. A malformed one loses the date, not the reminder —
/// a task with an unreadable due date is still a task.
fn optional_date(v: &Value, key: &str) -> Option<i64> {
    let raw = v.get(key)?;
    if raw.is_null() {
        return None;
    }
    parse_apple_date(raw, None).ok()
}

fn list_from(v: &Value) -> Option<ReminderList> {
    Some(ReminderList {
        guid: string(v, "guid")?,
        title: string(v, "title").unwrap_or_else(|| "Reminders".into()),
        colour: string(v, "color"),
        ctag: string(v, "ctag"),
        sort_order: v.get("order").and_then(Value::as_i64).unwrap_or(0),
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    })
}

fn reminder_from(v: &Value) -> Option<Reminder> {
    let guid = string(v, "guid")?;
    let list_guid = string(v, "pGuid")?;
    Some(Reminder {
        guid,
        list_guid,
        title: string(v, "title").unwrap_or_else(|| "Untitled".into()),
        notes: string(v, "description"),
        due_at: optional_date(v, "dueDate"),
        completed_at: optional_date(v, "completedDate"),
        priority: v.get("priority").and_then(Value::as_i64).unwrap_or(0),
        etag: string(v, "etag"),
        // Keep Apple's payload verbatim: a write echoes the whole task back, and
        // reconstructing one from parsed columns would drop fields we do not model.
        raw: serde_json::to_string(v).ok(),
    })
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub lists: Vec<ReminderList>,
    pub reminders: Vec<Reminder>,
}

/// Read a `/rd/startup` body.
///
/// Apple returns outstanding reminders under `Reminders` and finished ones under `Completed`,
/// so both are read — a list showing only open items is not the same as Apple's.
pub fn parse_snapshot(body: &Value) -> Snapshot {
    let lists = body
        .get("Collections")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(list_from).collect())
        .unwrap_or_default();

    let mut reminders: Vec<Reminder> = Vec::new();
    for key in ["Reminders", "Completed"] {
        if let Some(arr) = body.get(key).and_then(Value::as_array) {
            reminders.extend(arr.iter().filter_map(reminder_from));
        }
    }

    Snapshot { lists, reminders }
}

pub struct RemindersClient<'a> {
    http: &'a reqwest::Client,
    root: String,
    params: Vec<(String, String)>,
}

impl<'a> RemindersClient<'a> {
    pub fn new(http: &'a reqwest::Client, root: &str, params: Vec<(String, String)>) -> Self {
        Self {
            http,
            root: root.trim_end_matches('/').to_string(),
            params,
        }
    }

    pub async fn startup(&self, timezone: &str) -> Result<Snapshot> {
        let mut query = self.params.clone();
        query.push(("clientVersion".into(), "4.0".into()));
        query.push(("lang".into(), "en-us".into()));
        query.push(("usertz".into(), timezone.into()));

        let response = self
            .http
            .get(format!("{}/rd/startup", self.root))
            .query(&query)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "/rd/startup returned {status}: {}",
                text.chars().take(160).collect::<String>()
            )));
        }
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("/rd/startup returned invalid JSON: {e}")))?;
        Ok(parse_snapshot(&body))
    }
}

/// Reconcile reminders into the local library.
pub async fn sync(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
    timezone: &str,
) -> Result<(usize, usize)> {
    let client = RemindersClient::new(http, root, params);
    let snapshot = client.startup(timezone).await?;

    library.put_reminder_lists(&snapshot.lists)?;

    // Drop reminders whose list Apple did not report: they would fail the foreign key.
    let known: std::collections::HashSet<&str> =
        snapshot.lists.iter().map(|l| l.guid.as_str()).collect();
    let reminders: Vec<_> = snapshot
        .reminders
        .iter()
        .filter(|r| known.contains(r.list_guid.as_str()))
        .cloned()
        .collect();

    library.put_reminders(&reminders)?;
    library.mark_synced("reminders", None)?;
    Ok((snapshot.lists.len(), reminders.len()))
}

/// Mark a reminder done, or put it back.
///
/// Apple wants the whole task echoed back, so this mutates the payload it gave us rather
/// than building a fresh one — anything Apple sent that we do not model survives untouched.
pub async fn set_completed(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    reminder: &Reminder,
    done: bool,
    timezone: &str,
) -> Result<()> {
    let raw = reminder.raw.as_deref().ok_or_else(|| {
        Error::Protocol(
            "this reminder was stored before Quince kept Apple's original payload; \
             refresh once and try again"
                .into(),
        )
    })?;
    let mut task: Value = serde_json::from_str(raw)
        .map_err(|e| Error::Protocol(format!("stored reminder is unreadable: {e}")))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));

    task["completedDate"] = if done {
        crate::calendar::format_apple_date(now, Some(timezone))
    } else {
        Value::Null
    };
    task["lastModifiedDate"] = crate::calendar::format_apple_date(now, Some(timezone));

    let body = serde_json::json!({
        "Reminders": task,
        "ClientState": { "Collections": [] },
    });

    let mut query: Vec<(String, String)> = params.to_vec();
    query.push(("clientVersion".into(), "4.0".into()));
    query.push(("lang".into(), "en-us".into()));
    query.push(("usertz".into(), timezone.into()));
    // Apple's web client tunnels PUT through POST.
    query.push(("methodOverride".into(), "PUT".into()));

    let url = format!(
        "{}/rd/reminders/{}",
        root.trim_end_matches('/'),
        reminder.list_guid
    );
    let response = http.post(url).query(&query).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Protocol(format!(
            "couldn't update that reminder ({status}): {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_snapshot_reads_lists_and_both_states() {
        let body = json!({
            "Collections": [{ "guid": "l1", "title": "Groceries", "color": "#cc73e1", "order": 2 }],
            "Reminders": [
                { "guid": "r1", "pGuid": "l1", "title": "Milk", "priority": 1,
                  "dueDate": [20260913, 2026, 9, 13, 18, 0, 1080] }
            ],
            "Completed": [
                { "guid": "r2", "pGuid": "l1", "title": "Bread",
                  "completedDate": [20260910, 2026, 9, 10, 9, 0, 540] }
            ]
        });
        let snap = parse_snapshot(&body);

        assert_eq!(snap.lists.len(), 1);
        assert_eq!(snap.lists[0].title, "Groceries");
        assert_eq!(snap.lists[0].sort_order, 2);

        assert_eq!(snap.reminders.len(), 2, "finished reminders count too");
        let open = snap.reminders.iter().find(|r| r.guid == "r1").unwrap();
        assert!(!open.is_done());
        assert!(open.due_at.is_some());
        assert_eq!(open.priority, 1);
        assert!(snap
            .reminders
            .iter()
            .find(|r| r.guid == "r2")
            .unwrap()
            .is_done());
    }

    /// A reminder with no due date is the common case and must survive.
    #[test]
    fn a_reminder_without_a_due_date_is_kept() {
        let body = json!({ "Reminders": [{ "guid": "r1", "pGuid": "l1", "title": "Someday" }] });
        let snap = parse_snapshot(&body);
        assert_eq!(snap.reminders.len(), 1);
        assert!(snap.reminders[0].due_at.is_none());
    }

    /// An unreadable date loses the date, not the task.
    #[test]
    fn a_malformed_due_date_does_not_lose_the_reminder() {
        let body = json!({ "Reminders": [
            { "guid": "r1", "pGuid": "l1", "title": "Odd", "dueDate": "not-an-array" }
        ]});
        let snap = parse_snapshot(&body);
        assert_eq!(snap.reminders.len(), 1);
        assert!(snap.reminders[0].due_at.is_none());
    }

    /// The stored payload must be Apple's, verbatim, or a write would drop fields.
    #[test]
    fn apples_payload_is_kept_verbatim() {
        let body = json!({ "Reminders": [{
            "guid": "r1", "pGuid": "l1", "title": "Milk",
            "someFieldWeDoNotModel": { "nested": [1, 2, 3] }
        }]});
        let reminder = &parse_snapshot(&body).reminders[0];
        let raw: Value = serde_json::from_str(reminder.raw.as_ref().unwrap()).unwrap();
        assert_eq!(raw["someFieldWeDoNotModel"]["nested"][2], 3);
    }

    #[tokio::test]
    async fn completing_mutates_the_original_payload() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rd/reminders/l1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let body = json!({ "Reminders": [{
            "guid": "r1", "pGuid": "l1", "title": "Milk", "customField": "keep me"
        }]});
        let reminder = parse_snapshot(&body).reminders.remove(0);

        let http = reqwest::Client::new();
        set_completed(&http, &server.uri(), &[], &reminder, true, "UTC")
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            sent["Reminders"]["customField"], "keep me",
            "unmodelled fields must survive"
        );
        assert!(
            sent["Reminders"]["completedDate"].is_array(),
            "completion must be a date array"
        );
        assert!(requests[0]
            .url
            .query()
            .unwrap()
            .contains("methodOverride=PUT"));
    }

    /// Un-ticking must clear the date, not set it to something.
    #[tokio::test]
    async fn un_completing_clears_the_date() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rd/reminders/l1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let body = json!({ "Completed": [{ "guid": "r1", "pGuid": "l1", "title": "Bread",
            "completedDate": [20260910, 2026, 9, 10, 9, 0, 540] }]});
        let reminder = parse_snapshot(&body).reminders.remove(0);

        set_completed(
            &reqwest::Client::new(),
            &server.uri(),
            &[],
            &reminder,
            false,
            "UTC",
        )
        .await
        .unwrap();

        let requests = server.received_requests().await.unwrap();
        let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(sent["Reminders"]["completedDate"].is_null());
    }

    #[test]
    fn entries_without_identity_are_skipped() {
        let body = json!({ "Reminders": [
            { "title": "no guid" },
            { "guid": "r1", "title": "no list" },
            { "guid": "r2", "pGuid": "l1", "title": "fine" }
        ]});
        assert_eq!(parse_snapshot(&body).reminders.len(), 1);
    }
}

// ---------------------------------------------------------------------------------------
// CloudKit
// ---------------------------------------------------------------------------------------
//
// The legacy /rd endpoints answer with Apple's placeholder once an account's reminders have
// been migrated — a list named "Reminders ⚠️" holding "Where are my reminders?". The real
// records live in the CloudKit `Reminders` zone, so this is the path that actually works.

use crate::cloudkit::{field, ZoneClient};

const CONTAINER: &str = "com.apple.reminders";
const ZONE: &str = "Reminders";

/// Turn one CloudKit `Reminder` record into a reminder.
///
/// Returns `None` for records that are deleted or carry no identity, so tombstones — which
/// are the bulk of the zone — fall away.
fn reminder_from_record(record: &Value) -> Option<Reminder> {
    if field::flag(record, "Deleted") {
        return None;
    }
    let guid = field::record_name(record)?;
    let list_guid = field::reference(record, "List")?;

    let completed = field::flag(record, "Completed");
    Some(Reminder {
        guid,
        list_guid,
        // A reminder with no readable title is still a reminder; showing it beats hiding it.
        title: field::document_text(record, "TitleDocument").unwrap_or_else(|| "Untitled".into()),
        notes: field::document_text(record, "NotesDocument"),
        due_at: field::timestamp_seconds(record, "DueDate"),
        // CloudKit stores completion as a flag; the closest instant it also gives us is the
        // last modification, which is when the tick happened.
        completed_at: completed
            .then(|| field::timestamp_seconds(record, "LastModifiedDate"))
            .flatten()
            .or(if completed { Some(0) } else { None }),
        priority: field::int(record, "Priority").unwrap_or(0),
        etag: record
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        raw: serde_json::to_string(record).ok(),
    })
}

/// Turn a CloudKit list record into a reminder list.
fn list_from_record(record: &Value) -> Option<ReminderList> {
    if field::flag(record, "Deleted") {
        return None;
    }
    let guid = field::record_name(record)?;
    let title = field::string(record, "Name")
        .or_else(|| field::document_text(record, "NameDocument"))
        .or_else(|| field::string(record, "DisplayName"))
        .unwrap_or_else(|| "Reminders".into());
    Some(ReminderList {
        guid,
        title,
        colour: field::string(record, "Color"),
        ctag: None,
        sort_order: field::int(record, "SortOrder").unwrap_or(0),
        enabled: true,
    })
}

/// Read reminders from CloudKit into the local library.
///
/// Returns the number of lists and reminders stored, plus the sync token to resume from.
pub async fn sync_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
    since: Option<&str>,
) -> Result<(usize, usize, Option<String>)> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let (records, token) = client.all_records(ZONE, since).await?;

    let mut lists: Vec<ReminderList> = Vec::new();
    let mut reminders: Vec<Reminder> = Vec::new();

    for record in &records {
        match field::record_type(record).as_deref() {
            Some("Reminder") => reminders.extend(reminder_from_record(record)),
            // Only List records are the lists people made. ListSection is a heading inside
            // one, and SmartList is a built-in filter — Today, Scheduled, Flagged — which
            // carries a SmartListType instead of a Name and appeared as phantom entries
            // called "Reminders" that held nothing.
            Some("List") => lists.extend(list_from_record(record)),
            _ => {}
        }
    }

    // Every list a reminder points at must exist, or storing it fails the foreign key.
    // Apple does not always return the list record in the same page as its reminders.
    for reminder in &reminders {
        if !lists.iter().any(|l| l.guid == reminder.list_guid) {
            lists.push(ReminderList {
                guid: reminder.list_guid.clone(),
                title: "Reminders".into(),
                colour: None,
                ctag: None,
                sort_order: 0,
                enabled: true,
            });
        }
    }

    library.put_reminder_lists(&lists)?;
    library.put_reminders(&reminders)?;
    library.mark_synced("reminders", token.as_deref())?;
    Ok((lists.len(), reminders.len(), token))
}

#[cfg(test)]
mod cloudkit_tests {
    use super::*;
    use serde_json::json;

    fn document(text: &str) -> String {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        use std::io::Write;
        // Apple's shape: text at field path 2 -> 3 -> 2.
        fn field(tag: u8, payload: &[u8]) -> Vec<u8> {
            let mut out = vec![(tag << 3) | 2];
            let mut len = payload.len();
            loop {
                let mut byte = (len & 0x7F) as u8;
                len >>= 7;
                if len != 0 {
                    byte |= 0x80;
                }
                out.push(byte);
                if len == 0 {
                    break;
                }
            }
            out.extend_from_slice(payload);
            out
        }
        let proto = field(2, &field(3, &field(2, text.as_bytes())));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&proto).unwrap();
        B64.encode(gz.finish().unwrap())
    }

    fn reminder_record(name: &str, title: &str, completed: bool, deleted: bool) -> Value {
        json!({
            "recordName": name,
            "recordType": "Reminder",
            "recordChangeTag": "tag1",
            "fields": {
                "TitleDocument": { "type": "ENCRYPTED_BYTES", "value": document(title) },
                "Completed": { "type": "NUMBER_INT64", "value": i64::from(completed) },
                "Deleted": { "type": "NUMBER_INT64", "value": i64::from(deleted) },
                "LastModifiedDate": { "type": "TIMESTAMP", "value": 1_753_808_674_659_i64 },
                "List": { "type": "REFERENCE", "value": { "recordName": "List/abc" } }
            }
        })
    }

    #[test]
    fn a_record_becomes_a_reminder_with_its_real_title() {
        let r = reminder_from_record(&reminder_record(
            "Reminder/1",
            "🏃 Move more",
            false,
            false,
        ))
        .unwrap();
        assert_eq!(r.title, "🏃 Move more");
        assert_eq!(r.list_guid, "List/abc");
        assert!(!r.is_done());
        assert!(r.raw.is_some(), "the record is kept for writes");
    }

    #[test]
    fn completion_carries_an_instant() {
        let r = reminder_from_record(&reminder_record("Reminder/1", "Done thing", true, false))
            .unwrap();
        assert!(r.is_done());
        assert_eq!(r.completed_at, Some(1_753_808_674));
    }

    /// Tombstones are the bulk of the zone and must not appear as reminders.
    #[test]
    fn deleted_records_are_dropped() {
        assert!(
            reminder_from_record(&reminder_record("Reminder/1", "Gone", false, true)).is_none()
        );
    }

    #[test]
    fn a_record_with_no_list_is_skipped() {
        let mut record = reminder_record("Reminder/1", "Orphan", false, false);
        record["fields"].as_object_mut().unwrap().remove("List");
        assert!(reminder_from_record(&record).is_none());
    }

    #[test]
    fn an_unreadable_title_still_yields_a_reminder() {
        let mut record = reminder_record("Reminder/1", "x", false, false);
        record["fields"]["TitleDocument"]["value"] = json!("!!!not base64!!!");
        let r = reminder_from_record(&record).unwrap();
        assert_eq!(r.title, "Untitled", "showing it beats hiding it");
    }
}

/// Tick a CloudKit reminder off, or put it back.
///
/// Sends only the fields that change, guarded by the record's change tag, so a reminder
/// edited on the phone since we read it is refused rather than overwritten.
pub async fn set_completed_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    reminder: &Reminder,
    done: bool,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    client
        .update_record(
            ZONE,
            &reminder.guid,
            "Reminder",
            reminder.etag.as_deref(),
            serde_json::json!({
                "Completed": { "value": i64::from(done), "type": "NUMBER_INT64" }
            }),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod list_kind_tests {
    use super::*;
    use serde_json::json;

    fn record(name: &str, kind: &str, title: Option<&str>) -> Value {
        let mut fields = serde_json::Map::new();
        if let Some(t) = title {
            fields.insert("Name".into(), json!({ "type": "STRING", "value": t }));
        }
        json!({ "recordName": name, "recordType": kind, "fields": fields })
    }

    /// Only records of type List are lists. A ListSection is a heading inside one, and a
    /// SmartList is a built-in filter carrying SmartListType rather than a Name — mapping
    /// either produced phantom entries called "Reminders" that held nothing.
    #[test]
    fn only_list_records_become_lists() {
        let zone = vec![
            record("List/a", "List", Some("GYM")),
            record("ListSection/b", "ListSection", None),
            record("SmartList/c", "SmartList", None),
        ];
        let kept: Vec<_> = zone
            .iter()
            .filter(|r| field::record_type(r).as_deref() == Some("List"))
            .filter_map(list_from_record)
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "GYM");
    }

    #[test]
    fn a_real_list_keeps_its_name() {
        let l = list_from_record(&record("List/abc", "List", Some("GYM"))).unwrap();
        assert_eq!(l.title, "GYM");
        assert_eq!(l.guid, "List/abc");
    }
}
