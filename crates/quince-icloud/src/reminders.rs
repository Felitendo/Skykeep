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
        ..Default::default()
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
        // The legacy endpoint does not carry a flag; CloudKit does, and that is the path
        // this account actually uses.
        flagged: v.get("flagged").and_then(Value::as_bool).unwrap_or(false),
        parent_guid: string(v, "parentId"),
        recur_frequency: None,
        recur_interval: None,
        etag: string(v, "etag"),
        // Keep Apple's payload verbatim: a write echoes the whole task back, and
        // reconstructing one from parsed columns would drop fields we do not model.
        raw: serde_json::to_string(v).ok(),
        timezone: None,
        section_guid: None,
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

    // A recurring reminder is a single record that Apple advances: ticking today's
    // occurrence sets Completed and moves DueDate to the next one. So a recurring record
    // whose due date is still ahead represents work that is NOT done — which is how Apple
    // Reminders shows it. Reading the flag literally hid every daily task.
    let recurring = record
        .pointer("/fields/RecurrenceRuleIDs/value")
        .and_then(Value::as_array)
        .is_some_and(|ids| !ids.is_empty());
    let due_at = field::timestamp_seconds(record, "DueDate");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let next_occurrence_pending = recurring && due_at.is_some_and(|due| due > now);

    let completed = field::flag(record, "Completed") && !next_occurrence_pending;
    Some(Reminder {
        guid,
        list_guid,
        // A reminder with no readable title is still a reminder; showing it beats hiding it.
        title: field::document_text(record, "TitleDocument").unwrap_or_else(|| "Untitled".into()),
        notes: field::document_text(record, "NotesDocument"),
        due_at,
        // Apple records when it was ticked off in its own field. Falling back to the last
        // modification was close but wrong: any later edit moved the completion time with it.
        completed_at: completed
            .then(|| {
                field::timestamp_seconds(record, "CompletionDate")
                    .or_else(|| field::timestamp_seconds(record, "LastModifiedDate"))
            })
            .flatten()
            .or(if completed { Some(0) } else { None }),
        priority: field::int(record, "Priority").unwrap_or(0),
        flagged: field::flag(record, "Flagged"),
        // Apple nests a reminder under another with this reference. Without reading it,
        // every subtask showed as a sibling of the thing it belongs to.
        parent_guid: field::reference(record, "ParentReminder"),
        // Filled in from the RecurrenceRule records, which arrive separately.
        recur_frequency: None,
        recur_interval: None,
        etag: record
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        raw: serde_json::to_string(record).ok(),
        // Apple keeps a named zone beside the due date, because nine in the morning means
        // nine in the morning wherever the person is. A repeating reminder stepped without it
        // moves an hour on each of the two days a year a clock changes.
        timezone: field::string(record, "TimeZone"),
        section_guid: None,
    })
}

/// Turn a CloudKit list record into a reminder list.
fn list_from_record(record: &Value) -> Option<ReminderList> {
    if field::flag(record, "Deleted") {
        return None;
    }
    // A group is a folder of lists, not a list. Apple keeps both as `List` records and tells
    // them apart with this flag; two of the nine on a real account are groups. Shown as lists
    // they read as two permanently empty ones, and worse, they can be picked as the place to
    // file a new reminder — which puts it somewhere the phone will not show it.
    if field::flag(record, "IsGroup") {
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
        colour: field::string(record, "Color").and_then(|c| hex_colour(&c)),
        ctag: None,
        emblem: field::string(record, "BadgeEmblem").filter(|e| !e.trim().is_empty()),
        group_guid: field::reference(record, "ParentList"),
        pinned: is_pinned(record),
        sorting: field::string(record, "SortingStyle"),
        // A JSON array of reminder ids in a string field, in the order they were arranged.
        reminder_order: field::string(record, "ReminderIDs")
            .and_then(|ids| serde_json::from_str::<Vec<String>>(&ids).ok())
            .unwrap_or_default(),
        // Apple's List record carries no sort order — the earlier `SortOrder` here read a
        // field that has never existed, so every list scored zero and the sidebar came out in
        // whatever order the database happened to return. That is why a new reminder always
        // seemed to land in the same arbitrary list.
        //
        // The real arrangement lives in the account record's `ListIDsMergeableOrdering`, a
        // mergeable-ordering structure that also holds which group a list sits under. Until
        // that is decoded, the order Apple walks the zone in is used: stable between refreshes
        // and the same on every device, which is the part that was actually broken.
        sort_order: 0,
        enabled: true,
    })
}

/// A group record as the folder it is, or `None` for anything else.
fn group_from_record(record: &Value) -> Option<quince_core::reminders::ReminderGroup> {
    if field::flag(record, "Deleted") || !field::flag(record, "IsGroup") {
        return None;
    }
    Some(quince_core::reminders::ReminderGroup {
        guid: field::record_name(record)?,
        title: field::string(record, "Name")
            .or_else(|| field::document_text(record, "NameDocument"))
            .unwrap_or_else(|| "Group".into()),
        sort_order: 0,
    })
}

/// A list's colour as `#RRGGBB`.
///
/// CloudKit stores it as JSON — `{"daHexString":"#FFCC00","red":1,…}` — where the older web
/// service sent the hex alone. Kept as it arrived, the JSON reached the interface as a colour
/// it could not draw, and every list came out white.
fn hex_colour(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let is_hex = |h: &str| {
        h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit())
    };
    if is_hex(raw) {
        return Some(raw.to_ascii_uppercase());
    }
    let v: Value = serde_json::from_str(raw).ok()?;
    if let Some(hex) = v.get("daHexString").and_then(Value::as_str) {
        if is_hex(hex) {
            return Some(hex.to_ascii_uppercase());
        }
    }
    // Only components: work the hex out from them.
    let channel = |name: &str| -> Option<u8> {
        let c = v.get(name)?.as_f64()?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some((c.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        channel("red")?,
        channel("green")?,
        channel("blue")?
    ))
}

/// Whether a list is pinned beside Today and Scheduled.
///
/// Unpinning does not clear `PinnedDate`; it sets it to the first day of year one, which as
/// milliseconds since 1970 is a large negative number. Only a real date means pinned.
fn is_pinned(record: &Value) -> bool {
    record
        .pointer("/fields/PinnedDate/value")
        .and_then(Value::as_i64)
        .is_some_and(|ms| ms > 0)
}

/// The order lists appear in on the user's other devices.
///
/// Apple keeps this in a ManualSortHint record whose payload is JSON, despite the field
/// being named "AsBytes": `topLevelElementIDs` is the list order, and
/// `secondaryLevelElementIDsByTopLevelElementID` would carry the order of reminders inside
/// each list where one has been set by hand.
fn list_order_from(record: &Value) -> Vec<String> {
    let Some(encoded) = field::string(
        record,
        "HintDataIncludingRemindersInPrimaryAccountOnlyAsBytes",
    )
    .or_else(|| field::string(record, "HintDataExcludingRemindersInLocalAccountAsBytes")) else {
        return Vec::new();
    };

    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let Ok(bytes) = B64.decode(encoded.trim()) else {
        return Vec::new();
    };
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| {
            v.get("topLevelElementIDs")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().map(ToString::to_string))
                        .collect()
                })
        })
        .unwrap_or_default()
}

/// Apply that order to the lists.
///
/// The hint carries bare UUIDs while a list's record name is `List/<uuid>`, so they are
/// matched on the suffix. Anything the hint does not mention keeps its place after the rest
/// rather than jumping to the front.
/// Take Apple's own arrangement, but only while it still describes the lists that exist.
///
/// The record holding it is written by whichever device last rearranged the sidebar, and it is
/// not cleaned up when a list goes. On a real account it names eight lists of which seven have
/// since been deleted — obeying it put every surviving list on the same rank and left the order
/// to chance, which is exactly what made a new reminder appear to land somewhere arbitrary.
///
/// So the question asked of it is whether it still describes the account: most of what it names
/// should still be there. A list it has not heard of is just a new one and sorts after; a list
/// of names that are mostly gone is a leftover and is ignored.
fn apply_list_order(
    lists: &mut [ReminderList],
    groups: &mut [quince_core::reminders::ReminderGroup],
    order: &[String],
) {
    if order.is_empty() {
        return;
    }
    let uuid_of = |guid: &str| guid.rsplit('/').next().unwrap_or(guid).to_string();
    let position = |guid: &str| {
        let uuid = uuid_of(guid);
        order.iter().position(|id| id.eq_ignore_ascii_case(&uuid))
    };
    let known: Vec<String> = lists
        .iter()
        .map(|l| uuid_of(&l.guid))
        .chain(groups.iter().map(|g| uuid_of(&g.guid)))
        .collect();
    let still_here = order
        .iter()
        .filter(|id| known.iter().any(|k| k.eq_ignore_ascii_case(id)))
        .count();
    if still_here * 2 < order.len() {
        return;
    }
    // A list or group the arrangement has not heard of sits after the ones it has, keeping
    // the order it arrived in rather than tying with every other stranger.
    let place = |guid: &str, arrived: i64| {
        position(guid)
            .and_then(|i| i64::try_from(i).ok())
            .unwrap_or_else(|| i64::from(u32::MAX) + arrived)
    };
    for list in lists.iter_mut() {
        list.sort_order = place(&list.guid, list.sort_order);
    }
    for group in groups.iter_mut() {
        group.sort_order = place(&group.guid, group.sort_order);
    }
}

/// The order lists and groups appear in on the phone.
///
/// It is on the account record, as `ListIDsMergeableOrdering`: one ordered set holding every
/// list and group, a group's lists ordered among the rest by the same sequence. The
/// `ManualSortHint` read before this is a leftover — on a real account seven of its eight
/// ids named lists that no longer exist.
fn account_list_order(record: &Value) -> Vec<String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    record
        .pointer("/fields/ListIDsMergeableOrdering/value")
        .and_then(Value::as_str)
        .and_then(|encoded| B64.decode(encoded.trim()).ok())
        .and_then(|bytes| crate::mergeable::ordered_strings(&bytes))
        .unwrap_or_default()
}

/// Read reminders from CloudKit into the local library.
///
/// Pass the previous `since` token to fetch only what changed. That distinction matters for
/// more than speed: an incremental response contains only changes, so absence means
/// "unchanged", not "deleted" — treating it otherwise would empty the library on the first
/// incremental refresh.
///
/// Returns the number of lists and reminders written, plus the token to resume from.
pub async fn sync_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
    since: Option<&str>,
) -> Result<(usize, usize, Option<String>)> {
    let fetched = fetch_cloudkit(http, ckdatabasews_root, params, since).await?;
    apply_cloudkit(library, fetched)
}

/// One pass over the zone, with nothing stored yet.
///
/// Fetching and storing are separate so a caller can do the slow half — thousands of
/// records over the network — without holding the library open. Everything else in the app
/// reads that library, and a refresh that locked it would freeze the interface it is meant
/// to be keeping current.
pub struct RemindersFetch {
    records: Vec<Value>,
    token: Option<String>,
    incremental: bool,
    /// Which reminder sits in which section, per list, from each changed list's membership
    /// file. Only lists that came back in this walk are here; the rest are unchanged.
    memberships: std::collections::HashMap<String, Vec<(String, String)>>,
}

/// The placements in a list's membership file, as record names.
///
/// The file is JSON, a `memberships` array of `{ memberID, groupID }` where the member is a
/// reminder and the group a section, both as bare ids. A reminder with no groupID sits
/// outside every section and is not a placement. Anything unreadable places nothing, which
/// leaves the reminders in the list without headings rather than losing them.
#[must_use]
pub fn memberships_from(bytes: &[u8]) -> Vec<(String, String)> {
    let Ok(file) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    file.get("memberships")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|m| {
                    let member = m.get("memberID")?.as_str()?;
                    let group = m.get("groupID")?.as_str()?;
                    Some((format!("Reminder/{member}"), format!("ListSection/{group}")))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch the membership file for every list that came back in a walk.
///
/// A file that will not download is skipped rather than failing the sync: the reminders are
/// still right, and their sections simply stay as they were until the next pass.
async fn fetch_memberships(
    http: &reqwest::Client,
    records: &[Value],
) -> std::collections::HashMap<String, Vec<(String, String)>> {
    let mut out = std::collections::HashMap::new();
    for record in records {
        if field::record_type(record).as_deref() != Some("List") || field::flag(record, "Deleted") {
            continue;
        }
        let Some(name) = field::record_name(record) else {
            continue;
        };
        let Some(url) = record
            .pointer("/fields/MembershipsOfRemindersInSectionsAsData/value/downloadURL")
            .and_then(Value::as_str)
        else {
            continue;
        };
        // The URL is a template; Apple fills the file name in with whatever it is given.
        let Ok(response) = http.get(url.replace("${f}", "memberships")).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else {
            continue;
        };
        out.insert(name, memberships_from(&bytes));
    }
    out
}

/// Ask Apple what changed. Touches no local state.
pub async fn fetch_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    since: Option<&str>,
) -> Result<RemindersFetch> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let walk = client.all_records(ZONE, since).await?;
    let memberships = fetch_memberships(http, &walk.records).await;
    Ok(RemindersFetch {
        memberships,
        records: walk.records,
        token: walk.token,
        // A walk that stopped at its page cap saw only part of the zone, so it cannot be
        // treated as the whole truth however it was asked for.
        incremental: since.is_some() || !walk.complete,
    })
}

