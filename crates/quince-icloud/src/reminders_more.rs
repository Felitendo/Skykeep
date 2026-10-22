//! The rest of what the phone's Details screen shows for a reminder: its repeat whole (with
//! its end), its time zone, Urgent, a place it rings at, and the person it comes up for when
//! messaging. Read from the reminder record and the records Apple keeps beside it.
//!
//! Shapes, all measured on a real account (the "Skykeep Test" reminder made on an iPhone):
//!
//! * `RecurrenceRule` — `Frequency` (0 daily, 1 weekly, 2 monthly, 3 yearly, 4 hourly),
//!   `Interval`, `FirstDayOfTheWeek` 0, `OccurrenceCount` 0, and `EndDate` (a `TIMESTAMP`)
//!   when the repeat ends on a date. A reminder lists its rules' bare ids in
//!   `RecurrenceRuleIDs`.
//! * A place is an `AlarmTrigger` of `Type` `Location`: `Title` ("Alex's Home"),
//!   `Proximity` 1 arriving (2 leaving), `Latitude`, `Longitude`, `Radius`, `LocationUID`.
//! * When Messaging is the reminder's `ContactHandles`: JSON `{"emails":[…],"phones":[…]}`.
//! * Urgent is a small JSON file (`UrgentPresentationAlarmsAsData`):
//!   `{"account":[{"isEnabled":true,…}],"minimumSupportedVersion":…}`. Most reminders on a
//!   phone carry one with `isEnabled` false; only `true` is Urgent.

use crate::cloudkit::{field, ZoneClient};
use crate::Result;
use quince_core::reminders::{MessagingPerson, PlaceAlarm, ReminderMore, ReminderPart, RepeatRule};
use serde_json::Value;

/// The rule fields this app reads. Anything else on a rule is kept and named.
const RULE_FIELDS: [&str; 9] = [
    "DaysOfTheWeek",
    "Frequency",
    "Interval",
    "FirstDayOfTheWeek",
    "OccurrenceCount",
    "EndDate",
    "Imported",
    "Deleted",
    "Reminder",
];

/// Apple's frequencies, highest first in Apple's numbering: 0 daily … 4 hourly.
pub const MOST_FREQUENCY: i64 = 4;

/// A record worth keeping beside its reminder: a repeat rule, or a trigger that is a place
/// rather than a moment (moments are the alarm tables' business).
#[must_use]
pub fn part_from_record(record: &Value) -> Option<ReminderPart> {
    let kind = field::record_type(record)?;
    let keep = match kind.as_str() {
        "RecurrenceRule" => true,
        "AlarmTrigger" => field::string(record, "Type").is_some_and(|t| t != "Date"),
        _ => false,
    };
    if !keep || field::flag(record, "Deleted") {
        return None;
    }
    Some(ReminderPart {
        name: field::record_name(record)?,
        reminder_guid: field::reference(record, "Reminder"),
        kind,
        raw: serde_json::to_string(record).ok()?,
    })
}

/// A rule record as the phone's Repeat and End Repeat rows read it.
#[must_use]
pub fn rule_from_record(record: &Value) -> Option<RepeatRule> {
    let frequency = field::int(record, "Frequency")?;
    let fields = record.get("fields").and_then(Value::as_object)?;
    // Weekdays, Weekends and days chosen in Custom. A week of the month ("the first Monday")
    // is not read here, so it is named among the fields kept as they are.
    let days = field::string(record, "DaysOfTheWeek")
        .map(|b| quince_core::reminders::days_of_week_from(&b));
    let mut other: Vec<String> = fields
        .iter()
        .filter(|(name, v)| {
            !RULE_FIELDS.contains(&name.as_str())
                && !v.get("value").is_none_or(|x| {
                    x.is_null() || x.as_array().is_some_and(Vec::is_empty) || x.as_str() == Some("")
                })
        })
        .map(|(name, _)| name.clone())
        .collect();
    if matches!(days, Some(None)) {
        other.push("DaysOfTheWeek".into());
    }
    other.sort();
    Some(RepeatRule {
        days_of_week: days.flatten().unwrap_or_default(),
        frequency,
        interval: field::int(record, "Interval").unwrap_or(1).max(1),
        end_at: field::timestamp_seconds(record, "EndDate"),
        occurrence_count: field::int(record, "OccurrenceCount").unwrap_or(0),
        other,
    })
}

