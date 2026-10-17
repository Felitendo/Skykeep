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

/// Apple's postal address: a structured object, not a string.
///
/// Flattened into the lines a person would write on an envelope, in the order they would
/// write them. Reading it as a string — which is what every other method array holds —
/// simply skipped it, so the one contact with an address showed none.
fn address_from(entry: &Value) -> Option<String> {
    let field = entry.get("field")?;
    if let Some(text) = field.as_str() {
        return Some(text.trim().to_string()).filter(|s| !s.is_empty());
    }

    let part = |key: &str| string(field, key);
    let mut lines: Vec<String> = Vec::new();
    if let Some(street) = part("street") {
        lines.extend(street.lines().map(str::trim).map(ToString::to_string));
    }
    // City, state and postcode share a line, as they do on an envelope.
    let locality: Vec<String> = ["city", "state", "postalCode"]
        .iter()
        .filter_map(|k| part(k))
        .collect();
    if !locality.is_empty() {
        lines.push(locality.join(" "));
    }
    if let Some(country) = part("country") {
        lines.push(country);
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn contact_from(v: &Value) -> Option<Contact> {
    let guid = string(v, "contactId")?;
    let mut methods = methods_from(v, "phones", "phone");
    methods.extend(methods_from(v, "emailAddresses", "email"));
    methods.extend(methods_from(v, "urls", "url"));

    // A postal address nests its parts rather than holding a string.
    methods.extend(
        v.get("streetAddresses")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(ContactMethod {
                            kind: "address".into(),
                            label: string(entry, "label"),
                            value: address_from(entry)?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    // A profile carries both an encoded handle and the readable one a person typed. The
    // readable one is the point; `x-apple:%2B91%2090000` is not an answer to "who is this".
    methods.extend(
        v.get("profiles")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(ContactMethod {
                            kind: "profile".into(),
                            label: string(entry, "label"),
                            value: string(entry, "user").or_else(|| string(entry, "field"))?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    Some(Contact {
        guid,
        first_name: string(v, "firstName"),
        middle_name: string(v, "middleName"),
        // Apple hands out a URL for the picture, a signature that changes with it, and the
        // square its owner chose. Only the URL is always there.
        photo_url: v
            .pointer("/photo/url")
            .and_then(Value::as_str)
            .map(photo_url),
        photo_signature: v
            .pointer("/photo/signature")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        photo_crop: v.pointer("/photo/crop").and_then(crop),
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
/// Apple's own address book service appends a vCard property to some picture URLs, leaving
/// `.../card/<32 hex>X-SHARED-PHOTO-DISPLAY-PREF:ALWAYS_ASK`. That URL answers 404. Four of
/// seventeen pictures on a real account arrive this way, so the leak is trimmed back to the
/// card id — which does answer — rather than those people silently having no picture.
fn photo_url(url: &str) -> String {
    let Some((base, last)) = url.rsplit_once('/') else {
        return url.to_string();
    };
    let id: String = last.chars().take_while(char::is_ascii_hexdigit).collect();
    if id.len() == 32 && id.len() < last.len() {
        format!("{base}/{id}")
    } else {
        url.to_string()
    }
}

/// The chosen square, flattened to the `x,y,width,height` the library keeps.
fn crop(value: &Value) -> Option<String> {
    let at = |key: &str| value.get(key).and_then(Value::as_i64);
    let (x, y, width, height) = (at("x")?, at("y")?, at("width")?, at("height")?);
    (width > 0 && height > 0).then(|| format!("{x},{y},{width},{height}"))
}

pub fn parse(body: &Value) -> Vec<Contact> {
    body.get("contacts")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(contact_from).collect())
        .unwrap_or_default()
}

/// The tokens Apple hands out at startup, which every write has to carry.
///
/// `syncToken` is how the service knows which version of the address book a change is based
/// on; `prefToken` carries the sort order the account is set to. Writing without them is
/// refused.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub sync: String,
    pub pref: String,
}

/// Read the address book along with the tokens a write will need.
///
/// The tokens come from the same response as the contacts, so asking for both at once is
/// one request rather than two, and the tokens are guaranteed to match the data.
/// The whole address book — people and lists — with the tokens a write needs, in one read.
pub async fn fetch_book_with_tokens(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<(AddressBook, Tokens)> {
    let body = startup(http, root, params).await?;
    let token = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
    };
    Ok((
        AddressBook {
            contacts: parse(&body),
            lists: parse_lists(&body),
            me: me_card(&body),
        },
        Tokens {
            sync: token("syncToken")?,
            pref: token("prefToken")?,
        },
    ))
}

pub async fn fetch_with_tokens(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<(Vec<Contact>, Tokens)> {
    let body = startup(http, root, params).await?;
    let token = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
    };
    Ok((
        parse(&body),
        Tokens {
            sync: token("syncToken")?,
            pref: token("prefToken")?,
        },
    ))
}

/// Apple's card, built from ours.
///
/// Every field Apple sends back is sent again on a write, including the ones this app does
/// not model — a card written with only the fields we understand would quietly drop
/// everything else about the person.
fn card_from(contact: &Contact, existing: Option<&Value>) -> Value {
    // Apple's card is the starting point, and Apple is not obliged to send an object; a
    // write that panicked on network input would take the whole window down with it.
    let mut card = existing
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let Some(object) = card.as_object_mut() else {
        return serde_json::json!({});
    };

    object.insert("contactId".into(), Value::String(contact.guid.clone()));
    object.insert("isCompany".into(), Value::Bool(contact.is_company));
    for (key, value) in [
        ("firstName", &contact.first_name),
        ("lastName", &contact.last_name),
        ("companyName", &contact.company),
        ("birthday", &contact.birthday),
        ("notes", &contact.notes),
    ] {
        match value {
            Some(text) if !text.trim().is_empty() => {
                object.insert(key.into(), Value::String(text.trim().to_string()));
            }
            // Clearing a field means sending it empty; removing the key would leave what
            // Apple already has.
            _ => {
                object.insert(key.into(), Value::String(String::new()));
            }
        }
    }

    let of_kind = |kind: &str| -> Value {
        Value::Array(
            contact
                .methods
                .iter()
                .filter(|m| m.kind == kind && !m.value.trim().is_empty())
                .map(|m| {
                    let mut entry = serde_json::Map::new();
                    entry.insert("field".into(), Value::String(m.value.trim().to_string()));
                    if let Some(label) = &m.label {
                        entry.insert("label".into(), Value::String(label.clone()));
                    }
                    Value::Object(entry)
                })
                .collect(),
        )
    };
    object.insert("phones".into(), of_kind("phone"));
    object.insert("emailAddresses".into(), of_kind("email"));
    object.insert("urls".into(), of_kind("url"));

    if let Some(etag) = &contact.etag {
        object.insert("etag".into(), Value::String(etag.clone()));
    }
    card
}

async fn write(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    method: reqwest::Method,
    card: Value,
) -> Result<Value> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    // Fixed rather than the machine's own, and deliberately. Asked with en_US, fr_FR and
    // ja_JP, Apple sent back byte-identical contacts, ordering and section headers — the
    // parameter changes nothing this app reads, and sorting happens locally anyway.
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));

    let url = format!("{}/co/contacts/card", root.trim_end_matches('/'));
    let response = http
        .request(method, url)
        .query(&query)
        .json(&serde_json::json!({ "contacts": [card] }))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    // Apple answers a stale syncToken with a conflict rather than an error, and the fix is
    // to read the address book again — saying so beats a status code.
    if status.as_u16() == 412 || status.as_u16() == 409 {
        return Err(Error::Conflict);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "contacts returned {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Create a contact, and return the id Apple will know it by.
pub async fn create(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    contact: &Contact,
) -> Result<String> {
    // Apple keeps a card that is only a number or an address — "No Name" on the phone — so
    // only a card with nothing in it at all is turned away.
    let blank = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
    if blank(&contact.first_name)
        && blank(&contact.last_name)
        && blank(&contact.company)
        && contact.methods.iter().all(|m| m.value.trim().is_empty())
    {
        return Err(Error::Refused(
            "Add a name, a phone number or an email address first.".into(),
        ));
    }
    // Apple's own client mints the id rather than asking the service for one.
    let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
    let mut fresh = contact.clone();
    fresh.guid.clone_from(&guid);
    fresh.etag = None;

    write(
        http,
        root,
        params,
        tokens,
        reqwest::Method::POST,
        card_from(&fresh, None),
    )
    .await?;
    Ok(guid)
}

/// Save changes to a contact.
///
/// `existing` is Apple's own card for this person, so fields this app does not model are
/// written back untouched instead of being dropped.
pub async fn update(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    contact: &Contact,
    existing: Option<&Value>,
) -> Result<()> {
    write(
        http,
        root,
        params,
        tokens,
        reqwest::Method::PUT,
        card_from(contact, existing),
    )
    .await?;
    Ok(())
}

/// Remove a contact.
pub async fn delete(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    contact: &Contact,
) -> Result<()> {
    // The tag Apple holds now, not the one stored at the last sync: a card edited on the
    // phone since then was refused as a conflict, and the delete failed for no reason a
    // person could see. Gone already is done.
    let Some(current) = card(http, root, params.clone(), &contact.guid).await? else {
        return Ok(());
    };
    let etag = current
        .get("etag")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| contact.etag.clone());
    let mut query = params;
    query.push(("method".into(), "DELETE".into()));
    write(
        http,
        root,
        query,
        tokens,
        reqwest::Method::POST,
        serde_json::json!({
            "contactId": contact.guid,
            "etag": etag,
        }),
    )
    .await?;
    Ok(())
}

/// Apple's card for one person, as it currently stands.
pub async fn card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    guid: &str,
) -> Result<Option<Value>> {
    let body = startup(http, root, params).await?;
    Ok(body
        .get("contacts")
        .and_then(Value::as_array)
        .and_then(|cards| {
            cards
                .iter()
                .find(|c| c.get("contactId").and_then(Value::as_str) == Some(guid))
                .cloned()
        }))
}

/// A Contacts list — Apple's "group" — by name, with the contacts in it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactList {
    pub guid: String,
    pub name: String,
    pub members: Vec<String>,
    pub etag: Option<String>,
}

/// The address book as `/co/startup` gives it: the people, and the lists they are in.
pub struct AddressBook {
    pub contacts: Vec<quince_core::contacts::Contact>,
    pub lists: Vec<ContactList>,
    /// The card that is this person's own, if they have said.
    pub me: Option<String>,
}

/// Where the my-card id is kept between passes.
pub const ME_KEY: &str = "contacts.me";

/// Where the lists are kept between passes. They are few and small, so beside the library's
/// other settings rather than in a table of their own.
pub const LISTS_KEY: &str = "contacts.lists";

/// The lists in a `/co/startup` answer, in the account's own order.
pub fn parse_lists(body: &Value) -> Vec<ContactList> {
    let order: Vec<String> = body
        .pointer("/collections/0/groupsOrder")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut lists: Vec<ContactList> = body
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            Some(ContactList {
                guid: string(g, "groupId")?,
                name: string(g, "name").unwrap_or_else(|| "Untitled List".into()),
                members: g
                    .get("contactIds")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                etag: string(g, "etag"),
            })
        })
        .collect();
    lists.sort_by_key(|l| {
        order
            .iter()
            .position(|o| *o == l.guid)
            .unwrap_or(usize::MAX)
    });
    lists
}

/// The lists the library holds from the last pass.
pub fn stored_lists(library: &quince_core::Library) -> Vec<ContactList> {
    library
        .state(LISTS_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write a list as icloud.com does — the whole of it, every time: a new one is `POST`ed with
/// an id made here, a rename or a change of members is the full list `PUT` back with its
/// etag, a delete names it. Membership travels as the entire set of contact ids.
async fn write_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    method: Option<&str>,
    group: Value,
) -> Result<Value> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));
    if let Some(m) = method {
        query.push(("method".into(), m.into()));
    }
    let url = format!("{}/co/groups/card/", root.trim_end_matches('/'));
    let response = http
        .post(url)
        .query(&query)
        .json(&serde_json::json!({ "groups": [group] }))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if status.as_u16() == 412 || status.as_u16() == 409 {
        return Err(Error::Conflict);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "contacts returned {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    refused_group(&body)?;
    Ok(body)
}

/// Apple answers a refused group write with 200 and the reason in `errors` — a 409 when
/// the etag sent is no longer the list's. Read as success, a lost write would look saved.
fn refused_group(body: &Value) -> Result<()> {
    let Some(error) = body
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    else {
        return Ok(());
    };
    match error.get("errorCode").and_then(Value::as_i64) {
        Some(409) | Some(412) => Err(Error::Conflict),
        code => Err(Error::Protocol(format!(
            "Contacts refused the list ({}): {}",
            code.map_or_else(|| "no code".into(), |c| c.to_string()),
            error
                .get("errorMessage")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
        ))),
    }
}

/// Make a list, with these people in it. Answers with its id.
pub async fn create_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    name: &str,
    members: &[String],
) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Refused("A list needs a name.".into()));
    }
    let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
    write_list(
        http,
        root,
        params,
        tokens,
        None,
        serde_json::json!({ "groupId": guid, "name": name, "contactIds": members }),
    )
    .await?;
    Ok(guid)
}