/// Store a fetched pass. Touches no network.
pub fn apply_cloudkit(
    library: &mut quince_core::Library,
    fetched: RemindersFetch,
) -> Result<(usize, usize, Option<String>)> {
    let RemindersFetch {
        records,
        token,
        incremental,
        memberships,
    } = fetched;
    let mut sections: Vec<quince_core::reminders::ReminderSection> = Vec::new();
    let mut gone_sections: Vec<String> = Vec::new();

    let mut lists: Vec<ReminderList> = Vec::new();
    let mut reminders: Vec<Reminder> = Vec::new();
    let mut gone_reminders: Vec<String> = Vec::new();
    let mut gone_lists: Vec<String> = Vec::new();
    let mut list_order: Vec<String> = Vec::new();
    let mut groups: Vec<quince_core::reminders::ReminderGroup> = Vec::new();
    let mut account_order: Vec<String> = Vec::new();
    // Alerts: an Alarm on a reminder, and the AlarmTrigger holding its moment.
    let mut alarms: Vec<quince_core::alarms::Alarm> = Vec::new();
    let mut triggers: Vec<quince_core::alarms::AlarmTrigger> = Vec::new();
    let mut gone_alarms: Vec<String> = Vec::new();
    // A RecurrenceRule is its own record pointing back at the reminder it governs.
    let mut rules: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();

    for record in &records {
        let Some(name) = field::record_name(record) else {
            continue;
        };
        // CloudKit reports a removal either as a top-level flag or a Deleted field.
        let removed = record
            .get("deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || field::flag(record, "Deleted");

        let Some(kind) = field::record_type(record) else {
            // A tombstone carries a name and nothing else — no type, no fields. Skipping
            // them meant anything deleted on a phone stayed on screen here forever, and
            // saving one of those ghosts wrote it back to Apple. The name is enough: it
            // says which record is gone, and a name only ever belongs to one thing.
            if removed {
                gone_reminders.push(name.clone());
                gone_sections.push(name.clone());
                gone_alarms.push(name.clone());
                gone_lists.push(name);
            }
            continue;
        };

        match kind.as_str() {
            "Reminder" => {
                if removed {
                    gone_reminders.push(name);
                } else {
                    reminders.extend(reminder_from_record(record));
                }
            }
            // Only List records are the lists people made. ListSection is a heading inside
            // one, and SmartList is a built-in filter — Today, Scheduled, Flagged — which
            // carries a SmartListType instead of a Name and appeared as phantom entries
            // called "Reminders" that held nothing.
            "List" => {
                if removed {
                    gone_lists.push(name);
                } else if let Some(mut group) = group_from_record(record) {
                    // One count across groups and lists, so a group sits among the lists
                    // where Apple walks it rather than in a separate run of its own.
                    group.sort_order = i64::try_from(lists.len() + groups.len()).unwrap_or(0);
                    groups.push(group);
                } else if let Some(mut list) = list_from_record(record) {
                    // Apple walks the zone in a stable order that is the same on every device,
                    // and a List record carries no date or rank of its own. Keeping the
                    // position it arrived in is what stops the sidebar coming out differently
                    // on every refresh — which is what made a new reminder seem to land in an
                    // arbitrary list.
                    list.sort_order = i64::try_from(lists.len() + groups.len()).unwrap_or(0);
                    lists.push(list);
                }
            }
            // A heading inside a list. Which reminders sit under it is not on this record —
            // it is in the list's membership file, fetched alongside the walk.
            "ListSection" => {
                if removed {
                    gone_sections.push(name);
                } else if let Some(list_guid) = field::reference(record, "List") {
                    sections.push(quince_core::reminders::ReminderSection {
                        guid: name,
                        list_guid,
                        title: field::string(record, "DisplayName")
                            .unwrap_or_else(|| "Section".into()),
                        created_at: field::timestamp_seconds(record, "CreationDate"),
                    });
                }
            }
            // The phone removes an alarm outright rather than marking it; either way it
            // no longer rings.
            "Alarm" => match (removed, alarm_from_record(record)) {
                (false, Some(alarm)) => alarms.push(alarm),
                _ => gone_alarms.push(name),
            },
            "AlarmTrigger" => match (removed, trigger_from_record(record)) {
                (false, Some(trigger)) => triggers.push(trigger),
                _ => gone_alarms.push(name),
            },
            "RecurrenceRule" => {
                if !removed {
                    if let Some(target) = field::reference(record, "Reminder") {
                        let frequency = field::int(record, "Frequency").unwrap_or(0);
                        let interval = field::int(record, "Interval").unwrap_or(1);
                        rules.insert(target, (frequency, interval));
                    }
                }
            }
            "Account" => {
                let order = account_list_order(record);
                if !order.is_empty() {
                    account_order = order;
                }
            }
            // An older record of the order, used only where the account holds none.
            "ManualSortHint" => {
                let order = list_order_from(record);
                if !order.is_empty() {
                    list_order = order;
                }
            }
            _ => {}
        }
    }

    // Every list a reminder points at must exist, or storing it fails the foreign key.
    // On an incremental pass the list is usually already stored, so check before inventing.
    let known_locally: std::collections::HashSet<String> = library
        .reminder_lists()?
        .into_iter()
        .map(|l| l.guid)
        .collect();
    for reminder in &reminders {
        let missing = !lists.iter().any(|l| l.guid == reminder.list_guid)
            && !known_locally.contains(&reminder.list_guid);
        if missing {
            lists.push(ReminderList {
                guid: reminder.list_guid.clone(),
                title: "Reminders".into(),
                colour: None,
                ctag: None,
                sort_order: 0,
                enabled: true,
                ..Default::default()
            });
        }
    }

    // Attach each rule to the reminder it governs, so completing one can advance it.
    for reminder in &mut reminders {
        if let Some((frequency, interval)) = rules.get(&reminder.guid) {
            reminder.recur_frequency = Some(*frequency);
            reminder.recur_interval = Some(*interval);
        }
    }

    // Match the order the phone shows before storing.
    let order = if account_order.is_empty() {
        &list_order
    } else {
        &account_order
    };
    apply_list_order(&mut lists, &mut groups, order);
    // An incremental pass that carried no order says nothing about where anything sits, and
    // its walk positions count only what changed. A list already stored keeps its place.
    if incremental && order.is_empty() {
        let stored: std::collections::HashMap<String, i64> = library
            .reminder_lists()?
            .into_iter()
            .map(|l| (l.guid, l.sort_order))
            .collect();
        for list in &mut lists {
            if let Some(&at) = stored.get(&list.guid) {
                list.sort_order = at;
            }
        }
    }

    if incremental {
        library.merge_reminder_lists(&lists)?;
    } else {
        library.put_reminder_lists(&lists)?;
    }
    // Groups: a full pass has every one; an incremental pass only the ones that changed, laid
    // over what is stored, less any that were deleted.
    let groups = if incremental {
        let mut merged = library.reminder_groups()?;
        for group in groups {
            match merged.iter_mut().find(|g| g.guid == group.guid) {
                Some(existing) => existing.title = group.title,
                None => {
                    let mut group = group;
                    group.sort_order = i64::try_from(merged.len()).unwrap_or(0);
                    merged.push(group);
                }
            }
        }
        merged.retain(|g| !gone_lists.contains(&g.guid));
        merged
    } else {
        groups
    };
    library.put_reminder_groups(&groups)?;
    library.put_reminders(&reminders)?;
    library.put_alarms(&alarms, &triggers, &gone_alarms, !incremental)?;
    if !gone_reminders.is_empty() || !gone_lists.is_empty() {
        library.forget_reminders(&gone_reminders, &gone_lists)?;
    }
    // Sections before memberships: a placement is only honoured for a section this library
    // holds, so the section has to be there first.
    library.put_reminder_sections(&sections, &gone_sections)?;
    for (list, placed) in &memberships {
        library.set_reminder_sections(list, placed)?;
    }
    // A full pass saw everything Apple has, so anything else here is gone. An incremental
    // pass saw only changes, where absence means unchanged — pruning there would empty the
    // library on the first refresh.
    if !incremental {
        let keep: Vec<String> = reminders.iter().map(|r| r.guid.clone()).collect();
        library.retain_reminders(&keep)?;
    }
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

/// Milliseconds since the epoch, which is how CloudKit writes timestamps.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Add a reminder to a list.
///
/// Returns the record name Apple will know it by, so the caller can store it without waiting
/// for the next sync.
pub async fn create_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    list_guid: &str,
    title: &str,
) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Protocol("a reminder needs a title".into()));
    }
    // An account with no lists is a real account — a new one, or one whose lists were all
    // deleted. Sending a reference to nothing would have Apple file the reminder somewhere
    // no device can show it, so it is refused here in words a person can act on.
    if list_guid.trim().is_empty() {
        return Err(Error::Protocol(
            "a reminder has to go in a list, and this account has none yet".into(),
        ));
    }

    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let record_name = format!(
        "Reminder/{}",
        uuid::Uuid::new_v4().to_string().to_uppercase()
    );
    let stamp = now_millis();

    client
        .create_record_signed(
            ZONE,
            &record_name,
            "Reminder",
            serde_json::json!({
                "TitleDocument": {
                    "type": "ENCRYPTED_BYTES",
                    "value": crate::ckdocument::text_to_base64(title),
                },
                "List": client.reference_to(ZONE, list_guid),
                "Completed": { "type": "NUMBER_INT64", "value": 0 },
                "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                "CreationDate": { "type": "TIMESTAMP", "value": stamp },
                "LastModifiedDate": { "type": "TIMESTAMP", "value": stamp },
            }),
        )
        .await?;

    // The phone adds a new reminder to the end of its list's own order. Without that, a list
    // sorted by hand has nowhere to put it — measured: a list of 126 reminders held 125 ids,
    // the missing one the reminder made here. The reminder itself is made either way, so a
    // failure here costs its place in the order, never the reminder.
    let _ = place_in_list(&client, list_guid, &record_name, None).await;

    Ok(record_name)
}

/// Add a reminder to the end of a list's `ReminderIDs`, and take it out of the list it left.
async fn place_in_list(
    client: &ZoneClient<'_>,
    list_guid: &str,
    reminder_guid: &str,
    left: Option<&str>,
) -> Result<()> {
    let id = reminder_guid
        .rsplit('/')
        .next()
        .unwrap_or(reminder_guid)
        .to_string();
    let edit = |list: Value, add: bool| {
        let mut ids: Vec<String> = list
            .pointer("/fields/ReminderIDs/value")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let had = ids.iter().any(|x| x.eq_ignore_ascii_case(&id));
        if add && !had {
            ids.push(id.clone());
        } else if !add && had {
            ids.retain(|x| !x.eq_ignore_ascii_case(&id));
        } else {
            return None;
        }
        Some(serde_json::json!({
            "ReminderIDs": { "type": "STRING", "value": serde_json::to_string(&ids).unwrap_or_default() }
        }))
    };
    for (guid, add) in [Some((list_guid, true)), left.map(|l| (l, false))]
        .into_iter()
        .flatten()
    {
        let Some(list) = client.lookup_record(ZONE, guid).await? else {
            continue;
        };
        if let Some(fields) = edit(list, add) {
            client
                .update_record_signed(ZONE, guid, "List", None, fields)
                .await?;
        }
    }
    Ok(())
}

/// How often a reminder comes back.
///
/// Apple's frequency is 0 daily, 1 weekly, 2 monthly, 3 yearly — the same numbering the
/// stored rule uses, so nothing is translated between here and the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Repeat {
    pub frequency: i64,
    pub interval: i64,
}

