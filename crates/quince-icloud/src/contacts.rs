//! Apple's contacts web service.
//!
//! Unlike reminders, this account's contacts still come from the legacy endpoint rather than
//! CloudKit, so `/co/startup` is the whole of it.

use crate::{Error, Result};
use quince_core::contacts::{Contact, ContactMethod};
use serde_json::Value;

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Read one of Apple's method arrays — phones, emails and the rest share a shape.
///
/// Apple names the value `field`, with an optional `label`. Entries with no value are
/// skipped rather than stored as blanks.
fn methods_from(v: &Value, key: &str, kind: &str) -> Vec<ContactMethod> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(ContactMethod {
                        kind: kind.to_string(),
                        label: string(entry, "label"),
                        value: string(entry, "field")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn contact_from(v: &Value) -> Option<Contact> {
    let guid = string(v, "contactId")?;
    let mut methods = methods_from(v, "phones", "phone");
    methods.extend(methods_from(v, "emailAddresses", "email"));
    methods.extend(methods_from(v, "urls", "url"));

    Some(Contact {
        guid,
        first_name: string(v, "firstName"),
        last_name: string(v, "lastName"),
        company: string(v, "companyName"),
        is_company: v.get("isCompany").and_then(Value::as_bool).unwrap_or(false),
        birthday: string(v, "birthday"),
        notes: string(v, "notes"),
        etag: string(v, "etag"),
        methods,
    })
}

#[must_use]
pub fn parse(body: &Value) -> Vec<Contact> {
    body.get("contacts")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(contact_from).collect())
        .unwrap_or_default()
}

/// Fetch the address book and reconcile it into the library.
pub async fn sync(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
) -> Result<usize> {
    let contacts = fetch(http, root, params).await?;
    apply(library, contacts)
}

/// Ask Apple for the address book. Touches no local state, so a refresh can do the network
/// half without holding the library the interface is reading.
pub async fn fetch(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<quince_core::contacts::Contact>> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));

    let url = format!("{}/co/startup", root.trim_end_matches('/'));
    let response = http.get(url).query(&query).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "/co/startup returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }

    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("/co/startup returned invalid JSON: {e}")))?;
    Ok(parse(&body))
}

/// Store a fetched address book. Touches no network.
pub fn apply(
    library: &mut quince_core::Library,
    contacts: Vec<quince_core::contacts::Contact>,
) -> Result<usize> {
    library.put_contacts(&contacts)?;
    library.mark_synced("contacts", None)?;
    Ok(contacts.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_contact_is_read_with_its_methods() {
        let contacts = parse(&json!({ "contacts": [{
            "contactId": "abc",
            "firstName": "Meera",
            "lastName": "Iyer",
            "phones": [{ "field": "+919000000001", "label": "MOBILE" }],
            "emailAddresses": [{ "field": "meera@example.com" }],
            "etag": "e1"
        }]}));
        assert_eq!(contacts.len(), 1);
        let c = &contacts[0];
        assert_eq!(c.display_name(), "Meera Iyer");
        assert_eq!(c.methods.len(), 2);
        assert_eq!(c.methods[0].kind, "phone");
        assert_eq!(c.methods[0].label.as_deref(), Some("MOBILE"));
        assert_eq!(c.methods[1].kind, "email");
    }

    /// Most fields are optional — this account has contacts with only a first name and a
    /// number — so absence must not lose the record.
    #[test]
    fn a_contact_with_almost_nothing_still_parses() {
        let contacts = parse(&json!({ "contacts": [{
            "contactId": "abc",
            "firstName": "Amma",
            "phones": [{ "field": "+919000000001" }]
        }]}));
        assert_eq!(contacts[0].display_name(), "Amma");
        assert!(contacts[0].last_name.is_none());
        assert!(contacts[0].methods[0].label.is_none());
    }

    #[test]
    fn a_contact_without_an_id_is_skipped() {
        assert!(parse(&json!({ "contacts": [{ "firstName": "Nobody" }] })).is_empty());
    }

    /// Blank values must not become empty rows in the detail view.
    #[test]
    fn blank_values_are_dropped() {
        let contacts = parse(&json!({ "contacts": [{
            "contactId": "abc", "firstName": "  ", "lastName": "Iyer",
            "phones": [{ "field": "" }, { "field": "+9111" }]
        }]}));
        assert!(contacts[0].first_name.is_none());
        assert_eq!(contacts[0].methods.len(), 1);
    }

    #[test]
    fn an_empty_body_is_an_empty_address_book() {
        assert!(parse(&json!({})).is_empty());
    }
}
