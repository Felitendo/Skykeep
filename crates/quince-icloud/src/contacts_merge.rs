//! Merging cards that are one person, on Apple's own cards rather than this app's copy of
//! them: whatever a card holds — the fields this app shows and any it does not — reaches the
//! card that is kept. Every step is safe to take twice, so a merge cut off part way (offline,
//! a restart) is simply sent again:
//!
//! 1. the kept card is written with everything from the others it lacks — written again, it
//!    already has it all, and nothing is added twice;
//! 2. it joins every list the others were in — a list holds a person once;
//! 3. the others are deleted — a card already gone is done.

use super::{
    address, card_written, contact_from, entry_label, entry_value, parse_lists, postal_from,
    startup, string, write, ContactList, Ledger, Tokens, METHOD_KEYS,
};
use crate::{Error, Result};
use quince_core::contacts::merge::{joined_notes, method_key};
use quince_core::contacts::{Card, ContactMethod, PostalAddress};
use serde_json::{Map, Value};

/// The address book as one read found it: every card as Apple holds it, its lists, and the
/// tokens a write needs.
pub struct Book {
    body: Value,
    pub tokens: Tokens,
}

impl Book {
    /// Apple's card for one person, if it still has one.
    #[must_use]
    pub fn card(&self, guid: &str) -> Option<Value> {
        self.body
            .get("contacts")
            .and_then(Value::as_array)?
            .iter()
            .find(|c| {
                c.get("contactId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.eq_ignore_ascii_case(guid))
            })
            .cloned()
    }

    /// Which card is this person's own.
    #[must_use]
    pub fn me(&self) -> Option<String> {
        super::me_card(&self.body)
    }

    #[must_use]
    pub fn lists(&self) -> Vec<ContactList> {
        parse_lists(&self.body)
    }
}

/// Read the book, with its tokens.
///
/// # Errors
/// When Apple cannot be reached or refuses.
pub async fn read_book(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Book> {
    let body = startup(http, root, params).await?;
    let token = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
    };
    let tokens = Tokens {
        sync: token("syncToken")?,
        pref: token("prefToken")?,
    };
    Ok(Book { body, tokens })
}

/// A card as this app last saw it, in Apple's shape — for a card merged here and gone from
/// Apple before the merge was sent, so what it held still reaches the kept card.
#[must_use]
pub fn snapshot(card: &Card, addresses: &[PostalAddress]) -> Value {
    let mut v = card_written(&card.contact, Some(&card.details), None, None, None);
    if let Some(o) = v.as_object_mut() {
        o.insert(
            "streetAddresses".into(),
            Value::Array(addresses.iter().map(address::entry).collect()),
        );
        o.remove("photo");
    }
    v
}

/// Keys that are the card's own bookkeeping, never carried from one card to another.
const OWN: [&str; 11] = [
    "contactId",
    "etag",
    "normalized",
    "dateCreated",
    "dateModified",
    "photo",
    "isCompany",
    "order",
    "whitelisted",
    "isGuardianApproved",
    "notes",
];

/// What makes two entries of one of Apple's lists the same entry.
fn entry_key(key: &str, entry: &Value) -> String {
    if key == "streetAddresses" {
        if let Some(a) = postal_from(entry) {
            let text = quince_core::fold::fold(&a.formatted());
            return format!(
                "address:{}",
                text.split_whitespace().collect::<Vec<_>>().join(" ")
            );
        }
    }
    if let Some((_, kind)) = METHOD_KEYS.iter().find(|(k, _)| *k == key) {
        if let Some(value) = entry_value(kind, entry) {
            return method_key(&ContactMethod {
                kind: (*kind).into(),
                label: entry_label(kind, entry),
                value,
            });
        }
    }
    format!("{key}:{entry}")
}

fn has_photo(card: &Value) -> bool {
    card.pointer("/photo/url")
        .and_then(Value::as_str)
        .is_some_and(|u| !u.is_empty())
}

/// The kept card with everything the others hold that it lacks: a name, a company, a date or
/// any other value it has none of; every entry of every list (numbers, emails, addresses,
/// dates, people, profiles and any list this app does not know), each once; notes from each,
/// joined. Answers the card, and the first other card's picture when the kept card has none —
/// to be sent as an image of its own, since its link goes when that card is deleted.
#[must_use]
pub fn union(kept: &Value, others: &[Value]) -> (Value, Option<Value>) {
    let mut out = kept.clone();
    let mut photo = None;
    let Some(o) = out.as_object_mut() else {
        return (out, None);
    };
    for other in others {
        let Some(theirs) = other.as_object() else {
            continue;
        };
        for (key, value) in theirs {
            if OWN.contains(&key.as_str()) {
                continue;
            }
            match value {
                Value::String(text) if !text.trim().is_empty() => {
                    let empty = o
                        .get(key)
                        .and_then(Value::as_str)
                        .is_none_or(|v| v.trim().is_empty());
                    if empty && !o.get(key).is_some_and(|v| !v.is_string() && !v.is_null()) {
                        o.insert(key.clone(), value.clone());
                    }
                }
                Value::Array(entries) => {
                    let mut list = o
                        .get(key)
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut seen: Vec<String> = list.iter().map(|e| entry_key(key, e)).collect();
                    let mut grew = false;
                    for e in entries {
                        let k = entry_key(key, e);
                        if !seen.contains(&k) {
                            seen.push(k);
                            list.push(e.clone());
                            grew = true;
                        }
                    }
                    // Only a list that gained something is written: an empty one where the
                    // kept card had none would be a change of nothing.
                    if grew {
                        o.insert(key.clone(), Value::Array(list));
                    }
                }
                _ => {}
            }
        }
        if photo.is_none() && !has_photo(kept) && has_photo(other) {
            photo = other.get("photo").cloned();
        }
    }
    let notes = joined_notes(
        std::iter::once(kept)
            .chain(others)
            .map(|c| c.get("notes").and_then(Value::as_str)),
    );
    if let Some(n) = notes {
        o.insert("notes".into(), Value::String(n));
    }
    (out, photo)
}

/// Write a card as it stands, against the version it carries.
///
/// # Errors
/// [`Error::Conflict`] when it changed since it was read; otherwise when Apple refuses or
/// cannot be reached.
pub async fn put(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    card: Value,
) -> Result<()> {
    write(http, root, params, ledger, reqwest::Method::PUT, card).await?;
    Ok(())
}

/// Delete a card by its id, against the version Apple holds.
///
/// # Errors
/// When Apple refuses or cannot be reached.
pub async fn delete_card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    card: &Value,
) -> Result<()> {
    let Some(guid) = string(card, "contactId") else {
        return Ok(());
    };
    let mut query = params;
    query.push(("method".into(), "DELETE".into()));
    write(
        http,
        root,
        query,
        ledger,
        reqwest::Method::POST,
        serde_json::json!({ "contactId": guid, "etag": card.get("etag") }),
    )
    .await?;
    Ok(())
}