/// Set or clear a reminder's repeat.
///
/// A repeat is not a field on the reminder: it is its own record pointing back at the
/// reminder, with the reminder holding the rule's bare id in a list. So setting one is
/// three steps, and the order is what keeps it consistent — write the new rule, point the
/// reminder at it, and only then retire the old ones. Done the other way round there is a
/// moment where the reminder refers to a rule that no longer exists.
pub async fn set_recurrence_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    reminder: &Reminder,
    repeat: Option<Repeat>,
) -> Result<()> {
    // Apple will not schedule a repeat with nothing to repeat from, and a reminder that
    // repeats from no date would simply never come back.
    if repeat.is_some() && reminder.due_at.is_none() {
        return Err(Error::Protocol(
            "give this a date before asking it to repeat".into(),
        ));
    }

    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);

    // Read what Apple holds rather than what the last sync saw: the rule may have been
    // changed on a phone since, and the ids to retire are the current ones.
    let current = client
        .lookup_record(ZONE, &reminder.guid)
        .await?
        .ok_or_else(|| Error::Protocol("Apple no longer has that reminder.".into()))?;
    let existing: Vec<String> = current
        .pointer("/fields/RecurrenceRuleIDs/value")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();

    let wanted: Vec<String> = match repeat {
        None => Vec::new(),
        Some(repeat) => {
            if !(0..=3).contains(&repeat.frequency) {
                return Err(Error::Protocol("that is not a repeat Apple knows".into()));
            }
            // The list holds the bare id; the record is named with a prefix. Apple's own
            // records are written that way, and a mismatch leaves a rule nothing points at.
            let id = uuid::Uuid::new_v4().to_string().to_uppercase();
            client
                .create_record(
                    ZONE,
                    &format!("RecurrenceRule/{id}"),
                    "RecurrenceRule",
                    serde_json::json!({
                        "Frequency": { "type": "NUMBER_INT64", "value": repeat.frequency },
                        "Interval": { "type": "NUMBER_INT64", "value": repeat.interval.clamp(1, 1_000) },
                        "FirstDayOfTheWeek": { "type": "NUMBER_INT64", "value": 0 },
                        "OccurrenceCount": { "type": "NUMBER_INT64", "value": 0 },
                        "Imported": { "type": "NUMBER_INT64", "value": 0 },
                        "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                        "Reminder": client.reference_to(ZONE, &reminder.guid),
                    }),
                )
                .await?;
            vec![id]
        }
    };

    let tag = current
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .or(reminder.etag.as_deref());

    client
        .update_record_signed(
            ZONE,
            &reminder.guid,
            "Reminder",
            tag,
            serde_json::json!({
                "RecurrenceRuleIDs": { "type": "STRING_LIST", "value": wanted },
                "LastModifiedDate": { "type": "TIMESTAMP", "value": now_millis() },
            }),
        )
        .await?;

    // Only now that nothing points at them. A rule left behind would keep rescheduling the
    // reminder from a schedule the person just replaced.
    for old in existing {
        client
            .delete_record(ZONE, &format!("RecurrenceRule/{old}"))
            .await?;
    }
    Ok(())
}

/// Remove a reminder.
///
/// Apple marks removal with a Deleted flag rather than destroying the record, which is what
/// its own clients do and what keeps other devices able to reconcile.
pub async fn delete_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    reminder: &Reminder,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    client
        .update_record_signed(
            ZONE,
            &reminder.guid,
            "Reminder",
            reminder.etag.as_deref(),
            serde_json::json!({
                "Deleted": { "type": "NUMBER_INT64", "value": 1 },
                "LastModifiedDate": { "type": "TIMESTAMP", "value": now_millis() },
            }),
        )
        .await?;
    Ok(())
}

/// What to change about a reminder. Anything left `None` is untouched.
///
/// Written as one request so a title and a due date set together cannot half-apply.
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderEdit {
    pub title: Option<String>,
    /// `Some(None)` clears the notes; `None` leaves them alone.
    pub notes: Option<Option<String>>,
    /// Unix seconds. `Some(None)` clears the due date.
    pub due_at: Option<Option<i64>>,
    pub flagged: Option<bool>,
    pub completed: Option<bool>,
    /// Apple's scale, shared with EventKit: 0 none, 1 high, 5 medium, 9 low.
    pub priority: Option<i64>,
    /// Move to another list.
    pub list_guid: Option<String>,
    /// Nest under another reminder, or `Some(None)` to bring it back out to the top level.
    pub parent_guid: Option<Option<String>>,
}

impl ReminderEdit {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.notes.is_none()
            && self.due_at.is_none()
            && self.flagged.is_none()
            && self.completed.is_none()
            && self.priority.is_none()
            && self.list_guid.is_none()
            && self.parent_guid.is_none()
    }
}

/// Apply an edit to a reminder.
///
/// Sends only the fields that changed, guarded by the record's change tag, so a reminder
/// edited on the phone since we read it is refused rather than overwritten.
pub async fn edit_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    reminder: &Reminder,
    edit: &ReminderEdit,
) -> Result<()> {
    if edit.is_empty() {
        return Ok(());
    }

    // Refused before anything reaches the network, including the lookup below — asking
    // Apple about a record only to reject the edit locally is a request for nothing.
    if let Some(title) = &edit.title {
        if title.trim().is_empty() {
            return Err(Error::Protocol("a reminder needs a title".into()));
        }
    }

    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params.clone());
    let mut fields = serde_json::Map::new();

    // A title and a note are documents, not strings: around the text Apple keeps the state
    // its devices use to reconcile edits. Building a fresh minimal document for a rename
    // threw all of that away every time — the same mistake that stopped edited notes from
    // ever reaching a phone. Rewriting in place keeps it, and only falls back to a new
    // document when there is genuinely nothing to rewrite.
    let current = if edit.title.is_some() || edit.notes.is_some() {
        client.lookup_record(ZONE, &reminder.guid).await?
    } else {
        None
    };
    let document = |field: &str, text: &str| -> String {
        current
            .as_ref()
            .and_then(|rec| rec.pointer(&format!("/fields/{field}/value")))
            .and_then(Value::as_str)
            .and_then(|existing| crate::ckdocument::replace_text_in_base64(existing, text))
            .unwrap_or_else(|| crate::ckdocument::text_to_base64(text))
    };

    if let Some(title) = &edit.title {
        fields.insert(
            "TitleDocument".into(),
            serde_json::json!({
                "type": "ENCRYPTED_BYTES",
                "value": document("TitleDocument", title.trim()),
            }),
        );
    }

    if let Some(notes) = &edit.notes {
        // Clearing writes an empty document rather than removing the field: Apple's clients
        // expect it to exist.
        let text = notes.as_deref().unwrap_or("");
        fields.insert(
            "NotesDocument".into(),
            serde_json::json!({
                "type": "ENCRYPTED_BYTES",
                "value": document("NotesDocument", text),
            }),
        );
    }

    if let Some(due) = &edit.due_at {
        fields.insert(
            "DueDate".into(),
            match due {
                Some(seconds) => serde_json::json!({
                    "type": "TIMESTAMP", "value": seconds.saturating_mul(1000)
                }),
                None => serde_json::json!({ "type": "TIMESTAMP", "value": null }),
            },
        );
    }

    if let Some(priority) = edit.priority {
        // Anything off Apple's scale would render as an unknown marker on a phone rather
        // than being ignored, so it is clamped rather than passed through.
        let level = match priority {
            1..=3 => 1,
            4..=6 => 5,
            7..=9 => 9,
            _ => 0,
        };
        fields.insert(
            "Priority".into(),
            serde_json::json!({ "type": "NUMBER_INT64", "value": level }),
        );
    }

    if let Some(list_guid) = &edit.list_guid {
        fields.insert("List".into(), client.reference_to(ZONE, list_guid));
    }

    if let Some(parent) = &edit.parent_guid {
        fields.insert(
            "ParentReminder".into(),
            match parent {
                Some(guid) => client.reference_to(ZONE, guid),
                // CloudKit clears a reference with an explicit null; omitting the field
                // would leave the reminder nested where it was.
                None => Value::Null,
            },
        );
    }

    if let Some(flagged) = edit.flagged {
        fields.insert(
            "Flagged".into(),
            serde_json::json!({ "type": "NUMBER_INT64", "value": i64::from(flagged) }),
        );
    }

    if let Some(completed) = edit.completed {
        fields.insert(
            "Completed".into(),
            serde_json::json!({ "type": "NUMBER_INT64", "value": i64::from(completed) }),
        );
    }

    fields.insert(
        "LastModifiedDate".into(),
        serde_json::json!({ "type": "TIMESTAMP", "value": now_millis() }),
    );

    // The tag Apple has now, when a rename just read the record — fresher than whatever the
    // caller was holding.
    let tag = current
        .as_ref()
        .and_then(|rec| rec.get("recordChangeTag"))
        .and_then(Value::as_str)
        .or(reminder.etag.as_deref());

    client
        .update_record_signed(ZONE, &reminder.guid, "Reminder", tag, Value::Object(fields))
        .await?;
    // Moved to another list: it joins the end of that list's order and leaves the old one's,
    // as on the phone. The move itself has happened either way.
    if let Some(list_guid) = edit
        .list_guid
        .as_deref()
        .filter(|l| *l != reminder.list_guid)
    {
        let _ = place_in_list(
            &client,
            list_guid,
            &reminder.guid,
            Some(&reminder.list_guid),
        )
        .await;
    }
    Ok(())
}

/// Create a list.
///
/// Returns the record name Apple will know it by. `Name` is a plain string here, unlike a
/// reminder's title, which is wrapped in the document format.
pub async fn create_list_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    title: &str,
) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Protocol("a list needs a name".into()));
    }

    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let record_name = format!("List/{}", uuid::Uuid::new_v4().to_string().to_uppercase());

    client
        .create_record_signed(
            ZONE,
            &record_name,
            "List",
            serde_json::json!({
                "Name": { "type": "STRING", "value": title },
                "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                "IsGroup": { "type": "NUMBER_INT64", "value": 0 },
                "SortingStyle": { "type": "STRING", "value": "manual" },
            }),
        )
        .await?;

    Ok(record_name)
}

/// Rename a list.
pub async fn rename_list_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    list_guid: &str,
    title: &str,
) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Protocol("a list needs a name".into()));
    }
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    client
        .update_record_signed(
            ZONE,
            list_guid,
            "List",
            None,
            serde_json::json!({ "Name": { "type": "STRING", "value": title } }),
        )
        .await?;
    Ok(())
}

