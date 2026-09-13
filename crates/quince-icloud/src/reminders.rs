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
