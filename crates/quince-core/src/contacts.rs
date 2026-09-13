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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub guid: String,
    pub first_name: Option<String>,
    /// Apple keeps one, and four people in a real address book have one.
    #[serde(default)]
    pub middle_name: Option<String>,
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
        key.to_lowercase()
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
                                       sort_name, birthday, notes, etag, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(guid) DO UPDATE SET
                   first_name = excluded.first_name, middle_name = excluded.middle_name,
                   last_name = excluded.last_name,
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

    /// The address book in display order, optionally filtered.
    ///
    /// The filter matches names and any stored method, so searching a phone number finds the
    /// person it belongs to.
    pub fn contacts(&self, search: Option<&str>) -> Result<Vec<Contact>> {
        let needle = search
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", s.to_lowercase()));

        let mut stmt = self.conn.prepare(
            "SELECT c.guid, c.first_name, c.middle_name, c.last_name, c.company, c.is_company,
                    c.birthday, c.notes, c.etag
             FROM contacts c
             WHERE ?1 IS NULL
                OR lower(coalesce(c.first_name, '') || ' ' || coalesce(c.middle_name, '')
                         || ' ' || coalesce(c.last_name, '')
                         || ' ' || coalesce(c.company, '')) LIKE ?1
                OR EXISTS (SELECT 1 FROM contact_methods m
                           WHERE m.contact_guid = c.guid AND lower(m.value) LIKE ?1)
             ORDER BY c.sort_name",
        )?;
        let rows = stmt.query_map(params![needle], |row| {
            Ok(Contact {
                guid: row.get(0)?,
                first_name: row.get(1)?,
                middle_name: row.get(2)?,
                last_name: row.get(3)?,
                company: row.get(4)?,
                is_company: row.get::<_, i64>(5)? != 0,
                birthday: row.get(6)?,
                notes: row.get(7)?,
                etag: row.get(8)?,
                methods: Vec::new(),
            })
        })?;
        let mut contacts = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        let mut methods = self.conn.prepare(
            "SELECT kind, label, value FROM contact_methods
             WHERE contact_guid = ?1 ORDER BY position",
        )?;
        for contact in &mut contacts {
            let rows = methods.query_map(params![contact.guid], |row| {
                Ok(ContactMethod {
                    kind: row.get(0)?,
                    label: row.get(1)?,
                    value: row.get(2)?,
                })
            })?;
            contact.methods = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(contacts)
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