/// Remove a list.
///
/// Marks it deleted rather than destroying it, matching how Apple's own clients work and
/// leaving other devices able to reconcile. Its reminders go with it.
pub async fn delete_list_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    list_guid: &str,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    client
        .update_record_signed(
            ZONE,
            list_guid,
            "List",
            None,
            serde_json::json!({
                "Deleted": { "type": "NUMBER_INT64", "value": 1 },
            }),
        )
        .await?;
    Ok(())
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

    // A repeating reminder is already held completed while its record points at the next
    // occurrence, so writing Completed again changes nothing and the tick appears to do
    // nothing. Ticking one means moving to the occurrence after this one.
    let fields = if done && reminder.repeats() {
        let Some(next) = reminder.next_occurrence() else {
            return Err(Error::Protocol(
                "this repeats, but its schedule could not be read".into(),
            ));
        };
        serde_json::json!({
            "Completed": { "value": 1, "type": "NUMBER_INT64" },
            "DueDate": { "value": next.saturating_mul(1000), "type": "TIMESTAMP" },
            "LastModifiedDate": { "value": now_millis(), "type": "TIMESTAMP" },
        })
    } else {
        serde_json::json!({
            "Completed": { "value": i64::from(done), "type": "NUMBER_INT64" },
            "LastModifiedDate": { "value": now_millis(), "type": "TIMESTAMP" },
        })
    };

    client
        .update_record_signed(
            ZONE,
            &reminder.guid,
            "Reminder",
            reminder.etag.as_deref(),
            fields,
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
        let zone = [
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

#[cfg(test)]
mod incremental_tests {
    use super::*;
    use quince_core::Library;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn doc(text: &str) -> String {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        use std::io::Write;
        fn f(tag: u8, payload: &[u8]) -> Vec<u8> {
            let mut out = vec![(tag << 3) | 2];
            let mut len = payload.len();
            loop {
                let mut b = (len & 0x7F) as u8;
                len >>= 7;
                if len != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if len == 0 {
                    break;
                }
            }
            out.extend_from_slice(payload);
            out
        }
        let proto = f(2, &f(3, &f(2, text.as_bytes())));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&proto).unwrap();
        B64.encode(gz.finish().unwrap())
    }

    fn reminder(name: &str, title: &str) -> Value {
        json!({
            "recordName": name, "recordType": "Reminder",
            "fields": {
                "TitleDocument": { "type": "ENCRYPTED_BYTES", "value": doc(title) },
                "List": { "type": "REFERENCE", "value": { "recordName": "List/a" } }
            }
        })
    }

    fn list(name: &str, title: &str) -> Value {
        json!({
            "recordName": name, "recordType": "List",
            "fields": { "Name": { "type": "STRING", "value": title } }
        })
    }

    async fn serve(body: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    /// The bug worth guarding: an incremental response mentions one changed record, and
    /// everything else must survive. Replacing on a delta would empty the library.
    #[tokio::test]
    async fn an_incremental_sync_keeps_what_it_did_not_mention() {
        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();

        let full = serve(json!({ "zones": [{
            "records": [list("List/a", "GYM"), reminder("Reminder/1", "Squats"),
                        reminder("Reminder/2", "Deadlifts")],
            "syncToken": "t1", "moreComing": false }] }))
        .await;
        sync_cloudkit(&http, &full.uri(), vec![], &mut library, None)
            .await
            .unwrap();
        assert_eq!(library.reminders_in("List/a", true).unwrap().len(), 2);

        // A delta carrying only one changed reminder, and no list at all.
        let delta = serve(json!({ "zones": [{
            "records": [reminder("Reminder/1", "Squats (heavier)")],
            "syncToken": "t2", "moreComing": false }] }))
        .await;
        sync_cloudkit(&http, &delta.uri(), vec![], &mut library, Some("t1"))
            .await
            .unwrap();

        let after = library.reminders_in("List/a", true).unwrap();
        assert_eq!(
            after.len(),
            2,
            "the unmentioned reminder must survive a delta"
        );
        assert_eq!(
            library.reminder_lists().unwrap().len(),
            1,
            "and so must the list, which the delta never mentioned"
        );
        assert!(
            after.iter().any(|r| r.title == "Squats (heavier)"),
            "the changed one should be updated"
        );
    }

    /// A full sync still drops what Apple stopped reporting.
    #[tokio::test]
    async fn a_full_sync_still_removes_vanished_lists() {
        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();

        let first = serve(json!({ "zones": [{
            "records": [list("List/a", "GYM"), list("List/b", "Study")],
            "syncToken": "t1", "moreComing": false }] }))
        .await;
        sync_cloudkit(&http, &first.uri(), vec![], &mut library, None)
            .await
            .unwrap();
        assert_eq!(library.reminder_lists().unwrap().len(), 2);

        let second = serve(json!({ "zones": [{
            "records": [list("List/a", "GYM")], "syncToken": "t2", "moreComing": false }] }))
        .await;
        sync_cloudkit(&http, &second.uri(), vec![], &mut library, None)
            .await
            .unwrap();
        assert_eq!(
            library.reminder_lists().unwrap().len(),
            1,
            "a full pass is authoritative"
        );
    }

    /// A reminder deleted on the phone must disappear here.
    #[tokio::test]
    async fn a_deleted_reminder_is_removed() {
        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();

        let first = serve(json!({ "zones": [{
            "records": [list("List/a", "GYM"), reminder("Reminder/1", "Squats")],
            "syncToken": "t1", "moreComing": false }] }))
        .await;
        sync_cloudkit(&http, &first.uri(), vec![], &mut library, None)
            .await
            .unwrap();
        assert_eq!(library.reminders_in("List/a", true).unwrap().len(), 1);

        let delta = serve(json!({ "zones": [{
            "records": [{ "recordName": "Reminder/1", "recordType": "Reminder", "deleted": true }],
            "syncToken": "t2", "moreComing": false }] }))
        .await;
        sync_cloudkit(&http, &delta.uri(), vec![], &mut library, Some("t1"))
            .await
            .unwrap();
        assert!(library.reminders_in("List/a", true).unwrap().is_empty());
    }
}

#[cfg(test)]
mod recurrence_tests {
    use super::*;
    use serde_json::json;

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
    }

    fn record(completed: bool, recurring: bool, due_offset_secs: i64) -> Value {
        let due_ms = (now() + due_offset_secs) * 1000;
        json!({
            "recordName": "Reminder/1",
            "recordType": "Reminder",
            "fields": {
                "Completed": { "type": "NUMBER_INT64", "value": i64::from(completed) },
                "DueDate": { "type": "TIMESTAMP", "value": due_ms },
                "List": { "type": "REFERENCE", "value": { "recordName": "List/a" } },
                "RecurrenceRuleIDs": if recurring {
                    json!({ "type": "STRING_LIST", "value": ["rule-1"] })
                } else {
                    json!({ "type": "EMPTY_LIST", "value": [] })
                },
                "LastModifiedDate": { "type": "TIMESTAMP", "value": now() * 1000 }
            }
        })
    }

    /// The bug this fixes: a daily task ticked this morning carries Completed=1 while its
    /// due date has already moved to tomorrow. Reading the flag literally made a list of
    /// 102 recurring reminders render as "All done here".
    #[test]
    fn a_recurring_reminder_due_ahead_is_outstanding() {
        let r = reminder_from_record(&record(true, true, 3600)).unwrap();
        assert!(!r.is_done(), "the next occurrence is work still to do");
    }

    #[test]
    fn a_recurring_reminder_already_past_stays_done() {
        let r = reminder_from_record(&record(true, true, -3600)).unwrap();
        assert!(
            r.is_done(),
            "a past occurrence that was ticked is genuinely finished"
        );
    }

    /// A one-off must keep behaving exactly as before.
    #[test]
    fn a_one_off_reminder_follows_its_flag() {
        assert!(reminder_from_record(&record(true, false, 3600))
            .unwrap()
            .is_done());
        assert!(!reminder_from_record(&record(false, false, -3600))
            .unwrap()
            .is_done());
    }

    #[test]
    fn the_due_date_is_carried_through() {
        let r = reminder_from_record(&record(false, false, 7200)).unwrap();
        let due = r.due_at.expect("a due date should be stored");
        assert!(
            (due - (now() + 7200)).abs() <= 2,
            "due date must survive the round trip"
        );
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn accepting() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "x" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    /// A created reminder must be readable again, or it shows up blank on the phone.
    #[tokio::test]
    async fn a_created_reminder_carries_its_title() {
        let server = accepting().await;
        let http = reqwest::Client::new();
        let name = create_cloudkit(&http, &server.uri(), vec![], "List/abc", "🥛 Buy milk")
            .await
            .unwrap();
        assert!(
            name.starts_with("Reminder/"),
            "the caller needs the record name: {name}"
        );

        let sent: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        let record = &sent["operations"][0]["record"];
        assert_eq!(sent["operations"][0]["operationType"], "create");
        assert_eq!(record["recordType"], "Reminder");
        assert_eq!(record["fields"]["Completed"]["value"], 0);
        assert_eq!(
            record["fields"]["List"]["value"]["recordName"], "List/abc",
            "it must land in the list the user chose"
        );

        let title = record["fields"]["TitleDocument"]["value"].as_str().unwrap();
        assert_eq!(
            crate::ckdocument::text_from_base64(title).as_deref(),
            Some("🥛 Buy milk"),
            "the title must decode back to what was typed"
        );
    }

    #[tokio::test]
    async fn an_empty_title_is_refused_before_any_request() {
        let server = accepting().await;
        let http = reqwest::Client::new();
        assert!(
            create_cloudkit(&http, &server.uri(), vec![], "List/abc", "   ")
                .await
                .is_err()
        );
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing should reach Apple"
        );
    }

    /// Deleting marks the flag Apple's own clients use, rather than destroying the record.
    #[tokio::test]
    async fn deleting_sets_the_flag_with_the_change_tag() {
        let server = accepting().await;
        let reminder = Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/abc".into(),
            title: "Gone".into(),
            notes: None,
            due_at: None,
            completed_at: None,
            priority: 0,
            parent_guid: None,
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("tag-3".into()),
            raw: None,
            timezone: None,
            section_guid: None,
        };
        delete_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], &reminder)
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .expect("a write was sent");
        let sent: Value = serde_json::from_slice(&modify.body).unwrap();
        assert_eq!(sent["operations"][0]["operationType"], "update");
        assert_eq!(
            sent["operations"][0]["record"]["fields"]["Deleted"]["value"],
            1
        );
        // Signed, so a phone merging field by field has a reason to take it.
        let tokens: Value = serde_json::from_str(
            sent["operations"][0]["record"]["fields"]["ResolutionTokenMap"]["value"]
                .as_str()
                .expect("the write carries resolution tokens"),
        )
        .unwrap();
        assert!(tokens["map"]["deleted"]["counter"].as_u64() >= Some(1));
        assert_eq!(
            sent["operations"][0]["record"]["recordChangeTag"], "tag-3",
            "the change tag guards against clobbering a newer edit"
        );
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A server that answers a lookup with the reminder as Apple currently holds it, and
    /// accepts the write that follows. A title or note edit reads the record first now, so
    /// the document it rewrites is the one Apple actually has.
    async fn accepting() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{
                    "recordName": "Reminder/1",
                    "recordType": "Reminder",
                    "recordChangeTag": "fresh",
                    "fields": {
                        "TitleDocument": {
                            "type": "ENCRYPTED_BYTES",
                            "value": crate::ckdocument::text_to_base64("Old"),
                        },
                        "NotesDocument": {
                            "type": "ENCRYPTED_BYTES",
                            "value": crate::ckdocument::text_to_base64("old notes"),
                        },
                    }
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "Reminder/1" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    fn reminder() -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/a".into(),
            title: "Old".into(),
            notes: None,
            due_at: None,
            completed_at: None,
            priority: 0,
            parent_guid: None,
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("tag-1".into()),
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    /// The write itself, not whatever lookup preceded it.
    async fn sent(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .expect("a modify was sent");
        let body: Value = serde_json::from_slice(&modify.body).unwrap();
        body["operations"][0]["record"]["fields"].clone()
    }

    /// How many writes were sent — lookups do not count.
    async fn writes(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/records/modify"))
            .count()
    }

    #[tokio::test]
    async fn a_new_title_is_written_so_it_reads_back() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                title: Some("🥛 Buy oat milk".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let fields = sent(&server).await;
        let encoded = fields["TitleDocument"]["value"].as_str().unwrap();
        assert_eq!(
            crate::ckdocument::text_from_base64(encoded).as_deref(),
            Some("🥛 Buy oat milk")
        );
        assert!(
            fields.get("DueDate").is_none(),
            "untouched fields must not be sent"
        );
    }

    /// The point of reading the record first. A title is a document, and around its text
    /// Apple keeps the state its devices use to reconcile edits. Building a fresh minimal
    /// document for a rename threw that away every time — the same mistake that stopped
    /// edited notes from ever reaching a phone.
    #[tokio::test]
    async fn renaming_rewrites_the_document_apple_holds_rather_than_replacing_it() {
        let server = MockServer::start().await;
        // A title document carrying something beyond the text, as a real one does.
        let existing = {
            let mut note =
                crate::ckdocument::debug_inflate(&crate::ckdocument::text_to_base64("Old"))
                    .unwrap();
            // Append a field the minimal encoder never writes, to stand for Apple's state.
            note.extend_from_slice(&[0x22, 0x02, 0xAB, 0xCD]);
            use base64::{engine::general_purpose::STANDARD as B64, Engine};
            B64.encode(note)
        };
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{
                    "recordName": "Reminder/1",
                    "recordChangeTag": "fresh",
                    "fields": {
                        "TitleDocument": { "type": "ENCRYPTED_BYTES", "value": existing }
                    }
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "Reminder/1" }] })),
            )
            .mount(&server)
            .await;

        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                title: Some("New".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let fields = sent(&server).await;
        let written = fields["TitleDocument"]["value"].as_str().unwrap();
        assert_eq!(
            crate::ckdocument::text_from_base64(written).as_deref(),
            Some("New"),
            "the new title is what it says"
        );
        let raw = crate::ckdocument::debug_inflate(written).unwrap();
        assert!(
            raw.windows(2).any(|w| w == [0xAB, 0xCD]),
            "and Apple's own state came through the rename intact"
        );
        // The tag used is the one the lookup just returned, not the caller's stale copy.
        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .unwrap();
        let body: Value = serde_json::from_slice(&modify.body).unwrap();
        assert_eq!(body["operations"][0]["record"]["recordChangeTag"], "fresh");
    }

    /// Seconds in, milliseconds out — a due date landing on the wrong day is worse than none.
    #[tokio::test]
    async fn a_due_date_is_written_in_milliseconds_and_can_be_cleared() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                due_at: Some(Some(1_757_000_000)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            sent(&server).await["DueDate"]["value"],
            1_757_000_000_000_i64
        );

        let clearing = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &clearing.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                due_at: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(sent(&clearing).await["DueDate"]["value"].is_null());
    }

    #[tokio::test]
    async fn notes_can_be_set_and_cleared() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                notes: Some(Some("bring a bag".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let encoded = sent(&server).await["NotesDocument"]["value"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            crate::ckdocument::text_from_base64(&encoded).as_deref(),
            Some("bring a bag")
        );
    }

    #[tokio::test]
    async fn an_empty_title_is_refused_before_anything_is_sent() {
        let server = accepting().await;
        assert!(edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                title: Some("   ".into()),
                ..Default::default()
            },
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// An edit that changes nothing should not cost a request.
    #[tokio::test]
    async fn an_empty_edit_does_nothing() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit::default(),
        )
        .await
        .unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn several_changes_travel_in_one_request() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &reminder(),
            &ReminderEdit {
                title: Some("Renamed".into()),
                due_at: Some(Some(1_757_000_000)),
                flagged: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(writes(&server).await, 1, "one write, not three");
        let fields = sent(&server).await;
        assert!(fields.get("TitleDocument").is_some());
        assert_eq!(fields["DueDate"]["value"], 1_757_000_000_000_i64);
        assert_eq!(fields["Flagged"]["value"], 1);
    }
}

