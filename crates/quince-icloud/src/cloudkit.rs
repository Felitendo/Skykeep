//! Reading a CloudKit private-database zone.
//!
//! Three things about this API bite immediately, and all three fail quietly:
//!
//!   * `records/query` is refused for types Apple has not marked indexable — `Reminder` is
//!     one — so `changes/zone` is the way in. It is also the incremental path.
//!   * `desiredKeys: []` asks for *no* fields rather than all of them, which returns records
//!     that look empty.
//!   * Responses page. Ignoring `moreComing` silently truncates someone's data.

use crate::{Error, Result};
use serde_json::{json, Value};

/// Records fetched in one request. Apple's own client uses a few hundred.
const PAGE_SIZE: u64 = 200;

/// A ceiling on paging, so a server that always says `moreComing` cannot spin forever.
const MAX_PAGES: usize = 200;

pub struct ZoneClient<'a> {
    http: &'a reqwest::Client,
    base: String,
    container: String,
    params: Vec<(String, String)>,
}

/// One page of zone changes.
pub struct Page {
    pub records: Vec<Value>,
    pub sync_token: Option<String>,
    pub more_coming: bool,
}

impl<'a> ZoneClient<'a> {
    pub fn new(
        http: &'a reqwest::Client,
        ckdatabasews_root: &str,
        container: &str,
        params: Vec<(String, String)>,
    ) -> Self {
        Self {
            http,
            base: ckdatabasews_root.trim_end_matches('/').to_string(),
            container: container.to_string(),
            params,
        }
    }

    fn query(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();
        q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
        q.push(("ckjsVersion".into(), "2.6.1".into()));
        q
    }

