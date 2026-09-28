//! Contacts in the local library.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContactMethod {
    /// `phone`, `email`, `url`, `address`.
    pub kind: String,
    pub label: Option<String>,
    pub value: String,
}

/// Everything needed to put one person's picture on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactPhoto {
    pub url: String,
    /// Changes when the picture does, so a cached copy knows it has gone stale. Absent on a
    /// picture shared by the person it belongs to, which is then never cached to disk.
    pub signature: Option<String>,
    /// `x,y,width,height` in the stored picture's own pixels.
    pub crop: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub guid: String,
    pub first_name: Option<String>,
    /// Apple keeps one, and four people in a real address book have one.
    #[serde(default)]
    pub middle_name: Option<String>,
    /// Where Apple keeps this person's picture, and the signature that changes when the
    /// picture does. `None` for the great majority of an address book.
    #[serde(default)]
    pub photo_url: Option<String>,
    #[serde(default)]
    pub photo_signature: Option<String>,
    /// The square its owner chose, as `x,y,width,height` in the stored picture's own pixels.
    #[serde(default)]
    pub photo_crop: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub is_company: bool,
    pub birthday: Option<String>,
    pub notes: Option<String>,
    pub etag: Option<String>,
    #[serde(default)]
    pub methods: Vec<ContactMethod>,
}