#[cfg(test)]
mod list_write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn accepting() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "List/x" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    /// The write, not whatever lookup preceded it: an update with no local change tag
    /// fetches the current one first, which is a request of its own.
    async fn sent(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .expect("a modify was sent");
        serde_json::from_slice::<Value>(&modify.body).unwrap()["operations"][0].clone()
    }

    /// A list's name is a plain string, unlike a reminder's title — writing it as a document
    /// would leave the list unnamed on the phone.
    #[tokio::test]
    async fn a_created_list_carries_a_plain_name() {
        let server = accepting().await;
        let name =
            create_list_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], "Groceries")
                .await
                .unwrap();
        assert!(
            name.starts_with("List/"),
            "the caller needs the identifier: {name}"
        );

        let op = sent(&server).await;
        assert_eq!(op["operationType"], "create");
        assert_eq!(op["record"]["recordType"], "List");
        assert_eq!(op["record"]["fields"]["Name"]["value"], "Groceries");
        assert_eq!(op["record"]["fields"]["Name"]["type"], "STRING");
    }

    #[tokio::test]
    async fn renaming_sends_only_the_name() {
        let server = accepting().await;
        rename_list_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "List/a",
            "Errands",
        )
        .await
        .unwrap();
        let op = sent(&server).await;
        assert_eq!(op["operationType"], "update");
        assert_eq!(op["record"]["fields"]["Name"]["value"], "Errands");
        assert!(op["record"]["fields"].get("Deleted").is_none());
    }

    /// Deleting marks the flag Apple's own clients use rather than destroying the record.
    #[tokio::test]
    async fn deleting_marks_rather_than_destroys() {
        let server = accepting().await;
        delete_list_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], "List/a")
            .await
            .unwrap();
        let op = sent(&server).await;
        assert_eq!(op["operationType"], "update");
        assert_eq!(op["record"]["fields"]["Deleted"]["value"], 1);
    }

    #[tokio::test]
    async fn a_blank_name_is_refused_before_anything_is_sent() {
        let server = accepting().await;
        assert!(
            create_list_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], "  ")
                .await
                .is_err()
        );
        assert!(
            rename_list_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], "List/a", "")
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use serde_json::json;

    fn hint(ids: &[&str]) -> Value {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        let payload = json!({
            "topLevelElementIDs": ids,
            "secondaryLevelElementIDsByTopLevelElementID": {},
        });
        json!({
            "recordName": "ManualSortHint/1",
            "recordType": "ManualSortHint",
            "fields": {
                "HintDataIncludingRemindersInPrimaryAccountOnlyAsBytes": {
                    "type": "BYTES",
                    "value": B64.encode(serde_json::to_vec(&payload).unwrap()),
                }
            }
        })
    }

    fn list(guid: &str) -> ReminderList {
        ReminderList {
            guid: guid.into(),
            title: guid.into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }
    }

    /// The field is named AsBytes but holds JSON, which is what makes this readable at all.
    #[test]
    fn the_hint_payload_is_json_despite_its_name() {
        let order = list_order_from(&hint(&["AAA", "BBB"]));
        assert_eq!(order, vec!["AAA", "BBB"]);
    }

    /// The hint carries bare UUIDs; a list's record name is List/<uuid>.
    #[test]
    fn lists_take_the_order_the_phone_shows() {
        let mut lists = vec![list("List/CCC"), list("List/AAA"), list("List/BBB")];
        apply_list_order(
            &mut lists,
            &mut [],
            &["AAA".into(), "BBB".into(), "CCC".into()],
        );
        lists.sort_by_key(|l| l.sort_order);
        let order: Vec<&str> = lists.iter().map(|l| l.guid.as_str()).collect();
        assert_eq!(order, vec!["List/AAA", "List/BBB", "List/CCC"]);
    }

    /// A list the hint does not mention must not jump to the front.
    #[test]
    fn unmentioned_lists_sort_after_the_rest() {
        let mut lists = vec![list("List/NEW"), list("List/AAA")];
        apply_list_order(&mut lists, &mut [], &["AAA".into()]);
        assert!(
            lists
                .iter()
                .find(|l| l.guid == "List/NEW")
                .unwrap()
                .sort_order
                > lists
                    .iter()
                    .find(|l| l.guid == "List/AAA")
                    .unwrap()
                    .sort_order
        );
    }

    #[test]
    fn matching_ignores_case() {
        let mut lists = vec![list("List/abc")];
        apply_list_order(&mut lists, &mut [], &["ABC".into()]);
        assert_eq!(lists[0].sort_order, 0);
    }

    /// No hint, or an unreadable one, must leave the lists as they were.
    #[test]
    fn an_absent_or_broken_hint_changes_nothing() {
        let mut lists = vec![list("List/AAA")];
        lists[0].sort_order = 7;
        apply_list_order(&mut lists, &mut [], &[]);
        assert_eq!(lists[0].sort_order, 7);

        let broken = json!({
            "recordName": "ManualSortHint/1", "recordType": "ManualSortHint",
            "fields": { "HintDataIncludingRemindersInPrimaryAccountOnlyAsBytes":
                        { "type": "BYTES", "value": "!!not base64!!" } }
        });
        assert!(list_order_from(&broken).is_empty());
        assert!(list_order_from(&json!({ "recordName": "x", "fields": {} })).is_empty());
    }
}

#[cfg(test)]
mod recurring_tick_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn accepting() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "Reminder/1" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    /// 2100-01-01T00:00:00Z. Far enough ahead that one step forward is always the
    /// answer, so these tests measure the arithmetic and not today's date.
    const FUTURE: i64 = 4_102_444_800;

    fn repeating(frequency: i64, interval: i64, due: i64) -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/a".into(),
            title: "Water the plants".into(),
            notes: None,
            due_at: Some(due),
            completed_at: None,
            priority: 0,
            parent_guid: None,
            flagged: false,
            recur_frequency: Some(frequency),
            recur_interval: Some(interval),
            etag: Some("tag".into()),
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    /// The write, not whatever lookup preceded it — a title edit reads the record first so
    /// it can rewrite the document Apple holds rather than replace it.
    async fn fields(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .expect("a modify was sent");
        serde_json::from_slice::<Value>(&modify.body).unwrap()["operations"][0]["record"]["fields"]
            .clone()
    }

    /// The bug this fixes: Apple already holds a repeating reminder completed while its
    /// record points at the next occurrence, so writing Completed again changed nothing and
    /// the tick appeared to do nothing at all.
    #[tokio::test]
    async fn ticking_a_daily_reminder_moves_it_to_tomorrow() {
        let server = accepting().await;
        let due = FUTURE;
        set_completed_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &repeating(0, 1, due),
            true,
        )
        .await
        .unwrap();

        let sent = fields(&server).await;
        assert_eq!(sent["Completed"]["value"], 1);
        assert_eq!(
            sent["DueDate"]["value"],
            (due + 86_400) * 1000,
            "a daily reminder must advance exactly one day"
        );
    }

    #[tokio::test]
    async fn the_interval_is_respected() {
        let server = accepting().await;
        let due = FUTURE;
        set_completed_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &repeating(1, 2, due),
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            fields(&server).await["DueDate"]["value"],
            (due + 14 * 86_400) * 1000,
            "every two weeks means fourteen days"
        );
    }

    /// Un-ticking must not move the date; it just reopens the occurrence.
    #[tokio::test]
    async fn un_ticking_leaves_the_schedule_alone() {
        let server = accepting().await;
        set_completed_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &repeating(0, 1, FUTURE),
            false,
        )
        .await
        .unwrap();
        let sent = fields(&server).await;
        assert_eq!(sent["Completed"]["value"], 0);
        assert!(
            sent.get("DueDate").is_none(),
            "un-ticking must not reschedule"
        );
    }

    /// A one-off keeps behaving exactly as before.
    #[tokio::test]
    async fn a_one_off_just_completes() {
        let server = accepting().await;
        let mut once = repeating(0, 1, FUTURE);
        once.recur_frequency = None;
        once.recur_interval = None;
        set_completed_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], &once, true)
            .await
            .unwrap();
        let sent = fields(&server).await;
        assert_eq!(sent["Completed"]["value"], 1);
        assert!(sent.get("DueDate").is_none());
    }

    /// Months are calendar units, not 30-day blocks.
    #[test]
    fn monthly_and_yearly_advance_by_calendar() {
        // 2026-01-31T12:00:00Z
        let jan31 = 1_769_860_800;
        let monthly = repeating(2, 1, jan31).next_occurrence_after(jan31).unwrap();
        let as_date = jiff::Timestamp::from_second(monthly)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%d")
            .to_string();
        assert!(
            as_date.starts_with("2026-02"),
            "a month later, not 30 days: {as_date}"
        );

        let yearly = repeating(3, 1, jan31).next_occurrence_after(jan31).unwrap();
        assert!(yearly > monthly);
    }

    /// Skip a daily reminder for a week and tick it: it must land on the next occurrence
    /// that has not happened yet. Advancing a single interval would leave it six days
    /// overdue, which reads on screen as the tick having done nothing.
    #[test]
    fn ticking_an_overdue_repeat_catches_up_to_the_next_real_occurrence() {
        let week_ago = FUTURE - 7 * 86_400;
        let next = repeating(0, 1, week_ago)
            .next_occurrence_after(FUTURE)
            .unwrap();
        assert_eq!(next, FUTURE + 86_400, "the first occurrence still ahead");
    }

    /// The catch-up lands on the schedule, never merely on "now plus one".
    #[test]
    fn catching_up_stays_on_the_original_cadence() {
        // Due on a Monday, repeating every two weeks, ticked five weeks later.
        let start = FUTURE;
        let next = repeating(1, 2, start)
            .next_occurrence_after(start + 5 * 7 * 86_400)
            .unwrap();
        assert_eq!(
            next,
            start + 6 * 7 * 86_400,
            "six weeks on is the next multiple of the fortnight"
        );
    }

    /// A reminder abandoned years ago must still answer, not spin.
    #[test]
    fn a_long_abandoned_daily_reminder_still_resolves() {
        let ten_years = 3_653 * 86_400;
        let next = repeating(0, 1, FUTURE - ten_years)
            .next_occurrence_after(FUTURE)
            .unwrap();
        assert_eq!(next, FUTURE + 86_400);
    }

    #[test]
    fn a_reminder_with_no_due_date_has_no_next_occurrence() {
        let mut r = repeating(0, 1, 0);
        r.due_at = None;
        assert!(r.next_occurrence().is_none());
    }
}