/// Rename a list, or change who is in it: the whole list, sent back with its etag.
pub async fn update_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    list: &ContactList,
) -> Result<()> {
    write_list(
        http,
        root,
        params,
        tokens,
        Some("PUT"),
        serde_json::json!({ "groupId": list.guid, "etag": list.etag, "name": list.name, "contactIds": list.members }),
    )
    .await?;
    Ok(())
}

/// Delete a list. The people in it stay in Contacts.
pub async fn delete_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    guid: &str,
) -> Result<()> {
    write_list(
        http,
        root,
        params,
        tokens,
        Some("DELETE"),
        serde_json::json!({ "groupId": guid }),
    )
    .await?;
    Ok(())
}

/// A plain-text call to Contacts with the address book's tokens, as icloud.com makes its
/// export, import and my-card calls. Answers with Apple's reply as text.
async fn co_text(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    method: reqwest::Method,
    path: &str,
    body: String,
) -> Result<String> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));
    let url = format!("{}/{path}", root.trim_end_matches('/'));
    let response = http
        .request(method, url)
        .query(&query)
        .header("content-type", "text/plain")
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if status.as_u16() == 412 || status.as_u16() == 409 {
        return Err(Error::Conflict);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "contacts returned {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    Ok(text)
}

/// These people as one vCard file, written by Apple — every field it holds, photos too.
pub async fn export_vcards(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    ids: &[String],
    file_name: &str,
) -> Result<String> {
    let body = serde_json::json!({ "contactIdsToExport": ids, "exportFileName": file_name });
    let text = co_text(
        http,
        root,
        params,
        tokens,
        reqwest::Method::POST,
        "co/export/",
        body.to_string(),
    )
    .await?;
    if !text.trim_start().starts_with("BEGIN:VCARD") {
        return Err(Error::Protocol(
            "Contacts did not send back a vCard.".into(),
        ));
    }
    Ok(text)
}