/// A place trigger as the phone's Location row reads it.
#[must_use]
pub fn place_from_record(record: &Value) -> Option<PlaceAlarm> {
    let kind = field::string(record, "Type")?;
    if kind == "Date" {
        return None;
    }
    Some(PlaceAlarm {
        title: field::string(record, "Title").filter(|t| !t.trim().is_empty()),
        proximity: match field::int(record, "Proximity") {
            Some(1) => Some("arriving".into()),
            Some(2) => Some("leaving".into()),
            _ => None,
        },
        kind,
    })
}

/// The email addresses and numbers in a reminder's `ContactHandles`, emails first.
#[must_use]
pub fn contact_handles(reminder: &Value) -> Vec<String> {
    let Some(raw) = field::string(reminder, "ContactHandles") else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    ["emails", "phones"]
        .iter()
        .filter_map(|k| parsed.get(*k).and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Whether an Urgent file says the alarm is on. A file that does not read says nothing.
#[must_use]
pub fn urgent_from_file(bytes: &[u8]) -> Option<bool> {
    let parsed: Value = serde_json::from_slice(bytes).ok()?;
    let accounts = parsed.get("account")?.as_array()?;
    Some(
        accounts
            .iter()
            .any(|a| a.get("isEnabled").and_then(Value::as_bool) == Some(true)),
    )
}

/// Everything the Details screen shows beyond tags, a link and early alerts, from what the
/// library holds. Urgent is left unknown: it lives in a file, read by [`urgent_cloudkit`].
#[must_use]
pub fn more_from(
    guid: &str,
    raw: Option<&str>,
    timezone: Option<&str>,
    parts: &[ReminderPart],
) -> ReminderMore {
    let record: Option<Value> = raw.and_then(|r| serde_json::from_str(r).ok());
    let parsed: Vec<Value> = parts
        .iter()
        .filter_map(|p| serde_json::from_str(&p.raw).ok())
        .collect();
    // The rule the reminder names now; a rule left behind by an older repeat is not it.
    let named: Vec<String> = record
        .as_ref()
        .and_then(|r| r.pointer("/fields/RecurrenceRuleIDs/value"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let repeat = parsed
        .iter()
        .filter(|p| field::record_type(p).as_deref() == Some("RecurrenceRule"))
        .find(|p| {
            let name = field::record_name(p).unwrap_or_default();
            let bare = name.rsplit('/').next().unwrap_or(&name);
            named.iter().any(|id| id.eq_ignore_ascii_case(bare))
        })
        .and_then(rule_from_record);
    let location = parsed
        .iter()
        .filter(|p| field::record_type(p).as_deref() == Some("AlarmTrigger"))
        .find_map(place_from_record);
    let handles = record.as_ref().map(contact_handles).unwrap_or_default();
    ReminderMore {
        guid: guid.to_string(),
        repeat,
        time_zone: timezone.map(ToString::to_string).filter(|z| !z.is_empty()),
        urgent: None,
        location,
        messaging: (!handles.is_empty()).then_some(MessagingPerson {
            name: None,
            handles,
        }),
    }
}

/// Urgent files already read, by the checksum Apple keeps beside each: the file changes only
/// when its checksum does, so each is downloaded once a run.
static URGENT_SEEN: std::sync::Mutex<Option<std::collections::HashMap<String, bool>>> =
    std::sync::Mutex::new(None);

fn urgent_checksum(record: &Value) -> Option<String> {
    field::string(record, "UrgentPresentationAlarmsChecksum").filter(|c| !c.is_empty())
}

/// Whether a reminder is Urgent, from what the library holds when its file was read already.
#[must_use]
pub fn urgent_known(raw: Option<&str>) -> Option<bool> {
    let record: Value = serde_json::from_str(raw?).ok()?;
    if record
        .pointer("/fields/UrgentPresentationAlarmsAsData")
        .is_none()
    {
        // No file at all: never made Urgent.
        return Some(false);
    }
    let sum = urgent_checksum(&record)?;
    URGENT_SEEN.lock().ok()?.as_ref()?.get(&sum).copied()
}

/// A picture attached to a reminder, its file as the phone stored it (`FileAsset`, named by
/// `FileName`). Read only: the attachment record is looked up for a fresh link, which lapses
/// within the hour, and the file fetched. `None` when Apple has no such picture.
pub async fn attachment_file(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    attachment: &str,
) -> Result<Option<Vec<u8>>> {
    let client = ZoneClient::new(http, ckdatabasews_root, "com.apple.reminders", params);
    let Some(record) = client.lookup_record("Reminders", attachment).await? else {
        return Ok(None);
    };
    let Some(url) = record
        .pointer("/fields/FileAsset/value/downloadURL")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let name = record
        .pointer("/fields/FileName/value")
        .and_then(Value::as_str)
        .unwrap_or("picture");
    let response = http
        .get(url.replace("${f}", name))
        .send()
        .await
        .map_err(|e| crate::Error::Protocol(e.to_string()))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| crate::Error::Protocol(e.to_string()))?;
    Ok(Some(bytes.to_vec()))
}

/// Read a reminder's Urgent file from Apple. Read only: the record is looked up for a fresh
/// link (the one a sync stored expires within the hour) and the small file fetched.
pub async fn urgent_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
) -> Result<Option<bool>> {
    let client = ZoneClient::new(http, ckdatabasews_root, "com.apple.reminders", params);
    let Some(record) = client.lookup_record("Reminders", guid).await? else {
        return Ok(None);
    };
    let Some(url) = record
        .pointer("/fields/UrgentPresentationAlarmsAsData/value/downloadURL")
        .and_then(Value::as_str)
    else {
        return Ok(Some(false));
    };
    let response = http
        .get(url.replace("${f}", "urgent"))
        .send()
        .await
        .map_err(|e| crate::Error::Protocol(e.to_string()))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| crate::Error::Protocol(e.to_string()))?;
    let urgent = urgent_from_file(&bytes);
    if let (Some(on), Some(sum)) = (urgent, urgent_checksum(&record)) {
        if let Ok(mut seen) = URGENT_SEEN.lock() {
            seen.get_or_insert_with(Default::default).insert(sum, on);
        }
    }
    Ok(urgent)
}

/// Urgent files that could not be read lately, by checksum (or the reminder's name, for a record
/// without one), and when. A failure was never kept: offline, or with Apple out of reach, every
/// read of a list asked for the same forty files again and waited out each one's timeout.
static URGENT_FAILED: std::sync::Mutex<
    Option<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::Mutex::new(None);

/// How long a file that could not be read is left alone before it is asked for again.
pub const URGENT_RETRY: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn urgent_key(record: &Value, guid: &str) -> String {
    urgent_checksum(record).unwrap_or_else(|| guid.to_string())
}

/// Whether `key` failed within `retry` of `now`.
fn failed_within(
    failed: &std::collections::HashMap<String, std::time::Instant>,
    key: &str,
    now: std::time::Instant,
    retry: std::time::Duration,
) -> bool {
    failed
        .get(key)
        .is_some_and(|at| now.saturating_duration_since(*at) < retry)
}

fn note_urgent_failed(key: String, at: std::time::Instant) {
    if let Ok(mut failed) = URGENT_FAILED.lock() {
        let failed = failed.get_or_insert_with(Default::default);
        // Old failures go as new ones come, so the map never outgrows one list's worth.
        failed.retain(|_, when| at.saturating_duration_since(*when) < URGENT_RETRY);
        failed.insert(key, at);
    }
}

fn forget_urgent_failed(key: &str) {
    if let Ok(mut failed) = URGENT_FAILED.lock() {
        if let Some(failed) = failed.as_mut() {
            failed.remove(key);
        }
    }
}

/// Whether this reminder's Urgent file could not be read within the last [`URGENT_RETRY`]:
/// it is not asked for again until then, and its row shows Urgent off, as it does while unknown.
#[must_use]
pub fn urgent_failed_lately(guid: &str, raw: &str) -> bool {
    let Ok(record) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let key = urgent_key(&record, guid);
    URGENT_FAILED.lock().is_ok_and(|failed| {
        failed.as_ref().is_some_and(|failed| {
            failed_within(failed, &key, std::time::Instant::now(), URGENT_RETRY)
        })
    })
}

/// Whether a reminder is Urgent, read from Apple: the file's link from the record as synced
/// first (good for about an hour), and only if that has lapsed, a fresh lookup. Read only.
/// A file that cannot be read is not asked for again for [`URGENT_RETRY`].
pub async fn urgent_for(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    raw: &str,
) -> Option<bool> {
    let record: Value = serde_json::from_str(raw).ok()?;
    let key = urgent_key(&record, guid);
    let urgent = urgent_read(http, ckdatabasews_root, params, guid, &record).await;
    match urgent {
        Some(_) => forget_urgent_failed(&key),
        None => note_urgent_failed(key, std::time::Instant::now()),
    }
    urgent
}

async fn urgent_read(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    record: &Value,
) -> Option<bool> {
    let url = record
        .pointer("/fields/UrgentPresentationAlarmsAsData/value/downloadURL")
        .and_then(Value::as_str)?;
    if let Ok(response) = http.get(url.replace("${f}", "urgent")).send().await {
        if response.status().is_success() {
            if let Ok(bytes) = response.bytes().await {
                let urgent = urgent_from_file(&bytes);
                if let (Some(on), Some(sum)) = (urgent, urgent_checksum(record)) {
                    if let Ok(mut seen) = URGENT_SEEN.lock() {
                        seen.get_or_insert_with(Default::default).insert(sum, on);
                    }
                }
                return urgent;
            }
        }
    }
    urgent_cloudkit(http, ckdatabasews_root, params, guid)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that could not be read is left alone for a while, then asked for again.
    #[test]
    fn a_failed_urgent_read_waits_before_it_is_tried_again() {
        let now = std::time::Instant::now();
        let mut failed = std::collections::HashMap::new();
        assert!(!failed_within(&failed, "sum", now, URGENT_RETRY));
        failed.insert("sum".to_string(), now);
        assert!(failed_within(&failed, "sum", now, URGENT_RETRY));
        assert!(failed_within(
            &failed,
            "sum",
            now + URGENT_RETRY / 2,
            URGENT_RETRY
        ));
        assert!(!failed_within(
            &failed,
            "sum",
            now + URGENT_RETRY,
            URGENT_RETRY
        ));
        assert!(!failed_within(&failed, "other", now, URGENT_RETRY));
    }

    /// Kept by the file's checksum, so a changed file (a new checksum) is read at once.
    #[test]
    fn a_failure_is_kept_by_checksum_and_forgotten_on_success() {
        let raw = r#"{"recordName":"Reminder/F1","fields":{
            "UrgentPresentationAlarmsChecksum":{"value":"sum-failed-test"},
            "UrgentPresentationAlarmsAsData":{"value":{}}}}"#;
        let changed = raw.replace("sum-failed-test", "sum-failed-test-2");
        assert!(!urgent_failed_lately("Reminder/F1", raw));
        note_urgent_failed("sum-failed-test".into(), std::time::Instant::now());
        assert!(urgent_failed_lately("Reminder/F1", raw));
        assert!(!urgent_failed_lately("Reminder/F1", &changed));
        forget_urgent_failed("sum-failed-test");
        assert!(!urgent_failed_lately("Reminder/F1", raw));
    }
    use serde_json::json;

    /// The founder's "Skykeep Test" as the phone wrote it, personal text replaced.
    fn skykeep_test() -> Value {
        json!({
            "recordName": "Reminder/B51EAE34-018C-4F06-B97C-130C02E98799",
            "recordType": "Reminder",
            "recordChangeTag": "jfv",
            "fields": {
                "AlarmIDs": { "type": "STRING_LIST", "value": ["18154128-7CA1-46FA-9AB3-1E597CF2D3DC", "4FB1FED5-72D2-486C-A9F9-4F2123118C4A"] },
                "AllDay": { "type": "NUMBER_INT64", "value": 0 },
                "ContactHandles": { "isEncrypted": true, "type": "STRING", "value": "{\"emails\":[\"someone@example.com\"],\"phones\":[\"+910000000000\"]}" },
                "DueDate": { "type": "TIMESTAMP", "value": 1_792_681_200_000_i64 },
                "Flagged": { "type": "NUMBER_INT64", "value": 1 },
                "HashtagIDs": { "type": "STRING_LIST", "value": ["E805CCFA-D039-4FC5-ABEB-780FA9B9A3B8"] },
                "Priority": { "type": "NUMBER_INT64", "value": 5 },
                "RecurrenceRuleIDs": { "type": "STRING_LIST", "value": ["049EF968-7FAE-4585-94FB-E22D108B537D"] },
                "TimeZone": { "type": "STRING", "value": "America/New_York" },
                "UrgentPresentationAlarmsAsData": { "type": "ASSETID", "value": { "size": 155, "downloadURL": "https://example.invalid/${f}" } },
                "UrgentPresentationAlarmsChecksum": { "isEncrypted": true, "type": "STRING", "value": "ddfd7f9a" }
            }
        })
    }

    fn daily_rule(name: &str) -> Value {
        json!({
            "recordName": format!("RecurrenceRule/{name}"),
            "recordType": "RecurrenceRule",
            "fields": {
                "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                "FirstDayOfTheWeek": { "type": "NUMBER_INT64", "value": 0 },
                "Frequency": { "type": "NUMBER_INT64", "value": 0 },
                "Imported": { "type": "NUMBER_INT64", "value": 0 },
                "Interval": { "type": "NUMBER_INT64", "value": 1 },
                "OccurrenceCount": { "type": "NUMBER_INT64", "value": 0 },
                "Reminder": { "type": "REFERENCE", "value": { "recordName": "Reminder/B51EAE34-018C-4F06-B97C-130C02E98799", "action": "VALIDATE" } }
            }
        })
    }

    fn home_arriving() -> Value {
        json!({
            "recordName": "AlarmTrigger/61D17FA2-277F-41FB-B959-7F5A1C4FDDE9",
            "recordType": "AlarmTrigger",
            "fields": {
                "Alarm": { "type": "REFERENCE", "value": { "recordName": "Alarm/4FB1FED5-72D2-486C-A9F9-4F2123118C4A", "action": "VALIDATE" } },
                "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                "Imported": { "type": "NUMBER_INT64", "value": 0 },
                "Latitude": { "isEncrypted": true, "type": "NUMBER_DOUBLE", "value": 0.0 },
                "LocationUID": { "type": "STRING", "value": "A2ED6D26-D7B3-483D-B1D1-6B3C97F3A638" },
                "Longitude": { "isEncrypted": true, "type": "NUMBER_DOUBLE", "value": 0.0 },
                "Proximity": { "type": "NUMBER_INT64", "value": 1 },
                "Radius": { "type": "NUMBER_DOUBLE", "value": 100.0 },
                "ReferenceFrameString": { "isEncrypted": true, "type": "STRING", "value": "1" },
                "Title": { "isEncrypted": true, "type": "STRING", "value": "Someone\u{2019}s Home" },
                "Type": { "type": "STRING", "value": "Location" }
            }
        })
    }

    fn date_trigger() -> Value {
        json!({
            "recordName": "AlarmTrigger/E76736A9-2B02-486D-AEC8-574702186709",
            "recordType": "AlarmTrigger",
            "fields": {
                "DateComponentsData": { "type": "BYTES", "value": "e30=" },
                "Type": { "type": "STRING", "value": "Date" }
            }
        })
    }

    fn parts() -> Vec<ReminderPart> {
        [
            daily_rule("049EF968-7FAE-4585-94FB-E22D108B537D"),
            home_arriving(),
            date_trigger(),
        ]
        .iter()
        .filter_map(part_from_record)
        .collect()
    }

    #[test]
    fn the_rule_and_the_place_are_kept_and_the_moment_is_not() {
        let kept: Vec<String> = parts().into_iter().map(|p| p.name).collect();
        assert_eq!(
            kept,
            [
                "RecurrenceRule/049EF968-7FAE-4585-94FB-E22D108B537D",
                "AlarmTrigger/61D17FA2-277F-41FB-B959-7F5A1C4FDDE9"
            ]
        );
        assert_eq!(
            parts()[0].reminder_guid.as_deref(),
            Some("Reminder/B51EAE34-018C-4F06-B97C-130C02E98799")
        );
    }

    #[test]
    fn the_founders_reminder_reads_as_the_phone_shows_it() {
        let record = skykeep_test();
        let more = more_from(
            "Reminder/B51EAE34-018C-4F06-B97C-130C02E98799",
            Some(&record.to_string()),
            Some("America/New_York"),
            &parts(),
        );
        let repeat = more.repeat.expect("repeats");
        assert_eq!(
            (repeat.frequency, repeat.interval, repeat.end_at),
            (0, 1, None)
        );
        assert!(repeat.other.is_empty());
        assert_eq!(more.time_zone.as_deref(), Some("America/New_York"));
        let place = more.location.expect("a place");
        assert_eq!(place.title.as_deref(), Some("Someone\u{2019}s Home"));
        assert_eq!(place.proximity.as_deref(), Some("arriving"));
        assert_eq!(
            more.messaging.expect("a person").handles,
            ["someone@example.com", "+910000000000"]
        );
        assert_eq!(more.urgent, None, "Urgent lives in a file, read apart");
    }

    #[test]
    fn a_rule_the_reminder_no_longer_names_is_not_its_repeat() {
        let mut record = skykeep_test();
        record["fields"]["RecurrenceRuleIDs"] = json!({ "type": "EMPTY_LIST", "value": [] });
        let more = more_from(
            "Reminder/B51EAE34-018C-4F06-B97C-130C02E98799",
            Some(&record.to_string()),
            None,
            &parts(),
        );
        assert!(more.repeat.is_none());
    }

    #[test]
    fn an_hourly_rule_with_an_end_date_reads_whole() {
        let mut rule = daily_rule("H");
        rule["fields"]["Frequency"]["value"] = json!(4);
        rule["fields"]["EndDate"] = json!({ "type": "TIMESTAMP", "value": 1_791_557_520_000_i64 });
        let read = rule_from_record(&rule).unwrap();
        assert_eq!((read.frequency, read.end_at), (4, Some(1_791_557_520)));
    }

    #[test]
    fn fields_this_app_does_not_read_are_named_not_lost() {
        let mut rule = daily_rule("W");
        rule["fields"]["Frequency"]["value"] = json!(1);
        rule["fields"]["DaysOfTheWeek"] = json!({ "type": "STRING", "value": "[2,3,4,5,6]" });
        rule["fields"]["SetPositions"] = json!({ "type": "EMPTY_LIST", "value": [] });
        assert_eq!(rule_from_record(&rule).unwrap().other, ["DaysOfTheWeek"]);
    }

    #[test]
    fn leaving_is_read_and_a_place_with_no_side_says_none() {
        let mut t = home_arriving();
        t["fields"]["Proximity"]["value"] = json!(2);
        assert_eq!(
            place_from_record(&t).unwrap().proximity.as_deref(),
            Some("leaving")
        );
        t["fields"]["Proximity"]["value"] = json!(0);
        assert_eq!(place_from_record(&t).unwrap().proximity, None);
    }

    /// Through a real pass: the list, the reminder, its rule, its two alarms and their
    /// triggers, walked as Apple sends them, then read back as the Details screen asks.
    #[test]
    fn a_sync_pass_keeps_what_the_details_screen_needs() {
        let list = json!({
            "recordName": "List/FED5023E-668F-4670-89CB-2B7765A41F53",
            "recordType": "List",
            "fields": { "Name": { "type": "STRING", "value": "To-Do" } }
        });
        let mut reminder = skykeep_test();
        reminder["fields"]["List"] = json!({ "type": "REFERENCE", "value": { "recordName": "List/FED5023E-668F-4670-89CB-2B7765A41F53", "action": "VALIDATE" } });
        reminder["fields"]["TitleDocument"] = json!({ "type": "ENCRYPTED_BYTES", "value": crate::ckdocument::text_to_base64("Test") });
        let alarm = |id: &str, trigger: &str| {
            json!({
                "recordName": format!("Alarm/{id}"),
                "recordType": "Alarm",
                "fields": {
                    "Reminder": { "type": "REFERENCE", "value": { "recordName": "Reminder/B51EAE34-018C-4F06-B97C-130C02E98799", "action": "VALIDATE" } },
                    "TriggerID": { "type": "STRING", "value": trigger }
                }
            })
        };
        let mut library = quince_core::Library::in_memory().unwrap();
        crate::reminders::apply_cloudkit(
            &mut library,
            crate::reminders::RemindersFetch {
                memberships: std::collections::HashMap::new(),
                records: vec![
                    list,
                    reminder,
                    daily_rule("049EF968-7FAE-4585-94FB-E22D108B537D"),
                    alarm(
                        "4FB1FED5-72D2-486C-A9F9-4F2123118C4A",
                        "61D17FA2-277F-41FB-B959-7F5A1C4FDDE9",
                    ),
                    alarm(
                        "18154128-7CA1-46FA-9AB3-1E597CF2D3DC",
                        "E76736A9-2B02-486D-AEC8-574702186709",
                    ),
                    home_arriving(),
                    date_trigger(),
                ],
                token: Some("t".into()),
                incremental: false,
            },
        )
        .unwrap();
        let guid = "Reminder/B51EAE34-018C-4F06-B97C-130C02E98799";
        let stored = library.reminder(guid).unwrap().unwrap();
        assert_eq!(
            (stored.recur_frequency, stored.recur_interval),
            (Some(0), Some(1))
        );
        assert_eq!((stored.priority, stored.flagged), (5, true));
        let parts = library.reminder_parts(guid).unwrap();
        let raw = library.reminder_raw(guid).unwrap();
        let more = more_from(guid, raw.as_deref(), stored.timezone.as_deref(), &parts);
        assert_eq!(more.repeat.map(|r| (r.frequency, r.interval)), Some((0, 1)));
        assert_eq!(more.time_zone.as_deref(), Some("America/New_York"));
        assert_eq!(
            more.location.and_then(|l| l.proximity).as_deref(),
            Some("arriving")
        );
        assert!(more.messaging.is_some());
    }

    // ---- The founder's samples, made on his iPhone for this: one per Repeat option, early
    // reminders and Urgent. Ids and dates as the phone wrote them; no personal text.

    /// "Repeating 1" and "Repeating 3": Weekdays — the phone wrote the keys in either order.
    const WEEKDAYS_A: &str = "W3siZGF5T2ZUaGVXZWVrIjoyLCJ3ZWVrTnVtYmVyIjowfSx7ImRheU9mVGhlV2VlayI6Mywid2Vla051bWJlciI6MH0seyJkYXlPZlRoZVdlZWsiOjQsIndlZWtOdW1iZXIiOjB9LHsiZGF5T2ZUaGVXZWVrIjo1LCJ3ZWVrTnVtYmVyIjowfSx7ImRheU9mVGhlV2VlayI6Niwid2Vla051bWJlciI6MH1d";
    const WEEKDAYS_B: &str = "W3sid2Vla051bWJlciI6MCwiZGF5T2ZUaGVXZWVrIjoyfSx7IndlZWtOdW1iZXIiOjAsImRheU9mVGhlV2VlayI6M30seyJ3ZWVrTnVtYmVyIjowLCJkYXlPZlRoZVdlZWsiOjR9LHsid2Vla051bWJlciI6MCwiZGF5T2ZUaGVXZWVrIjo1fSx7IndlZWtOdW1iZXIiOjAsImRheU9mVGhlV2VlayI6Nn1d";
    /// "Rep 4": Weekends.
    const WEEKENDS: &str = "W3siZGF5T2ZUaGVXZWVrIjoxLCJ3ZWVrTnVtYmVyIjowfSx7ImRheU9mVGhlV2VlayI6Nywid2Vla051bWJlciI6MH1d";

    fn sample_rule(
        frequency: i64,
        interval: i64,
        days: Option<&str>,
        end_ms: Option<i64>,
    ) -> Value {
        let mut r = daily_rule("SAMPLE");
        r["fields"]["Frequency"]["value"] = json!(frequency);
        r["fields"]["Interval"]["value"] = json!(interval);
        if let Some(d) = days {
            r["fields"]["DaysOfTheWeek"] = json!({ "type": "BYTES", "value": d });
        }
        if let Some(e) = end_ms {
            r["fields"]["EndDate"] = json!({ "type": "TIMESTAMP", "value": e });
        }
        r
    }

    #[test]
    fn every_repeat_the_phone_wrote_reads_as_its_menu_shows_it() {
        // (sample, frequency, interval, days, end) — Repeating 1, a, 3; Rep 4, 5, 6, 7, 8.
        let cases = [
            (
                sample_rule(1, 1, Some(WEEKDAYS_A), Some(1_792_656_031_000)),
                1,
                1,
                vec![2, 3, 4, 5, 6],
                Some(1_792_656_031),
            ),
            (sample_rule(4, 1, None, None), 4, 1, vec![], None),
            (
                sample_rule(1, 1, Some(WEEKDAYS_B), None),
                1,
                1,
                vec![2, 3, 4, 5, 6],
                None,
            ),
            (
                sample_rule(1, 1, Some(WEEKENDS), None),
                1,
                1,
                vec![1, 7],
                None,
            ),
            (sample_rule(1, 1, None, None), 1, 1, vec![], None),
            (sample_rule(1, 2, None, None), 1, 2, vec![], None),
            (sample_rule(2, 1, None, None), 2, 1, vec![], None),
            (
                sample_rule(2, 3, None, Some(1_792_656_137_000)),
                2,
                3,
                vec![],
                Some(1_792_656_137),
            ),
        ];
        for (record, frequency, interval, days, end) in cases {
            let read = rule_from_record(&record).unwrap();
            assert_eq!(
                (
                    read.frequency,
                    read.interval,
                    read.days_of_week,
                    read.end_at
                ),
                (frequency, interval, days, end)
            );
            assert!(
                read.other.is_empty(),
                "nothing on the phone's rules is left unread"
            );
        }
    }

    /// Written back, Weekdays and Weekends are the phone's own bytes.
    #[test]
    fn weekdays_and_weekends_are_written_as_the_phone_wrote_them() {
        assert_eq!(
            quince_core::reminders::days_of_week_to(&[2, 3, 4, 5, 6]),
            WEEKDAYS_A
        );
        assert_eq!(quince_core::reminders::days_of_week_to(&[7, 1]), WEEKENDS);
        // Monday, Wednesday, Friday, chosen in Custom: the same form.
        let mwf = quince_core::reminders::days_of_week_to(&[2, 4, 6]);
        assert_eq!(
            quince_core::reminders::days_of_week_from(&mwf),
            Some(vec![2, 4, 6])
        );
    }

    /// A week of the month ("the first Monday") is not a weekday choice: kept, and named.
    #[test]
    fn a_week_of_the_month_is_kept_not_misread() {
        use base64::Engine;
        let first_monday = base64::engine::general_purpose::STANDARD
            .encode(r#"[{"dayOfTheWeek":2,"weekNumber":1}]"#);
        let read = rule_from_record(&sample_rule(2, 1, Some(&first_monday), None)).unwrap();
        assert!(read.days_of_week.is_empty());
        assert_eq!(read.other, ["DaysOfTheWeek"]);
    }

    /// The early reminders the phone wrote ("Repeating 1": 1 day before; "Rep 7": 2 days
    /// before), read by the details reader as count and unit (2 = days).
    #[test]
    fn the_phones_early_reminders_read_as_days_before() {
        use base64::Engine;
        let blob = |count: i64| {
            base64::engine::general_purpose::STANDARD.encode(format!(
                r#"{{"reminderIdentifier":"C7191213-7FE3-47D2-A60A-FB4C1BF98EF6","dueDateDeltaAlerts":[{{"minimumSupportedAppVersion":0,"identifier":"20813BA8-FF88-4487-94F0-4205C37F5A90","dueDateDeltaCount":{count},"creationDate":814348968.97073495,"dueDateDeltaUnit":2}}],"accountIdentifier":"6D050766-549E-4C2C-93F1-53CE17E5798F","minimumSupportedVersion":20230430}}"#
            ))
        };
        for count in [-1, -2] {
            let record = json!({ "fields": { "DueDateDeltaAlertsData": { "type": "BYTES", "value": blob(count) } } });
            let (_, alerts) = crate::reminders::details::early_alerts_from_record(&record);
            assert_eq!((alerts[0].count, alerts[0].unit), (-count, 2));
        }
    }

    /// "Rep 5" and "Rep 8" are Urgent; the other six are not.
    #[test]
    fn the_phones_urgent_files_read_on_and_off() {
        let on = br#"{"minimumSupportedVersion":20251103,"account":[{"modifiedOn":814348900.87413299,"personID":"P","isEnabled":true}]}"#;
        let off = br#"{"account":[{"modifiedOn":814348852.927158,"personID":"P","isEnabled":false}],"minimumSupportedVersion":20251103}"#;
        assert_eq!(urgent_from_file(on), Some(true));
        assert_eq!(urgent_from_file(off), Some(false));
    }

    #[test]
    fn urgent_is_on_only_when_the_file_says_so() {
        let on = br#"{"minimumSupportedVersion":20251103,"account":[{"personID":"x","modifiedOn":814338293.79,"isEnabled":true}]}"#;
        let off = br#"{"account":[{"isEnabled":false,"modifiedOn":788163438.85,"personID":"x"}],"minimumSupportedVersion":20251103}"#;
        assert_eq!(urgent_from_file(on), Some(true));
        assert_eq!(urgent_from_file(off), Some(false));
        assert_eq!(urgent_from_file(b"not json"), None);
    }

    #[test]
    fn a_reminder_with_no_urgent_file_was_never_urgent() {
        let mut record = skykeep_test();
        record["fields"]
            .as_object_mut()
            .unwrap()
            .remove("UrgentPresentationAlarmsAsData");
        assert_eq!(urgent_known(Some(&record.to_string())), Some(false));
        // With a file not yet read, it is not known.
        assert_eq!(urgent_known(Some(&skykeep_test().to_string())), None);
    }
}