#[cfg(test)]
mod priority_and_move_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn accepting() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "Reminder/1" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    fn plain() -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/a".into(),
            title: "Renew passport".into(),
            notes: None,
            due_at: None,
            completed_at: None,
            priority: 0,
            parent_guid: None,
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("tag".into()),
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    /// The write, not whatever lookup preceded it — a title edit reads the record first so
    /// it can rewrite the document Apple holds rather than replace it.
    async fn fields(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .expect("a modify was sent");
        serde_json::from_slice::<Value>(&modify.body).unwrap()["operations"][0]["record"]["fields"]
            .clone()
    }

    async fn write(edit: ReminderEdit) -> Value {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &plain(),
            &edit,
        )
        .await
        .unwrap();
        fields(&server).await
    }

    /// Apple's scale is 1 high, 5 medium, 9 low — the same one EventKit uses. Sending a
    /// number off it would show as an unknown marker on a phone.
    #[tokio::test]
    async fn priority_lands_on_apples_own_scale() {
        for (asked, stored) in [(1, 1), (2, 1), (5, 5), (6, 5), (9, 9), (7, 9), (0, 0)] {
            let sent = write(ReminderEdit {
                priority: Some(asked),
                ..ReminderEdit::default()
            })
            .await;
            assert_eq!(
                sent["Priority"]["value"], stored,
                "asking for {asked} should store {stored}"
            );
        }
    }

    #[tokio::test]
    async fn an_out_of_range_priority_becomes_none_rather_than_nonsense() {
        let sent = write(ReminderEdit {
            priority: Some(42),
            ..ReminderEdit::default()
        })
        .await;
        assert_eq!(sent["Priority"]["value"], 0);
    }

    /// Apple nests a reminder under another with a reference, and clears it with an
    /// explicit null — omitting the field would leave it nested where it was.
    #[tokio::test]
    async fn nesting_and_unnesting_both_write_the_reference() {
        let nested = write(ReminderEdit {
            parent_guid: Some(Some("Reminder/parent".into())),
            ..ReminderEdit::default()
        })
        .await;
        assert_eq!(nested["ParentReminder"]["type"], "REFERENCE");
        assert_eq!(
            nested["ParentReminder"]["value"]["recordName"],
            "Reminder/parent"
        );

        let freed = write(ReminderEdit {
            parent_guid: Some(None),
            ..ReminderEdit::default()
        })
        .await;
        assert_eq!(
            freed["ParentReminder"],
            Value::Null,
            "coming back out has to be written, not omitted"
        );
    }

    #[tokio::test]
    async fn moving_rewrites_the_list_reference() {
        let sent = write(ReminderEdit {
            list_guid: Some("List/b".into()),
            ..ReminderEdit::default()
        })
        .await;
        assert_eq!(sent["List"]["type"], "REFERENCE");
        assert_eq!(sent["List"]["value"]["recordName"], "List/b");
    }

    /// One request, so a move and a priority change cannot half-apply.
    #[tokio::test]
    async fn several_changes_go_in_one_write() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &plain(),
            &ReminderEdit {
                title: Some("Renew passport urgently".into()),
                priority: Some(1),
                list_guid: Some("List/b".into()),
                ..ReminderEdit::default()
            },
        )
        .await
        .unwrap();

        // One write, whatever it took to prepare it: a title edit reads the record first
        // so it can rewrite the existing document rather than replace it.
        let writes = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/records/modify"))
            .count();
        assert_eq!(writes, 1, "a single write, however it was prepared");
        let sent = fields(&server).await;
        assert_eq!(sent["Priority"]["value"], 1);
        assert_eq!(sent["List"]["value"]["recordName"], "List/b");
        assert!(sent.get("TitleDocument").is_some());
    }

    /// An edit that asks for nothing must not reach Apple at all.
    #[tokio::test]
    async fn an_empty_edit_sends_nothing() {
        let server = accepting().await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &plain(),
            &ReminderEdit::default(),
        )
        .await
        .unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[cfg(test)]
mod tombstone_tests {
    use super::*;
    use serde_json::json;

    fn library_with(guid: &str) -> quince_core::Library {
        let mut library = quince_core::Library::in_memory().unwrap();
        library
            .put_reminder_lists(&[ReminderList {
                guid: "List/a".into(),
                title: "Daily".into(),
                colour: None,
                ctag: None,
                sort_order: 0,
                enabled: true,
                ..Default::default()
            }])
            .unwrap();
        library
            .put_reminders(&[Reminder {
                guid: guid.into(),
                list_guid: "List/a".into(),
                title: "Deleted on the phone".into(),
                notes: None,
                due_at: None,
                completed_at: None,
                priority: 0,
                parent_guid: None,
                flagged: false,
                recur_frequency: None,
                recur_interval: None,
                etag: None,
                raw: None,
                timezone: None,
                section_guid: None,
            }])
            .unwrap();
        library
    }

    fn apply(library: &mut quince_core::Library, records: Vec<Value>) {
        apply_cloudkit(
            library,
            RemindersFetch {
                memberships: std::collections::HashMap::new(),
                records,
                token: Some("t".into()),
                incremental: true,
            },
        )
        .unwrap();
    }

    /// The bug this fixes: a record deleted elsewhere comes back as a tombstone with a name
    /// and nothing else — no recordType, no fields. Those were skipped, so anything deleted
    /// on a phone stayed on screen here forever.
    #[test]
    fn a_bare_tombstone_removes_the_reminder() {
        let mut library = library_with("Reminder/1");
        assert_eq!(library.reminders_in("List/a", true).unwrap().len(), 1);

        apply(
            &mut library,
            vec![json!({ "recordName": "Reminder/1", "deleted": true })],
        );
        assert!(
            library.reminders_in("List/a", true).unwrap().is_empty(),
            "a tombstone must remove the reminder"
        );
    }

    /// And a tombstone for a list takes the list with it.
    #[test]
    fn a_bare_tombstone_removes_the_list() {
        let mut library = library_with("Reminder/1");
        apply(
            &mut library,
            vec![json!({ "recordName": "List/a", "deleted": true })],
        );
        assert!(library.reminder_lists().unwrap().is_empty());
    }

    /// A tombstone that is not marked deleted is just a record we cannot read, and guessing
    /// it away would delete things that still exist.
    #[test]
    fn an_untyped_record_that_is_not_deleted_is_left_alone() {
        let mut library = library_with("Reminder/1");
        apply(&mut library, vec![json!({ "recordName": "Reminder/1" })]);
        assert_eq!(
            library.reminders_in("List/a", true).unwrap().len(),
            1,
            "only an explicit deletion may remove anything"
        );
    }

    /// The floor under tombstones: a tombstone is delivered once, so a sync that consumed
    /// one before this code knew what to do with it would leave that record on screen
    /// forever. A full pass saw everything Apple has, so anything else is gone.
    #[test]
    fn a_full_pass_removes_what_apple_no_longer_reports() {
        let mut library = library_with("Reminder/ghost");
        apply_cloudkit(
            &mut library,
            RemindersFetch {
                memberships: std::collections::HashMap::new(),
                records: vec![json!({
                    "recordName": "List/a",
                    "recordType": "List",
                    "fields": { "Name": { "type": "STRING", "value": "Daily" } }
                })],
                token: Some("t".into()),
                incremental: false,
            },
        )
        .unwrap();
        assert!(
            library.reminders_in("List/a", true).unwrap().is_empty(),
            "a full pass is the whole truth"
        );
    }

    /// And the opposite must never happen: an incremental pass carries only changes, so
    /// pruning there would empty the library on the first refresh.
    #[test]
    fn an_incremental_pass_never_prunes() {
        let mut library = library_with("Reminder/1");
        apply(&mut library, vec![]);
        assert_eq!(
            library.reminders_in("List/a", true).unwrap().len(),
            1,
            "absence in a delta means unchanged, not deleted"
        );
    }

    /// A name that matches nothing must not disturb what is there.
    #[test]
    fn a_tombstone_for_something_we_never_had_is_harmless() {
        let mut library = library_with("Reminder/1");
        apply(
            &mut library,
            vec![json!({ "recordName": "Reminder/999", "deleted": true })],
        );
        assert_eq!(library.reminders_in("List/a", true).unwrap().len(), 1);
        assert_eq!(library.reminder_lists().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod recurrence_write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn dated() -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/a".into(),
            title: "Water the plants".into(),
            notes: None,
            due_at: Some(4_102_444_800),
            completed_at: None,
            priority: 0,
            parent_guid: None,
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("tag".into()),
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    /// A server that answers a lookup with whatever rule ids the reminder currently has.
    async fn server(existing: &[&str]) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{
                    "recordName": "Reminder/1",
                    "recordType": "Reminder",
                    "recordChangeTag": "fresh",
                    "fields": {
                        "RecurrenceRuleIDs": { "type": "STRING_LIST", "value": existing }
                    }
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "Reminder/1" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    /// Every modify, in the order it was sent.
    async fn operations(server: &MockServer) -> Vec<Value> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/records/modify"))
            .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["operations"][0].clone())
            .collect()
    }

    #[tokio::test]
    async fn setting_a_repeat_writes_a_rule_and_links_it() {
        let server = server(&[]).await;
        set_recurrence_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &dated(),
            Some(Repeat {
                frequency: 1,
                interval: 2,
            }),
        )
        .await
        .unwrap();

        let ops = operations(&server).await;
        assert_eq!(ops.len(), 2, "a rule, then the link");

        let rule = &ops[0];
        assert_eq!(rule["operationType"], "create");
        assert_eq!(rule["record"]["recordType"], "RecurrenceRule");
        assert_eq!(rule["record"]["fields"]["Frequency"]["value"], 1);
        assert_eq!(rule["record"]["fields"]["Interval"]["value"], 2);
        assert_eq!(
            rule["record"]["fields"]["Reminder"]["value"]["recordName"],
            "Reminder/1"
        );

        // The reminder holds the bare id; the record is named with a prefix.
        let link = &ops[1];
        let id = link["record"]["fields"]["RecurrenceRuleIDs"]["value"][0]
            .as_str()
            .unwrap();
        assert!(
            !id.starts_with("RecurrenceRule/"),
            "the list holds a bare id: {id}"
        );
        assert_eq!(
            rule["record"]["recordName"].as_str().unwrap(),
            format!("RecurrenceRule/{id}"),
            "the rule written must be the rule pointed at"
        );
    }

    /// The order is the whole point: the reminder must never refer to a rule that is gone.
    #[tokio::test]
    async fn the_old_rule_is_retired_only_after_the_link_moves() {
        let server = server(&["OLD-1"]).await;
        set_recurrence_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &dated(),
            Some(Repeat {
                frequency: 0,
                interval: 1,
            }),
        )
        .await
        .unwrap();

        let ops = operations(&server).await;
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0]["operationType"], "create", "the new rule first");
        assert!(
            ops[1]["record"]["fields"]["RecurrenceRuleIDs"].is_object(),
            "then the reminder is repointed"
        );
        assert_eq!(
            ops[2]["operationType"], "forceDelete",
            "then the old rule goes"
        );
        assert_eq!(ops[2]["record"]["recordName"], "RecurrenceRule/OLD-1");
    }

    #[tokio::test]
    async fn clearing_a_repeat_empties_the_list_and_removes_the_rule() {
        let server = server(&["OLD-1"]).await;
        set_recurrence_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &dated(),
            None,
        )
        .await
        .unwrap();

        let ops = operations(&server).await;
        assert_eq!(ops.len(), 2, "no rule is created when clearing");
        assert_eq!(
            ops[0]["record"]["fields"]["RecurrenceRuleIDs"]["value"],
            json!([])
        );
        assert_eq!(ops[1]["record"]["recordName"], "RecurrenceRule/OLD-1");
    }

    /// A repeat with nothing to repeat from would simply never come back.
    #[tokio::test]
    async fn a_repeat_needs_a_date_and_says_so_before_sending_anything() {
        let server = server(&[]).await;
        let mut undated = dated();
        undated.due_at = None;
        let err = set_recurrence_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &undated,
            Some(Repeat {
                frequency: 0,
                interval: 1,
            }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("date"), "{err}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_frequency_apple_does_not_know_is_refused() {
        let server = server(&[]).await;
        assert!(set_recurrence_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &dated(),
            Some(Repeat {
                frequency: 9,
                interval: 1,
            }),
        )
        .await
        .is_err());
        assert!(
            operations(&server).await.is_empty(),
            "nothing may be written for a rule Apple would reject"
        );
    }

    /// Clearing a repeat on a reminder with no date is legitimate — it is the date that the
    /// repeat needed, not the removal.
    #[tokio::test]
    async fn clearing_works_even_without_a_date() {
        let server = server(&["OLD-1"]).await;
        let mut undated = dated();
        undated.due_at = None;
        set_recurrence_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &undated,
            None,
        )
        .await
        .unwrap();
        assert_eq!(operations(&server).await.len(), 2);
    }
}

#[cfg(test)]
mod list_shape_tests {
    use super::*;
    use serde_json::json;

    fn list_record(name: &str, title: &str, group: bool) -> Value {
        json!({
            "recordName": name,
            "recordType": "List",
            "fields": {
                "Name": { "type": "STRING", "value": title },
                "IsGroup": { "type": "NUMBER_INT64", "value": i64::from(group) },
            }
        })
    }

    /// What a list record on a real account carries, copied from one.
    fn gym_record() -> Value {
        json!({
            "recordName": "List/B54667C9-8785-4A05-B669-EACC5DA5E2DE",
            "recordType": "List",
            "fields": {
                "Name": { "type": "STRING", "value": "GYM" },
                "IsGroup": { "type": "NUMBER_INT64", "value": 0 },
                "BadgeEmblem": { "type": "STRING", "value": "fitness" },
                "Color": { "type": "STRING", "value": "{\"alpha\":1,\"red\":0,\"colorRGBSpace\":2,\"daHexString\":\"#0088FF\",\"ckSymbolicColorName\":\"blue\",\"green\":0.53333333333333333,\"daSymbolicColorName\":\"blue\",\"blue\":1}" },
                "ParentList": { "type": "REFERENCE", "value": { "recordName": "List/43A40727-5DAF-4107-B7C9-6B9161592F5D", "action": "VALIDATE" } },
                "PinnedDate": { "type": "TIMESTAMP", "value": 1781974802494_i64 },
                "SortingStyle": { "type": "STRING", "value": "displayDate_asc" },
            }
        })
    }

