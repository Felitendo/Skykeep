//! One search across everything the library holds — notes, reminders, events, contacts,
//! files and photos — offline and at once, as neither icloud.com nor a Mac offers: each app
//! searches only itself there. Each kind is searched as its own screen searches it (case and
//! accents aside, every word), and only its best few are kept: the window shows a handful of
//! each and opens the app for the rest.

use crate::{
    calendar::Event, contacts::Contact, drive::DriveItem, notes::Note, photos::Photo,
    reminders::Reminder, Library, Result,
};
use serde::Serialize;

/// What one search found, the first few of each kind, and how many of each there were.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Everything {
    pub notes: Vec<Note>,
    pub reminders: Vec<Reminder>,
    pub events: Vec<Event>,
    pub contacts: Vec<Contact>,
    pub files: Vec<DriveItem>,
    pub photos: Vec<Photo>,
    /// How many each kind found in all (up to what was looked through), in the order above.
    pub counts: [usize; 6],
}

impl Library {
    /// Everything that holds every word of `query`, at most `each` of a kind.
    pub fn search_everything(&self, query: &str, each: usize) -> Result<Everything> {
        if crate::fold::words(query).is_empty() {
            return Ok(Everything::default());
        }
        let mut notes = self.search_notes(query)?;
        let mut reminders = self.search_reminders(query)?;
        let mut events = self.search_events(query)?;
        let mut contacts = self.contacts(Some(query))?;
        let mut files = self.search_drive(query, each.saturating_mul(20))?;
        let mut photos = self.search_photos(query, each.saturating_mul(20))?;
        let counts = [
            notes.len(),
            reminders.len(),
            events.len(),
            contacts.len(),
            files.len(),
            photos.len(),
        ];
        notes.truncate(each);
        reminders.truncate(each);
        events.truncate(each);
        contacts.truncate(each);
        files.truncate(each);
        photos.truncate(each);
        Ok(Everything {
            notes,
            reminders,
            events,
            contacts,
            files,
            photos,
            counts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One word finds it wherever it is — a file and a contact here — case and accents aside;
    /// a word nothing holds finds nothing; each kind is kept to its few.
    #[test]
    fn one_search_finds_it_in_every_app() {
        let mut lib = Library::in_memory().unwrap();
        let file = |id: &str, name: &str| crate::drive::DriveItem {
            drivewsid: id.into(),
            parent_id: Some("root".into()),
            name: name.into(),
            extension: Some("pdf".into()),
            kind: "FILE".into(),
            size: Some(1),
            child_count: None,
            created_at: None,
            modified_at: Some(1),
            etag: None,
        };
        lib.put_drive_children(
            "root",
            &[
                file("F1", "Résumé Springfield"),
                file("F2", "Springfield map"),
                file("F3", "Budget"),
            ],
        )
        .unwrap();
        lib.put_contacts(&[crate::contacts::Contact {
            guid: "C1".into(),
            first_name: Some("Asha".into()),
            middle_name: None,
            last_name: Some("Springfield".into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            etag: None,
            methods: vec![],
        }])
        .unwrap();
        let found = lib.search_everything("SPRINGFIELD", 1).unwrap();
        assert_eq!(found.counts[4], 2, "both files found");
        assert_eq!(found.files.len(), 1, "and only the first kept");
        assert_eq!(found.contacts.len(), 1);
        assert_eq!(
            lib.search_everything("resume springfield", 5)
                .unwrap()
                .files
                .len(),
            1,
            "accents aside, every word"
        );
        assert_eq!(lib.search_everything("zebra", 5).unwrap().counts, [0; 6]);
        assert_eq!(lib.search_everything("   ", 5).unwrap().counts, [0; 6]);
    }
}