/// Whether a card read from Apple parses as a person at all — a guard before writing over it.
#[must_use]
pub fn readable(card: &Value) -> bool {
    contact_from(card).is_some()
}

/// The entry Apple's card keeps a picture in, for an image sent whole as a `data:` URL.
#[must_use]
pub fn photo_entry(data_url: &str, crop: Option<(i64, i64, i64, i64)>) -> Value {
    let mut p = Map::new();
    p.insert("url".into(), Value::String(data_url.into()));
    if let Some((x, y, width, height)) = crop {
        p.insert(
            "crop".into(),
            serde_json::json!({ "x": x, "y": y, "width": width, "height": height }),
        );
    }
    Value::Object(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_kept_card_gains_everything_and_repeats_nothing() {
        let kept = json!({
            "contactId": "A", "etag": "ea", "firstName": "Zoya", "lastName": "",
            "phones": [{ "label": "MOBILE", "field": "+91 90000 00001" }],
            "notes": "Met at the conference",
        });
        let other = json!({
            "contactId": "B", "etag": "eb", "firstName": "Zoya", "lastName": "Ahuja",
            "jobTitle": "Architect",
            "phones": [{ "label": "_$!<Mobile>!$_", "field": "090000 00001" },
                       { "label": "WORK", "field": "+91 80 4000 0000" }],
            "streetAddresses": [{ "label": "HOME", "field": { "street": "2 New Rd", "city": "Pune" } }],
            "IMs": [{ "field": { "userName": "zoya", "IMService": "SKYPE" } }],
            "X-SOMETHING-NEW": [{ "a": 1 }],
            "notes": "Prefers WhatsApp",
            "photo": { "url": "https://p/B", "crop": { "x": 0, "y": 0, "width": 10, "height": 10 } },
        });
        let (card, photo) = union(&kept, std::slice::from_ref(&other));
        assert_eq!(card["contactId"], "A");
        assert_eq!(card["etag"], "ea", "against the kept card's version");
        assert_eq!(card["lastName"], "Ahuja");
        assert_eq!(card["jobTitle"], "Architect");
        assert_eq!(
            card["phones"].as_array().unwrap().len(),
            2,
            "the same number once"
        );
        assert_eq!(card["streetAddresses"][0]["field"]["city"], "Pune");
        assert_eq!(card["IMs"][0]["field"]["IMService"], "SKYPE");
        assert_eq!(
            card["X-SOMETHING-NEW"][0]["a"], 1,
            "even what this app does not know"
        );
        assert_eq!(card["notes"], "Met at the conference\n\nPrefers WhatsApp");
        assert!(
            card.get("photo").is_none(),
            "a picture is sent as an image of its own"
        );
        assert_eq!(photo.unwrap()["url"], "https://p/B");

        // Sent again over what the first send wrote: nothing changes.
        let (again, photo) = union(&card, std::slice::from_ref(&other));
        assert_eq!(again, card);
        assert!(photo.is_some(), "still none on the kept card");
        let mut with_photo = card.clone();
        with_photo["photo"] = json!({ "url": "https://p/A" });
        assert!(
            union(&with_photo, &[other]).1.is_none(),
            "a kept picture stays"
        );
    }

    #[test]
    fn a_card_gone_from_apple_still_brings_what_it_held() {
        use quince_core::contacts::{Contact, ContactDetails};
        let card = Card {
            contact: Contact {
                guid: "B".into(),
                first_name: Some("Zoya".into()),
                middle_name: None,
                photo_url: None,
                photo_signature: None,
                photo_crop: None,
                last_name: None,
                company: None,
                is_company: false,
                birthday: Some("1604-03-17".into()),
                notes: None,
                etag: None,
                methods: vec![ContactMethod {
                    kind: "related".into(),
                    label: Some("MOTHER".into()),
                    value: "Amma".into(),
                }],
            },
            details: ContactDetails {
                nickname: Some("Zo".into()),
                ..ContactDetails::default()
            },
        };
        let home = PostalAddress {
            label: Some("HOME".into()),
            city: Some("Pune".into()),
            ..PostalAddress::default()
        };
        let v = snapshot(&card, &[home]);
        assert_eq!(v["birthday"], "1604-03-17");
        assert_eq!(v["nickName"], "Zo");
        assert_eq!(v["relatedNames"][0]["label"], "MOTHER");
        assert_eq!(v["streetAddresses"][0]["field"]["city"], "Pune");
        let (merged, _) = union(&json!({ "contactId": "A", "firstName": "Zoya" }), &[v]);
        assert_eq!(merged["relatedNames"][0]["field"], "Amma");
    }
}