    /// The colour arrives as JSON; stored as it came, every list drew white.
    #[test]
    fn a_list_keeps_its_colour_glyph_group_and_order() {
        let l = list_from_record(&gym_record()).unwrap();
        assert_eq!(l.colour.as_deref(), Some("#0088FF"));
        assert_eq!(l.emblem.as_deref(), Some("fitness"));
        assert_eq!(
            l.group_guid.as_deref(),
            Some("List/43A40727-5DAF-4107-B7C9-6B9161592F5D")
        );
        assert!(l.pinned);
        assert_eq!(l.sorting.as_deref(), Some("displayDate_asc"));
    }

    #[test]
    fn a_colour_is_read_in_every_form_it_comes_in() {
        assert_eq!(hex_colour("#ffcc00").as_deref(), Some("#FFCC00"));
        assert_eq!(
            hex_colour("{\"daHexString\":\"#5AC8FA\",\"red\":0.35}").as_deref(),
            Some("#5AC8FA")
        );
        // Components alone, the hex worked out from them.
        assert_eq!(
            hex_colour("{\"red\":1,\"green\":0.8,\"blue\":0}").as_deref(),
            Some("#FFCC00")
        );
        for rubbish in ["", "blue", "{}", "{\"red\":\"x\"}", "#12345", "#GGGGGG"] {
            assert_eq!(hex_colour(rubbish), None, "{rubbish:?}");
        }
    }

    /// Unpinning leaves a date in year one behind rather than clearing the field.
    #[test]
    fn a_list_unpinned_on_the_phone_is_not_pinned() {
        let mut r = gym_record();
        r["fields"]["PinnedDate"]["value"] = json!(-62_135_769_600_000_i64);
        assert!(!list_from_record(&r).unwrap().pinned);
        r["fields"].as_object_mut().unwrap().remove("PinnedDate");
        assert!(!list_from_record(&r).unwrap().pinned);
    }

    #[test]
    fn a_group_is_kept_as_a_group() {
        let g = group_from_record(&list_record("List/A", "Life", true)).unwrap();
        assert_eq!((g.guid.as_str(), g.title.as_str()), ("List/A", "Life"));
        assert!(group_from_record(&list_record("List/B", "GYM", false)).is_none());
    }

    /// A group is a folder of lists. Two of the nine on a real account are groups, and shown as
    /// lists they read as permanently empty — and can be chosen as the place to file a reminder,
    /// which puts it where the phone will not show it.
    #[test]
    fn a_group_is_not_offered_as_a_list() {
        assert!(list_from_record(&list_record("List/A", "Life", true)).is_none());
        assert_eq!(
            list_from_record(&list_record("List/B", "GYM", false))
                .unwrap()
                .title,
            "GYM"
        );
    }

    /// Apple's List record has never had a SortOrder field; reading one scored every list zero
    /// and left the sidebar to chance.
    #[test]
    fn a_list_does_not_invent_a_rank_from_a_field_apple_does_not_send() {
        let l = list_from_record(&list_record("List/B", "GYM", false)).unwrap();
        assert_eq!(
            l.sort_order, 0,
            "the rank comes from the walk, not the record"
        );
    }

    /// The arrangement on a real account names eight lists of which seven are long gone.
    /// Obeying it tied every surviving list on the same rank.
    #[test]
    fn an_arrangement_that_no_longer_describes_the_account_is_ignored() {
        let mut lists: Vec<ReminderList> = ["List/A", "List/B", "List/C", "List/D"]
            .iter()
            .enumerate()
            .map(|(i, g)| ReminderList {
                guid: (*g).into(),
                title: (*g).into(),
                colour: None,
                ctag: None,
                sort_order: i64::try_from(i).unwrap(),
                enabled: true,
                ..Default::default()
            })
            .collect();
        let before: Vec<i64> = lists.iter().map(|l| l.sort_order).collect();
        apply_list_order(
            &mut lists,
            &mut [],
            &["GONE1".into(), "GONE2".into(), "GONE3".into(), "A".into()],
        );
        let after: Vec<i64> = lists.iter().map(|l| l.sort_order).collect();
        assert_eq!(before, after, "a stale arrangement changes nothing");
    }

    /// One new list is not a stale arrangement.
    #[test]
    fn an_arrangement_missing_only_a_new_list_is_still_obeyed() {
        let mut lists: Vec<ReminderList> = ["List/A", "List/B", "List/NEW"]
            .iter()
            .enumerate()
            .map(|(i, g)| ReminderList {
                guid: (*g).into(),
                title: (*g).into(),
                colour: None,
                ctag: None,
                sort_order: i64::try_from(i).unwrap(),
                enabled: true,
                ..Default::default()
            })
            .collect();
        apply_list_order(&mut lists, &mut [], &["B".into(), "A".into()]);
        lists.sort_by_key(|l| l.sort_order);
        let order: Vec<&str> = lists.iter().map(|l| l.guid.as_str()).collect();
        assert_eq!(order, vec!["List/B", "List/A", "List/NEW"]);
    }
}

/// An account that looks nothing like the one this was built against.
///
/// Everything here was measured on one real library with nine lists, three sections and
/// seven hundred reminders. Someone installing this may have none of that: a new account with
/// no lists at all, one list and nothing in it, or a set of lists whose names and ids look
/// nothing like these. These check the shapes that account would produce.
#[cfg(test)]
mod other_accounts_tests {
    use super::*;
    use serde_json::json;

    fn list_record(name: &str, title: &str, group: bool) -> Value {
        json!({
            "recordName": name,
            "recordType": "List",
            "fields": {
                "Name": { "type": "STRING", "value": title },
                "IsGroup": { "type": "NUMBER_INT64", "value": i64::from(group) },
            }
        })
    }

    /// A brand new account: Apple answers with a zone and nothing in it.
    #[test]
    fn an_empty_account_syncs_to_an_empty_library_rather_than_failing() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let stored = apply_cloudkit(
            &mut lib,
            RemindersFetch {
                memberships: std::collections::HashMap::new(),
                records: Vec::new(),
                token: Some("t1".into()),
                incremental: false,
            },
        )
        .unwrap();
        assert_eq!(stored.0, 0, "no lists");
        assert_eq!(stored.1, 0, "no reminders");
        assert!(lib.reminder_lists().unwrap().is_empty());
    }

    /// Every list on an account can be a group — someone who files everything in folders and
    /// has since emptied them. There is then nowhere to put a reminder, and saying so beats
    /// putting one where no device will show it.
    #[test]
    fn an_account_whose_lists_are_all_groups_offers_none_of_them() {
        let records = [
            list_record("List/A", "Work", true),
            list_record("List/B", "Home", true),
        ];
        let lists: Vec<ReminderList> = records.iter().filter_map(list_from_record).collect();
        assert!(lists.is_empty());
    }

    /// A one-list account must not be reordered into nonsense by an arrangement it predates.
    #[test]
    fn a_single_list_account_keeps_its_one_list() {
        let mut lists = vec![ReminderList {
            guid: "List/ONLY".into(),
            title: "Reminders".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }];
        apply_list_order(&mut lists, &mut [], &[]);
        assert_eq!(lists[0].sort_order, 0);
        apply_list_order(&mut lists, &mut [], &["SOMETHING-ELSE".into()]);
        assert_eq!(
            lists[0].sort_order, 0,
            "an arrangement about other lists is not ours"
        );
    }

    /// Apple's own list names are translated, and a person's are whatever they typed. Nothing
    /// here may depend on a particular word.
    #[test]
    fn a_list_is_recognised_by_its_record_not_by_what_it_is_called() {
        for title in ["Erinnerungen", "रिमाइंडर", "🌈", "  ", "Assignments"] {
            let parsed = list_from_record(&list_record("List/X", title, false));
            assert!(parsed.is_some(), "{title:?} must still be a list");
        }
    }

    /// A title Apple sends as empty still needs something to show.
    #[test]
    fn a_list_with_no_name_still_has_one() {
        let bare = json!({
            "recordName": "List/X",
            "recordType": "List",
            "fields": {}
        });
        assert_eq!(list_from_record(&bare).unwrap().title, "Reminders");
    }
}

/// An `Alarm` record, as the reminder and trigger it ties together.
///
/// Measured on the account: `Reminder` is a reference to `Reminder/<id>`, `TriggerID` a bare
/// id naming `AlarmTrigger/<id>`, and `AcknowledgedDate` — on the alarms someone has already
/// seen go off — a timestamp. An alarm pointing at no reminder (there are dozens, left from
/// reminders long gone) rings for nothing and is not kept.
fn alarm_from_record(record: &Value) -> Option<quince_core::alarms::Alarm> {
    let name = field::record_name(record)?;
    Some(quince_core::alarms::Alarm {
        id: name.strip_prefix("Alarm/").unwrap_or(&name).to_string(),
        reminder_guid: field::reference(record, "Reminder")?,
        trigger_id: field::string(record, "TriggerID")?,
        acknowledged_at: field::timestamp_seconds(record, "AcknowledgedDate"),
    })
}

/// An `AlarmTrigger` record, its moment resolved. Only date triggers ring here — every one on
/// the account is one; a location trigger has no moment to wait for.
fn trigger_from_record(record: &Value) -> Option<quince_core::alarms::AlarmTrigger> {
    let name = field::record_name(record)?;
    if field::string(record, "Type").as_deref() != Some("Date") {
        return None;
    }
    let data = record
        .pointer("/fields/DateComponentsData/value")
        .and_then(Value::as_str)?;
    Some(quince_core::alarms::AlarmTrigger {
        id: name
            .strip_prefix("AlarmTrigger/")
            .unwrap_or(&name)
            .to_string(),
        fires_at: moment_of(data)?,
    })
}

/// The instant an alarm's date components name.
///
/// They are a wall clock and a zone — nine in the morning, in Kolkata — not an instant, which
/// is what keeps a reminder at nine wherever its owner is. Components with no zone are
/// floating and mean this machine's own nine o'clock.
fn moment_of(components_base64: &str) -> Option<i64> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let parsed: Value = serde_json::from_slice(&B64.decode(components_base64).ok()?).ok()?;
    let part = |k: &str| parsed.get(k).and_then(Value::as_i64);
    let zone = parsed
        .pointer("/timeZone/identifier")
        .and_then(Value::as_str)
        .and_then(|id| jiff::tz::TimeZone::get(id).ok())
        .unwrap_or_else(jiff::tz::TimeZone::system);
    let at = jiff::civil::date(
        i16::try_from(part("year")?).ok()?,
        i8::try_from(part("month")?).ok()?,
        i8::try_from(part("day")?).ok()?,
    )
    .at(
        i8::try_from(part("hour").unwrap_or(0)).ok()?,
        i8::try_from(part("minute").unwrap_or(0)).ok()?,
        i8::try_from(part("second").unwrap_or(0)).ok()?,
        0,
    );
    Some(at.to_zoned(zone).ok()?.timestamp().as_second())
}

/// The date components an alarm is set by, as Apple writes them.
///
/// `DateComponentsData` is declared BYTES and holds JSON — the same trick the sort hint uses.
/// Written out rather than derived from a timestamp at read time because the components carry
/// a named zone: a reminder set for nine in the morning is for nine in the morning wherever
/// the person happens to be, which a fixed instant is not.
fn date_components(at_seconds: i64, timezone: &str) -> Result<String> {
    let zone = jiff::tz::TimeZone::get(timezone).unwrap_or_else(|_| jiff::tz::TimeZone::system());
    let moment = jiff::Timestamp::from_second(at_seconds)
        .map_err(|e| Error::Protocol(format!("that is not a time: {e}")))?
        .to_zoned(zone.clone());
    let components = serde_json::json!({
        "era": 1,
        "year": moment.year(),
        "month": moment.month(),
        "day": moment.day(),
        "hour": moment.hour(),
        "minute": moment.minute(),
        "second": 0,
        "timeZone": { "identifier": zone.iana_name().unwrap_or("UTC") },
    });
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    Ok(B64.encode(
        serde_json::to_vec(&components)
            .map_err(|e| Error::Protocol(format!("could not write the alarm's date: {e}")))?,
    ))
}