impl Contact {
    /// What to show, whichever name fields this contact happens to have.
    ///
    /// A contact with only a phone number still needs a label, so the fallback chain ends at
    /// the first method rather than at an empty string.
    #[must_use]
    pub fn display_name(&self) -> String {
        let full = [
            self.first_name.as_deref(),
            self.middle_name.as_deref(),
            self.last_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        if !full.trim().is_empty() {
            return full;
        }
        if let Some(company) = self.company.as_deref().filter(|c| !c.trim().is_empty()) {
            return company.to_string();
        }
        self.methods
            .first()
            .map_or_else(|| "No Name".to_string(), |m| m.value.clone())
    }

    /// Sorted on last name where there is one, which is how Apple orders the list.
    fn sort_key(&self) -> String {
        let key = [self.last_name.as_deref(), self.first_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let key = if key.trim().is_empty() {
            self.display_name()
        } else {
            key
        };
        crate::fold::fold(&key)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    /// Replace the address book, dropping anyone Apple no longer reports.
    pub fn put_contacts(&mut self, contacts: &[Contact]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO contacts (guid, first_name, middle_name, last_name, company, is_company,
                                       sort_name, birthday, notes, photo_url, photo_signature,
                                       photo_crop, etag, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(guid) DO UPDATE SET
                   first_name = excluded.first_name, middle_name = excluded.middle_name,
                   last_name = excluded.last_name,
                   photo_url = excluded.photo_url,
                   photo_signature = excluded.photo_signature,
                   photo_crop = excluded.photo_crop,
                   company = excluded.company, is_company = excluded.is_company,
                   sort_name = excluded.sort_name, birthday = excluded.birthday,
                   notes = excluded.notes, etag = excluded.etag,
                   updated_at = excluded.updated_at",
            )?;
            let mut clear = tx.prepare("DELETE FROM contact_methods WHERE contact_guid = ?1")?;
            let mut add = tx.prepare(
                "INSERT INTO contact_methods (contact_guid, kind, label, value, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            let stamp = now();
            for c in contacts {
                upsert.execute(params![
                    c.guid,
                    c.first_name,
                    c.middle_name,
                    c.last_name,
                    c.company,
                    i64::from(c.is_company),
                    c.sort_key(),
                    c.birthday,
                    c.notes,
                    c.photo_url,
                    c.photo_signature,
                    c.photo_crop,
                    c.etag,
                    stamp
                ])?;
                // Methods are rewritten wholesale: a number removed on the phone must not
                // linger here.
                clear.execute(params![c.guid])?;
                for (i, m) in c.methods.iter().enumerate() {
                    add.execute(params![
                        c.guid,
                        m.kind,
                        m.label,
                        m.value,
                        i64::try_from(i).unwrap_or(0)
                    ])?;
                }
            }

            let keep: Vec<&str> = contacts.iter().map(|c| c.guid.as_str()).collect();
            if keep.is_empty() {
                tx.execute("DELETE FROM contacts", [])?;
            } else {
                let placeholders = vec!["?"; keep.len()].join(",");
                tx.execute(
                    &format!("DELETE FROM contacts WHERE guid NOT IN ({placeholders})"),
                    rusqlite::params_from_iter(keep),
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Where one person's picture lives, without reading the rest of the address book.
    ///
    /// The picture is served per row as a list scrolls, so this stays a single-row lookup
    /// rather than loading everyone and their phone numbers to answer it.
    pub fn contact_photo_source(&self, guid: &str) -> Result<Option<ContactPhoto>> {
        let mut stmt = self.conn.prepare(
            "SELECT photo_url, photo_signature, photo_crop FROM contacts WHERE guid = ?1",
        )?;
        let mut rows = stmt.query(params![guid])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        // The signature and the crop are each absent on real contacts — a picture shared by
        // the person it belongs to arrives with neither. Only the URL decides whether there
        // is anything to fetch.
        let Some(url) = row.get::<_, Option<String>>(0)? else {
            return Ok(None);
        };
        Ok(Some(ContactPhoto {
            url,
            signature: row.get(1)?,
            crop: row.get(2)?,
        }))
    }

    /// The address book in display order, optionally filtered.
    ///
    /// The filter matches names and any stored method, so searching a phone number finds the
    /// person it belongs to. Every word must match, in any order, without regard to case or
    /// accents, and a number matches however it was typed: "98765 43210" finds
    /// "+91 98765-43210". That is done here rather than in SQL, which folds ASCII alone and
    /// knows nothing about phone formatting.
    pub fn contacts(&self, search: Option<&str>) -> Result<Vec<Contact>> {
        let words = crate::fold::words(search.unwrap_or(""));

        let mut stmt = self.conn.prepare(
            "SELECT c.guid, c.first_name, c.middle_name, c.last_name, c.company, c.is_company,
                    c.birthday, c.notes, c.photo_url, c.photo_signature, c.photo_crop, c.etag
             FROM contacts c",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Contact {
                guid: row.get(0)?,
                first_name: row.get(1)?,
                middle_name: row.get(2)?,
                last_name: row.get(3)?,
                company: row.get(4)?,
                is_company: row.get::<_, i64>(5)? != 0,
                birthday: row.get(6)?,
                notes: row.get(7)?,
                photo_url: row.get(8)?,
                photo_signature: row.get(9)?,
                photo_crop: row.get(10)?,
                etag: row.get(11)?,
                methods: Vec::new(),
            })
        })?;
        let mut contacts = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        // One pass over every method rather than a query per person: an address book of a
        // few thousand would otherwise cost a few thousand queries on every keystroke.
        let mut methods: std::collections::HashMap<String, Vec<ContactMethod>> =
            std::collections::HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT contact_guid, kind, label, value FROM contact_methods
             ORDER BY contact_guid, position",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ContactMethod {
                    kind: row.get(1)?,
                    label: row.get(2)?,
                    value: row.get(3)?,
                },
            ))
        })?;
        for row in rows {
            let (guid, method) = row?;
            methods.entry(guid).or_default().push(method);
        }
        for contact in &mut contacts {
            contact.methods = methods.remove(&contact.guid).unwrap_or_default();
        }

        if !words.is_empty() {
            contacts.retain(|c| c.matches(&words));
        }
        let mut keyed: Vec<(String, Contact)> =
            contacts.into_iter().map(|c| (c.sort_key(), c)).collect();
        keyed.sort_by(|(a, _), (b, _)| crate::fold::natural(a, b));
        Ok(keyed.into_iter().map(|(_, c)| c).collect())
    }
}

/// Only the digits of a number, so its formatting stops mattering.
fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// True for a query word that reads as (part of) a phone number rather than a name.
fn looks_like_number(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
        && word
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '(' | ')' | '.' | '/'))
}