/// Add the people in a vCard file to Contacts. Answers with the ids Apple gave them, and
/// how many cards it could not read.
pub async fn import_vcards(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    vcard: &str,
) -> Result<(Vec<String>, usize)> {
    // Apple's own ceiling for an import, checked here so the refusal says why.
    if vcard.len() > 10 * 1024 * 1024 {
        return Err(Error::Refused(
            "That file is larger than the 10 MB Contacts takes at once.".into(),
        ));
    }
    if !vcard.contains("BEGIN:VCARD") {
        return Err(Error::Refused("That file holds no contacts.".into()));
    }
    let text = co_text(
        http,
        root,
        params,
        tokens,
        reqwest::Method::POST,
        "co/import/",
        vcard.to_string(),
    )
    .await?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let ids = body
        .get("importedContactIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let failed = body
        .get("errors")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Ok((ids, failed))
}

/// Say which card is this person's own — "My Card" on every device.
pub async fn set_me_card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    id: &str,
) -> Result<()> {
    let body = serde_json::json!({ "meCardId": id });
    co_text(
        http,
        root,
        params,
        tokens,
        reqwest::Method::PUT,
        "co/mecardid",
        body.to_string(),
    )
    .await?;
    Ok(())
}

/// Which card is this person's own, as `/co/startup` says.
pub fn me_card(body: &Value) -> Option<String> {
    body.get("meCardId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Fetch the address book and reconcile it into the library.
pub async fn sync(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
) -> Result<usize> {
    let book = fetch(http, root, params).await?;
    apply(library, book)
}

/// Ask Apple for the address book. Touches no local state, so a refresh can do the network
/// half without holding the library the interface is reading.
pub async fn fetch(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<AddressBook> {
    let body = startup(http, root, params).await?;
    Ok(AddressBook {
        contacts: parse(&body),
        lists: parse_lists(&body),
        me: me_card(&body),
    })
}

/// The whole of `/co/startup`: the address book, the tokens, and everything else Apple
/// sends with it.
async fn startup(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Value> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    // Fixed rather than the machine's own, and deliberately. Asked with en_US, fr_FR and
    // ja_JP, Apple sent back byte-identical contacts, ordering and section headers — the
    // parameter changes nothing this app reads, and sorting happens locally anyway.
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

    serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("/co/startup returned invalid JSON: {e}")))
}

/// Store a fetched address book. Touches no network.
pub fn apply(library: &mut quince_core::Library, book: AddressBook) -> Result<usize> {
    library.put_contacts(&book.contacts)?;
    library.set_state(
        LISTS_KEY,
        &serde_json::to_string(&book.lists).unwrap_or_else(|_| "[]".into()),
    )?;
    library.set_state(ME_KEY, book.me.as_deref().unwrap_or(""))?;
    library.mark_synced("contacts", None)?;
    Ok(book.contacts.len())
}

#[cfg(test)]
mod list_tests {
    #[test]
    fn a_refused_group_write_is_an_error_not_a_save() {
        let busy = serde_json::json!({ "errors": [{ "groupId": "G", "etag": "e", "errorCode": 409, "errorMessage": "Conflict" }] });
        assert!(matches!(
            super::refused_group(&busy),
            Err(crate::Error::Conflict)
        ));
        let other = serde_json::json!({ "errors": [{ "groupId": "G", "errorCode": 400, "errorMessage": "Bad" }] });
        assert!(matches!(
            super::refused_group(&other),
            Err(crate::Error::Protocol(_))
        ));
        assert!(super::refused_group(&serde_json::json!({ "groups": [], "errors": [] })).is_ok());
    }

    use super::*;

    /// Lists come out in the account's own order — `groupsOrder` — with their members.
    #[test]
    fn lists_are_read_in_the_accounts_order() {
        let body = serde_json::json!({
            "groups": [
                { "groupId": "B", "name": "Family", "contactIds": ["c1", "c2"], "etag": "e1" },
                { "groupId": "A", "name": "Work", "contactIds": [], "etag": "e2" }
            ],
            "collections": [{ "groupsOrder": ["A", "B"] }]
        });
        let lists = parse_lists(&body);
        assert_eq!(
            lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["Work", "Family"]
        );
        assert_eq!(lists[1].members, ["c1", "c2"]);
        assert_eq!(lists[1].etag.as_deref(), Some("e1"));
    }
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

#[cfg(test)]
mod write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn person() -> Contact {
        Contact {
            guid: "C1".into(),
            first_name: Some("Ada".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Lovelace".into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: Some("v1".into()),
            methods: vec![
                ContactMethod {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: "+441234567890".into(),
                },
                ContactMethod {
                    kind: "email".into(),
                    label: None,
                    value: "ada@example.com".into(),
                },
            ],
        }
    }

    fn tokens() -> Tokens {
        Tokens {
            sync: "SYNC".into(),
            pref: "PREF".into(),
        }
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        server
    }

    async fn sent(server: &MockServer) -> (Request, Value) {
        let request = server.received_requests().await.unwrap().remove(0);
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        (request, body["contacts"][0].clone())
    }

    /// The one that matters. Apple's card carries fields this app does not model — related
    /// names, social profiles, a photo. Writing only what we understand would drop all of
    /// it, so an edit starts from Apple's own card and changes what it means to change.
    #[tokio::test]
    async fn an_edit_keeps_the_fields_this_app_does_not_model() {
        let server = server().await;
        let apples = json!({
            "contactId": "C1",
            "firstName": "Ada",
            "photo": { "url": "https://example.invalid/ada.jpg" },
            "relatedNames": [{ "label": "sister", "field": "Anne" }],
            "etag": "v1"
        });
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            Some(&apples),
        )
        .await
        .unwrap();

        let (_, card) = sent(&server).await;
        assert_eq!(
            card["photo"]["url"], "https://example.invalid/ada.jpg",
            "a photo must survive an edit"
        );
        assert_eq!(card["relatedNames"][0]["field"], "Anne");
        assert_eq!(card["lastName"], "Lovelace", "and the change still lands");
    }

    /// Every write carries the tokens Apple handed out, or the service refuses it.
    #[tokio::test]
    async fn the_tokens_go_with_every_write() {
        let server = server().await;
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            None,
        )
        .await
        .unwrap();
        let (request, _) = sent(&server).await;
        let query: std::collections::HashMap<_, _> = request.url.query_pairs().collect();
        assert_eq!(query["syncToken"], "SYNC");
        assert_eq!(query["prefToken"], "PREF");
    }

    /// Apple groups methods by kind, each as `field` with an optional `label`.
    #[tokio::test]
    async fn methods_are_grouped_the_way_apple_groups_them() {
        let server = server().await;
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            None,
        )
        .await
        .unwrap();
        let (_, card) = sent(&server).await;
        assert_eq!(card["phones"][0]["field"], "+441234567890");
        assert_eq!(card["phones"][0]["label"], "mobile");
        assert_eq!(card["emailAddresses"][0]["field"], "ada@example.com");
        assert!(
            card["emailAddresses"][0].get("label").is_none(),
            "no label means no label, not an empty one"
        );
        assert_eq!(card["urls"], json!([]));
    }

    /// Clearing a field means sending it empty. Leaving the key out would leave whatever
    /// Apple already has, so the change would appear to do nothing.
    #[tokio::test]
    async fn clearing_a_field_sends_it_empty_rather_than_omitting_it() {
        let server = server().await;
        let mut cleared = person();
        cleared.company = None;
        cleared.notes = None;
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &cleared,
            Some(&json!({ "companyName": "Old Co", "notes": "old note" })),
        )
        .await
        .unwrap();
        let (_, card) = sent(&server).await;
        assert_eq!(card["companyName"], "");
        assert_eq!(card["notes"], "");
    }

    #[tokio::test]
    async fn a_contact_with_no_name_at_all_is_refused_before_anything_is_sent() {
        let server = server().await;
        let nameless = Contact {
            first_name: None,
            last_name: None,
            company: None,
            methods: vec![],
            ..person()
        };
        assert!(create(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &nameless
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// A create gets a fresh id and no etag: an etag would claim to be replacing something.
    #[tokio::test]
    async fn a_create_mints_its_own_id() {
        let server = server().await;
        let guid = create(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
        )
        .await
        .unwrap();
        assert_ne!(guid, "C1", "the id is minted, not carried over");

        let (request, card) = sent(&server).await;
        assert_eq!(request.method, reqwest::Method::POST);
        assert_eq!(card["contactId"], guid);
        assert!(card.get("etag").is_none(), "nothing to replace yet");
    }

    /// The etag sent is the one Apple holds now: a card edited on the phone since the last
    /// sync was refused as a conflict when the stored one went.
    #[tokio::test]
    async fn deleting_says_so_and_carries_apples_current_etag() {
        let server = server().await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [{ "contactId": "C1", "etag": "v2" }]
            })))
            .mount(&server)
            .await;
        delete(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
        )
        .await
        .unwrap();
        let request = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path().ends_with("/co/contacts/card"))
            .expect("the delete was sent");
        let query: std::collections::HashMap<_, _> = request.url.query_pairs().collect();
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(query["method"], "DELETE");
        assert_eq!(body["contacts"][0]["contactId"], "C1");
        assert_eq!(body["contacts"][0]["etag"], "v2");
    }

    /// A card Apple no longer has is already deleted: nothing is sent and nothing fails.
    #[tokio::test]
    async fn deleting_a_card_already_gone_succeeds_quietly() {
        let server = server().await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        delete(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
        )
        .await
        .unwrap();
        assert!(!server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/co/contacts/card")));
    }

    /// A number alone is a contact, as on the phone.
    #[tokio::test]
    async fn a_contact_with_only_a_number_is_created() {
        let server = server().await;
        let number_only = Contact {
            first_name: None,
            last_name: None,
            company: None,
            ..person()
        };
        assert!(
            number_only.methods.iter().any(|m| !m.value.is_empty()),
            "the fixture has a number"
        );
        create(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &number_only,
        )
        .await
        .unwrap();
    }

    /// A stale sync token is recoverable by re-reading, so it is named as a conflict rather
    /// than surfaced as an HTTP status nobody can act on.
    #[tokio::test]
    async fn a_stale_sync_token_reads_as_a_conflict() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .and(method("PUT"))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server)
            .await;
        let err = update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Conflict), "got {err:?}");
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;
    use serde_json::json;

    /// A postal address nests its parts rather than holding a string, so reading it the way
    /// every other method array is read skipped it entirely — the one contact with an
    /// address showed none.
    #[test]
    fn a_postal_address_reads_as_the_lines_it_would_be_written_on() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "firstName": "Asha",
                "streetAddresses": [{
                    "label": "HOME",
                    "field": {
                        "street": "12 Example Street\nSpringfield",
                        "city": "Springfield",
                        "state": "ST",
                        "postalCode": "123456",
                        "country": "India"
                    }
                }]
            }]
        }));
        let address = contacts[0]
            .methods
            .iter()
            .find(|m| m.kind == "address")
            .expect("the address is read");
        assert_eq!(
            address.value,
            "12 Example Street\nSpringfield\nSpringfield ST 123456\nIndia"
        );
        assert_eq!(address.label.as_deref(), Some("HOME"));
    }

    /// Town, county and postcode belong on one line; a missing part must not leave a gap.
    #[test]
    fn a_partial_address_leaves_no_holes() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "streetAddresses": [{ "field": { "city": "Springfield", "country": "India" } }]
            }]
        }));
        assert_eq!(contacts[0].methods[0].value, "Springfield\nIndia");
    }

    /// A profile carries an encoded handle and the readable one a person typed. Showing
    /// `x-apple:%2B91...` answers nothing.
    #[test]
    fn a_profile_shows_what_a_person_typed_not_the_encoding() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "profiles": [{
                    "field": "x-apple:+91%2090000%2000003",
                    "user": "+91 90000 00003"
                }]
            }]
        }));
        assert_eq!(contacts[0].methods[0].kind, "profile");
        assert_eq!(contacts[0].methods[0].value, "+91 90000 00003");
    }

    /// Apple keeps a middle name, and a person with one is not shown under a name that is
    /// only nearly theirs.
    #[test]
    fn a_middle_name_is_part_of_the_name() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "firstName": "Asha",
                "middleName": "Devi",
                "lastName": "Rao"
            }]
        }));
        assert_eq!(contacts[0].display_name(), "Asha Devi Rao");
    }

    /// An address that really is a plain string still reads.
    #[test]
    fn an_address_given_as_a_string_still_reads() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "streetAddresses": [{ "field": "12 Somewhere Lane" }]
            }]
        }));
        assert_eq!(contacts[0].methods[0].value, "12 Somewhere Lane");
    }

    #[test]
    fn an_empty_address_is_left_out_rather_than_shown_blank() {
        let contacts = parse(&json!({
            "contacts": [{ "contactId": "C1", "streetAddresses": [{ "field": {} }] }]
        }));
        assert!(contacts[0].methods.is_empty());
    }
}