    /// One page of changes for `zone`.
    pub async fn changes(&self, zone: &str, sync_token: Option<&str>) -> Result<Page> {
        let mut zone_body = json!({
            "zoneID": { "zoneName": zone, "zoneType": "REGULAR_CUSTOM_ZONE" },
            "maxChanges": PAGE_SIZE,
        });
        if let Some(token) = sync_token {
            zone_body["syncToken"] = Value::String(token.to_string());
        }
        // Note: no `desiredKeys`. Sending an empty list asks Apple for no fields at all.

        let url = format!(
            "{}/database/1/{}/production/private/changes/zone",
            self.base, self.container
        );
        let response = self
            .http
            .post(url)
            .query(&self.query())
            .json(&json!({ "zones": [zone_body] }))
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit zone {zone} returned {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }

        let body: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("CloudKit returned invalid JSON: {e}")))?;
        let zone_result = body
            .get("zones")
            .and_then(Value::as_array)
            .and_then(|zones| zones.first())
            .ok_or_else(|| Error::Protocol("CloudKit returned no zone".into()))?;

        if let Some(reason) = zone_result.get("reason").and_then(Value::as_str) {
            return Err(Error::Protocol(format!(
                "CloudKit refused zone {zone}: {reason}"
            )));
        }

        Ok(Page {
            records: zone_result
                .get("records")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            sync_token: zone_result
                .get("syncToken")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            more_coming: zone_result
                .get("moreComing")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Every record in a zone, following `moreComing`.
    ///
    /// Returns the records and the final sync token, which a later call can resume from so a
    /// refresh fetches only what changed.
    pub async fn all_records(
        &self,
        zone: &str,
        since: Option<&str>,
    ) -> Result<(Vec<Value>, Option<String>)> {
        let mut records = Vec::new();
        let mut token = since.map(ToString::to_string);

        for _ in 0..MAX_PAGES {
            let page = self.changes(zone, token.as_deref()).await?;
            records.extend(page.records);
            token = page.sync_token;
            if !page.more_coming {
                break;
            }
            // A page that says "more coming" but hands back no cursor would loop forever.
            if token.is_none() {
                break;
            }
        }
        Ok((records, token))
    }
}

/// Field helpers. CloudKit wraps every value as `{"type": …, "value": …}`.
pub mod field {
    use serde_json::Value;

    #[must_use]
    pub fn string(record: &Value, name: &str) -> Option<String> {
        record
            .pointer(&format!("/fields/{name}/value"))?
            .as_str()
            .map(ToString::to_string)
    }

    #[must_use]
    pub fn int(record: &Value, name: &str) -> Option<i64> {
        record.pointer(&format!("/fields/{name}/value"))?.as_i64()
    }

    #[must_use]
    pub fn flag(record: &Value, name: &str) -> bool {
        int(record, name).unwrap_or(0) != 0
    }

    /// CloudKit timestamps are milliseconds; the library stores seconds.
    #[must_use]
    pub fn timestamp_seconds(record: &Value, name: &str) -> Option<i64> {
        int(record, name).map(|ms| ms / 1000)
    }

    /// The target of a REFERENCE field, e.g. `List/A233F124-…`.
    #[must_use]
    pub fn reference(record: &Value, name: &str) -> Option<String> {
        record
            .pointer(&format!("/fields/{name}/value/recordName"))?
            .as_str()
            .map(ToString::to_string)
    }

    /// Text out of a document field such as `TitleDocument`.
    #[must_use]
    pub fn document_text(record: &Value, name: &str) -> Option<String> {
        crate::ckdocument::text_from_base64(&string(record, name)?)
    }

    #[must_use]
    pub fn record_name(record: &Value) -> Option<String> {
        record.get("recordName")?.as_str().map(ToString::to_string)
    }

    #[must_use]
    pub fn record_type(record: &Value) -> Option<String> {
        record.get("recordType")?.as_str().map(ToString::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn record(name: &str, kind: &str) -> Value {
        json!({
            "recordName": name,
            "recordType": kind,
            "fields": { "Completed": { "type": "NUMBER_INT64", "value": 0 } }
        })
    }

    /// Ignoring moreComing silently truncates someone's data, so paging is the thing to test.
    #[tokio::test]
    async fn paging_follows_more_coming_to_the_end() {
        let server = MockServer::start().await;
        let pages = vec![
            json!({ "zones": [{ "records": [record("a", "Reminder")],
                                "syncToken": "t1", "moreComing": true }] }),
            json!({ "zones": [{ "records": [record("b", "Reminder")],
                                "syncToken": "t2", "moreComing": true }] }),
            json!({ "zones": [{ "records": [record("c", "Reminder")],
                                "syncToken": "t3", "moreComing": false }] }),
        ];
        for page in pages {
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone$"))
                .respond_with(ResponseTemplate::new(200).set_body_json(page))
                .up_to_n_times(1)
                .mount(&server)
                .await;
        }

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let (records, token) = client.all_records("Reminders", None).await.unwrap();

        assert_eq!(records.len(), 3, "every page must be collected");
        assert_eq!(
            token.as_deref(),
            Some("t3"),
            "the final cursor is what resumes later"
        );
    }

    /// A server that always says more-coming without a cursor must not spin.
    #[tokio::test]
    async fn paging_stops_when_there_is_no_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{ "records": [record("a", "Reminder")], "moreComing": true }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let (records, _) = client.all_records("Reminders", None).await.unwrap();
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_zone_is_an_error_not_an_empty_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{ "reason": "Zone does not exist" }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        assert!(client.all_records("Missing", None).await.is_err());
    }

    #[test]
    fn field_helpers_read_cloudkits_wrapping() {
        let rec = json!({
            "recordName": "Reminder/abc",
            "recordType": "Reminder",
            "fields": {
                "Completed": { "type": "NUMBER_INT64", "value": 1 },
                "CreationDate": { "type": "TIMESTAMP", "value": 1_753_808_338_928_i64 },
                "List": { "type": "REFERENCE", "value": { "recordName": "List/xyz" } }
            }
        });
        assert_eq!(field::record_name(&rec).as_deref(), Some("Reminder/abc"));
        assert_eq!(field::record_type(&rec).as_deref(), Some("Reminder"));
        assert!(field::flag(&rec, "Completed"));
        assert!(
            !field::flag(&rec, "Missing"),
            "an absent flag reads as false"
        );
        assert_eq!(field::reference(&rec, "List").as_deref(), Some("List/xyz"));
        assert_eq!(
            field::timestamp_seconds(&rec, "CreationDate"),
            Some(1_753_808_338),
            "milliseconds must become seconds"
        );
    }
}