impl Contact {
    fn matches(&self, words: &[String]) -> bool {
        let mut text = [
            self.first_name.as_deref(),
            self.middle_name.as_deref(),
            self.last_name.as_deref(),
            self.company.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        for m in &self.methods {
            text.push('\n');
            text.push_str(&m.value);
        }
        let text = crate::fold::fold(&text);
        let numbers: Vec<String> = self
            .methods
            .iter()
            .filter(|m| m.kind == "phone")
            .map(|m| digits(&m.value))
            .collect();
        words.iter().all(|w| {
            text.contains(w.as_str())
                || (looks_like_number(w) && {
                    let d = digits(w);
                    numbers.iter().any(|n| n.contains(&d))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(guid: &str, first: Option<&str>, last: Option<&str>) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: first.map(ToString::to_string),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: last.map(ToString::to_string),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: vec![ContactMethod {
                kind: "phone".into(),
                label: Some("mobile".into()),
                value: format!("+91{guid}"),
            }],
        }
    }

    #[test]
    fn search_ignores_case_accents_and_word_order() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Émile"), Some("Zola")),
            contact("2", Some("Zoë"), Some("Ahuja")),
        ])
        .unwrap();
        let found = |q: &str| -> Vec<String> {
            lib.contacts(Some(q))
                .unwrap()
                .iter()
                .map(|c| c.guid.clone())
                .collect()
        };
        assert_eq!(found("emile"), ["1"]);
        assert_eq!(found("ÉMILE"), ["1"]);
        assert_eq!(found("zola emile"), ["1"]);
        assert_eq!(found("zoe"), ["2"]);
        // A wildcard character is a character, not a pattern.
        assert!(found("%").is_empty());
        assert!(found("_").is_empty());
    }

    #[test]
    fn a_number_matches_however_it_was_typed() {
        let mut lib = Library::in_memory().unwrap();
        let mut c = contact("1", Some("Meera"), None);
        c.methods[0].value = "+91 98765-43210".into();
        lib.put_contacts(&[c]).unwrap();
        for typed in ["9876543210", "98765 43210", "(987) 65", "+91-98765"] {
            assert_eq!(lib.contacts(Some(typed)).unwrap().len(), 1, "{typed}");
        }
        assert!(lib.contacts(Some("12345")).unwrap().is_empty());
    }

    #[test]
    fn accented_names_sort_with_their_letter() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Zoya"), Some("Zutshi")),
            contact("2", Some("Émile"), Some("Écrivain")),
            contact("3", Some("Aarav"), Some("Ahuja")),
        ])
        .unwrap();
        let order: Vec<String> = lib
            .contacts(None)
            .unwrap()
            .iter()
            .map(|c| c.guid.clone())
            .collect();
        assert_eq!(order, ["3", "2", "1"]);
    }

    #[test]
    fn contacts_sort_by_last_name_the_way_apple_does() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Aarav"), Some("Zutshi")),
            contact("2", Some("Zoya"), Some("Ahuja")),
        ])
        .unwrap();
        let names: Vec<String> = lib
            .contacts(None)
            .unwrap()
            .iter()
            .map(Contact::display_name)
            .collect();
        assert_eq!(names, vec!["Zoya Ahuja", "Aarav Zutshi"]);
    }

    /// A contact with only a phone number still needs something to show.
    #[test]
    fn a_nameless_contact_falls_back_to_its_first_method() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("9", None, None)]).unwrap();
        assert_eq!(lib.contacts(None).unwrap()[0].display_name(), "+919");
    }

    #[test]
    fn searching_matches_a_phone_number_as_well_as_a_name() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("555", Some("Meera"), Some("Iyer")),
            contact("777", Some("Rahul"), Some("Nair")),
        ])
        .unwrap();
        assert_eq!(lib.contacts(Some("meera")).unwrap().len(), 1);
        assert_eq!(
            lib.contacts(Some("+91777")).unwrap()[0].display_name(),
            "Rahul Nair"
        );
        assert_eq!(
            lib.contacts(Some("   ")).unwrap().len(),
            2,
            "a blank search is no filter"
        );
    }

    /// A number removed on the phone must not linger here.
    #[test]
    fn methods_are_replaced_not_accumulated() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("1", Some("Ada"), Some("Lovelace"))])
            .unwrap();

        let mut changed = contact("1", Some("Ada"), Some("Lovelace"));
        changed.methods = vec![ContactMethod {
            kind: "email".into(),
            label: None,
            value: "ada@example.com".into(),
        }];
        lib.put_contacts(&[changed]).unwrap();

        let stored = &lib.contacts(None).unwrap()[0];
        assert_eq!(stored.methods.len(), 1);
        assert_eq!(stored.methods[0].kind, "email");
    }

    #[test]
    fn contacts_apple_stops_reporting_are_removed() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("1", Some("A"), None), contact("2", Some("B"), None)])
            .unwrap();
        lib.put_contacts(&[contact("1", Some("A"), None)]).unwrap();
        assert_eq!(lib.contacts(None).unwrap().len(), 1);
    }
}