/// Give a reminder the alert that makes a phone actually ring.
///
/// A due date on its own notifies nobody. Every reminder on a real account that has a time has
/// exactly one `Alarm`, which points at an `AlarmTrigger` holding the date to fire on — three
/// records, not one field. Reminders written here had the date and no alarm, so they showed
/// the right time on a phone and then said nothing when it arrived.
///
/// An all-day reminder has no alarm, which is also what the phone does: there is no moment to
/// ring at. Clearing the date takes the alarm away with it.
///
/// # Errors
/// When Apple refuses any of the three writes, or the reminder cannot be read back.
pub async fn set_alarm_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    reminder_guid: &str,
    at_seconds: Option<i64>,
    timezone: &str,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let current = client.lookup_record(ZONE, reminder_guid).await?;
    let existing: Vec<String> = current
        .as_ref()
        .and_then(|r| r.pointer("/fields/AlarmIDs/value"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();

    // Whatever was there goes, whether it is being replaced or removed. An alarm left behind
    // pointing at an old time is worse than none: the phone rings for something that moved.
    for id in &existing {
        let alarm = client.lookup_record(ZONE, &format!("Alarm/{id}")).await?;
        if let Some(trigger) = alarm
            .as_ref()
            .and_then(|a| a.pointer("/fields/TriggerID/value"))
            .and_then(Value::as_str)
        {
            client
                .delete_record(ZONE, &format!("AlarmTrigger/{trigger}"))
                .await?;
        }
        client.delete_record(ZONE, &format!("Alarm/{id}")).await?;
    }

    let Some(at) = at_seconds else {
        return client
            .update_record(
                ZONE,
                reminder_guid,
                "Reminder",
                None,
                serde_json::json!({ "AlarmIDs": { "type": "STRING_LIST", "value": [] } }),
            )
            .await
            .map(|_| ());
    };

    let alarm_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let trigger_id = uuid::Uuid::new_v4().to_string().to_uppercase();

    // The alarm first: the trigger points back at it with a reference Apple validates, so it
    // has to exist by then. The alarm's own TriggerID is a plain string and can name a record
    // that is still to come.
    client
        .create_record(
            ZONE,
            &format!("Alarm/{alarm_id}"),
            "Alarm",
            serde_json::json!({
                "AlarmUID": { "type": "STRING", "value": alarm_id },
                "TriggerID": { "type": "STRING", "value": trigger_id },
                "Reminder": client.reference_to(ZONE, reminder_guid),
                "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                "Imported": { "type": "NUMBER_INT64", "value": 0 },
            }),
        )
        .await?;

    client
        .create_record(
            ZONE,
            &format!("AlarmTrigger/{trigger_id}"),
            "AlarmTrigger",
            serde_json::json!({
                "Alarm": client.reference_to(ZONE, &format!("Alarm/{alarm_id}")),
                "DateComponentsData": {
                    "type": "BYTES",
                    "value": date_components(at, timezone)?,
                },
                "Type": { "type": "STRING", "value": "Date" },
                "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                "Imported": { "type": "NUMBER_INT64", "value": 0 },
            }),
        )
        .await?;

    client
        .update_record(
            ZONE,
            reminder_guid,
            "Reminder",
            None,
            serde_json::json!({
                "AlarmIDs": { "type": "STRING_LIST", "value": [alarm_id] },
            }),
        )
        .await
        .map(|_| ())
}

#[cfg(test)]
mod alarm_tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    fn decoded(at: i64, tz: &str) -> Value {
        serde_json::from_slice(&B64.decode(date_components(at, tz).unwrap()).unwrap()).unwrap()
    }

    /// An alarm the phone set — 9:15 PM on 21 October 2025 in Kolkata, copied from the
    /// account — is that instant, and one written here reads back as what was written.
    #[test]
    fn an_alarms_moment_is_its_wall_clock_in_its_zone() {
        let phone = B64.encode(
            r#"{"second":0,"month":10,"day":21,"minute":15,"hour":21,"year":2025,"era":1,"timeZone":{"identifier":"Asia\/Kolkata"}}"#,
        );
        // 21:15 IST is 15:45 UTC.
        assert_eq!(moment_of(&phone), Some(1_761_061_500));
        let at = 1_789_382_400;
        assert_eq!(
            moment_of(&date_components(at, "America/New_York").unwrap()),
            Some(at)
        );
        assert_eq!(moment_of("not base64 at all"), None);
    }

    #[test]
    fn a_trigger_that_is_not_a_date_has_no_moment() {
        let record = serde_json::json!({
            "recordName": "AlarmTrigger/X",
            "recordType": "AlarmTrigger",
            "fields": { "Type": { "type": "STRING", "value": "Location" } }
        });
        assert!(trigger_from_record(&record).is_none());
    }

    /// Apple's own alarm on this account decodes to exactly this shape.
    #[test]
    fn the_components_are_the_local_wall_clock_not_the_instant() {
        // A whole minute, the way an alarm always lands on one.
        let at = 1_789_382_400;
        let india = decoded(at, "Asia/Kolkata");
        let utc = decoded(at, "UTC");
        assert_eq!(india["era"], 1);
        assert_eq!(india["second"], 0);
        assert_eq!(india["timeZone"]["identifier"], "Asia/Kolkata");
        assert_eq!(utc["timeZone"]["identifier"], "UTC");
        assert_ne!(
            india["hour"], utc["hour"],
            "the same instant is a different hour in each place, which is the point"
        );
    }

    /// Nine in the morning is nine in the morning wherever the person is, so the zone travels
    /// with the alarm rather than being resolved away.
    #[test]
    fn a_named_zone_is_carried_rather_than_flattened() {
        let v = decoded(1_757_000_000, "America/New_York");
        assert_eq!(v["timeZone"]["identifier"], "America/New_York");
    }

    /// A zone this machine has never heard of must not take the alarm down with it.
    #[test]
    fn an_unknown_zone_falls_back_rather_than_failing() {
        assert!(date_components(1_757_000_000, "Mars/Olympus_Mons").is_ok());
        assert!(date_components(1_757_000_000, "").is_ok());
    }
}

#[cfg(test)]
mod membership_tests {
    use super::*;

    /// The shape Apple's membership file actually has, taken from a real list: some entries
    /// name a section in groupID, some carry no groupID and sit outside every section.
    #[test]
    fn placements_are_read_and_the_unplaced_are_left_out() {
        let file = br#"{"minimumSupportedVersion":20230430,"memberships":[
            {"memberID":"6B06F5A6","modifiedOn":775500357.6},
            {"memberID":"D21F787F","modifiedOn":775501474.6,"groupID":"5D09C4F4"},
            {"groupID":"5D09C4F4","modifiedOn":775501474.6,"memberID":"D2CE1947"}
        ]}"#;
        assert_eq!(
            memberships_from(file),
            vec![
                (
                    "Reminder/D21F787F".to_string(),
                    "ListSection/5D09C4F4".to_string()
                ),
                (
                    "Reminder/D2CE1947".to_string(),
                    "ListSection/5D09C4F4".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_places_nothing() {
        assert!(memberships_from(b"").is_empty());
        assert!(memberships_from(b"not json").is_empty());
        assert!(memberships_from(br#"{"memberships":"wrong"}"#).is_empty());
        assert!(memberships_from(br#"{"other":[]}"#).is_empty());
    }
}

/// Where lists and groups sit, as the phone has them.
#[cfg(test)]
mod account_order_tests {
    use super::*;
    use serde_json::json;

    fn list(name: &str, title: &str, group: Option<&str>, is_group: bool) -> Value {
        let mut fields = json!({
            "Name": { "type": "STRING", "value": title },
            "IsGroup": { "type": "NUMBER_INT64", "value": i64::from(is_group) },
        });
        if let Some(parent) = group {
            fields["ParentList"] =
                json!({ "type": "REFERENCE", "value": { "recordName": parent } });
        }
        json!({ "recordName": name, "recordType": "List", "fields": fields })
    }

    fn account(order: &[&str]) -> Value {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        let wire: Vec<usize> = (0..order.len()).rev().collect();
        let bytes = crate::mergeable::build::ordered_set(order, &wire);
        json!({
            "recordName": "Account/x",
            "recordType": "Account",
            "fields": { "ListIDsMergeableOrdering": { "type": "BYTES", "value": B64.encode(bytes) } }
        })
    }

    fn pass(library: &mut quince_core::Library, records: Vec<Value>, incremental: bool) {
        apply_cloudkit(
            library,
            RemindersFetch {
                memberships: std::collections::HashMap::new(),
                records,
                token: Some("t".into()),
                incremental,
            },
        )
        .unwrap();
    }

    fn walked() -> Vec<Value> {
        // Walked in an order that is not the phone's.
        vec![
            list("List/GOALS", "Life Goals", Some("List/LIFE"), false),
            list("List/LIFE", "Life", None, true),
            list("List/DAILY", "Daily", Some("List/LIFE"), false),
            list("List/UNI", "University", None, true),
            list("List/TODO", "To-Do", None, false),
        ]
    }

    /// The account record's order is the phone's, for groups as much as lists. The stale
    /// hint beside it named lists that no longer exist, so the sidebar followed the walk.
    #[test]
    fn lists_and_groups_follow_the_account_record() {
        let mut library = quince_core::Library::in_memory().unwrap();
        let mut records = walked();
        records.push(account(&["DAILY", "TODO", "UNI", "LIFE", "GOALS"]));
        pass(&mut library, records, false);

        let order = |lists: Vec<ReminderList>| -> Vec<String> {
            let mut lists = lists;
            lists.sort_by_key(|l| l.sort_order);
            lists.into_iter().map(|l| l.title).collect()
        };
        assert_eq!(
            order(library.reminder_lists().unwrap()),
            ["Daily", "To-Do", "Life Goals"]
        );
        let groups: Vec<String> = library
            .reminder_groups()
            .unwrap()
            .into_iter()
            .map(|g| g.title)
            .collect();
        assert_eq!(groups, ["University", "Life"]);
        // And groups and lists share one sequence: University is before Life Goals's group.
        let uni = library.reminder_groups().unwrap()[0].sort_order;
        let todo = library
            .reminder_lists()
            .unwrap()
            .iter()
            .find(|l| l.title == "To-Do")
            .unwrap()
            .sort_order;
        assert!(todo < uni);
    }

    /// A later pass with no order in it leaves every list where it was.
    #[test]
    fn a_pass_without_an_order_keeps_the_places_it_has() {
        let mut library = quince_core::Library::in_memory().unwrap();
        let mut records = walked();
        records.push(account(&["DAILY", "TODO", "UNI", "LIFE", "GOALS"]));
        pass(&mut library, records, false);
        let before: Vec<(String, i64)> = library
            .reminder_lists()
            .unwrap()
            .into_iter()
            .map(|l| (l.title, l.sort_order))
            .collect();

        // Only Life Goals changed; it arrives first in this walk.
        pass(
            &mut library,
            vec![list("List/GOALS", "Life Goals", Some("List/LIFE"), false)],
            true,
        );
        let after: Vec<(String, i64)> = library
            .reminder_lists()
            .unwrap()
            .into_iter()
            .map(|l| (l.title, l.sort_order))
            .collect();
        assert_eq!(before, after);
    }
}

/// A reminder made here takes its place in its list's own order, as one made on the phone does.
#[cfg(test)]
mod list_order_write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn a_new_reminder_joins_the_end_of_its_list() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "records": [{
                "recordName": "List/L", "recordType": "List", "recordChangeTag": "t1",
                "fields": {
                    "ReminderIDs": { "type": "STRING", "value": "[\"AAA\",\"BBB\"]" },
                    "ResolutionTokenMap": { "type": "STRING", "value":
                        "{\"map\":{\"reminderIDsMergeableOrdering\":{\"counter\":123,\"modificationTime\":1,\"replicaID\":\"P\"}}}" }
                }
            }]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "x" }] })),
            )
            .mount(&server)
            .await;

        let made = create_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "List/L",
            "Buy milk",
        )
        .await
        .unwrap();
        let uuid = made.trim_start_matches("Reminder/").to_string();

        let lists: Vec<Value> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/records/modify"))
            .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
            .map(|b| b["operations"][0]["record"].clone())
            .filter(|r| r["recordType"] == "List")
            .collect();
        assert_eq!(lists.len(), 1, "the list's order is written once");
        let ids: Vec<String> =
            serde_json::from_str(lists[0]["fields"]["ReminderIDs"]["value"].as_str().unwrap())
                .unwrap();
        assert_eq!(ids, vec!["AAA".to_string(), "BBB".into(), uuid]);
        let tokens: Value = serde_json::from_str(
            lists[0]["fields"]["ResolutionTokenMap"]["value"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            tokens["map"]["reminderIDsMergeableOrdering"]["counter"],
            124
        );
    }
}