#[cfg(test)]
mod photo_tests {
    use super::*;

    /// Apple's own service sends these; four of seventeen pictures on a real account arrive
    /// with a vCard property welded onto the card id, and that URL answers 404.
    #[test]
    fn a_url_apple_mangled_is_trimmed_back_to_the_card_it_names() {
        let base = "https://gateway.icloud.com/contacts/1148/ck/card";
        assert_eq!(
            photo_url(&format!(
                "{base}/095356b02d69e2fbebb51c11fee2459dX-SHARED-PHOTO-DISPLAY-PREF:ALWAYS_ASK"
            )),
            format!("{base}/095356b02d69e2fbebb51c11fee2459d")
        );
        assert_eq!(
            photo_url(&format!(
                "{base}/396d3dd0fc24ceeb66305190d44840edX-SHARED-PHOTO-DISPLAY-PREF:AUTOUPDATE"
            )),
            format!("{base}/396d3dd0fc24ceeb66305190d44840ed")
        );
    }

    /// A URL that is already right must come back untouched, whatever shape it is.
    #[test]
    fn a_sound_url_is_left_exactly_as_it_came() {
        for url in [
            "https://gateway.icloud.com/contacts/1148/ck/card/5dcf9896c11839d3119214c8a000fe0f",
            "https://example.invalid/ada.jpg",
            "https://example.invalid/deadbeef",
            "no-slash-at-all",
        ] {
            assert_eq!(photo_url(url), url, "{url}");
        }
    }

    #[test]
    fn a_crop_is_flattened_and_a_useless_one_is_dropped() {
        let value = serde_json::json!({"x": 251, "y": 1249, "width": 1279, "height": 1279});
        assert_eq!(crop(&value).as_deref(), Some("251,1249,1279,1279"));
        assert_eq!(
            crop(&serde_json::json!({"x": 0, "y": 0, "width": 0, "height": 9})),
            None
        );
        assert_eq!(crop(&serde_json::json!({"x": 0, "y": 0})), None);
        assert_eq!(crop(&Value::Null), None);
    }

    /// The picture fields come off a real payload, not an invented one.
    #[test]
    fn a_shared_picture_parses_with_a_usable_url_and_nothing_else() {
        let body = serde_json::json!({"contacts": [{
            "contactId": "abc",
            "firstName": "Atta",
            "photo": { "url":
                "https://gateway.icloud.com/contacts/1148/ck/card/095356b02d69e2fbebb51c11fee2459dX-SHARED-PHOTO-DISPLAY-PREF:ALWAYS_ASK" }
        }]});
        let parsed = parse(&body);
        assert_eq!(
            parsed[0].photo_url.as_deref(),
            Some(
                "https://gateway.icloud.com/contacts/1148/ck/card/095356b02d69e2fbebb51c11fee2459d"
            )
        );
        assert_eq!(parsed[0].photo_signature, None);
        assert_eq!(parsed[0].photo_crop, None);
    }
}

#[cfg(test)]
mod odd_card_tests {
    use super::*;

    /// Apple's record is the starting point for a write, and nothing obliges it to be an
    /// object. A panic here would take the whole window down on a single odd reply.
    #[test]
    fn a_card_that_is_not_an_object_is_set_aside_rather_than_panicking() {
        let contact = Contact {
            guid: "C1".into(),
            first_name: Some("Ada".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Lovelace".into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: vec![],
        };
        for odd in [
            Value::Null,
            serde_json::json!([1, 2]),
            serde_json::json!("text"),
            serde_json::json!(7),
        ] {
            let card = card_from(&contact, Some(&odd));
            assert_eq!(card["firstName"], "Ada", "{odd} still produces the card");
        }
    }
}
