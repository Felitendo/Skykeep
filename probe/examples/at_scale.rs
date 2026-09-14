//! Throwaway: what a big account costs. Local only — no network, no real data.
use quince_core::contacts::{Contact, ContactMethod};
use quince_core::notes::{Note, NoteFolder};
use quince_core::reminders::{Reminder, ReminderList};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("quince-scale-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let mut lib = quince_core::Library::open(&dir)?;

    // A heavy but entirely ordinary account.
    let lists: Vec<ReminderList> = (0..50)
        .map(|i| ReminderList {
            guid: format!("List/{i}"),
            title: format!("List {i}"),
            colour: None,
            ctag: None,
            sort_order: i,
            enabled: true,
        })
        .collect();
    lib.put_reminder_lists(&lists)?;

    let reminders: Vec<Reminder> = (0..20_000)
        .map(|i| Reminder {
            guid: format!("Reminder/{i}"),
            list_guid: format!("List/{}", i % 50),
            title: format!("Something to do number {i}"),
            notes: Some("with a note on it".into()),
            due_at: Some(1_789_000_000 + i * 60),
            completed_at: if i % 3 == 0 {
                Some(1_789_000_000)
            } else {
                None
            },
            priority: i % 4,
            flagged: i % 7 == 0,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: Some(format!("e{i}")),
            raw: None,
        })
        .collect();
    let t = Instant::now();
    lib.put_reminders(&reminders)?;
    println!(
        "storing 20,000 reminders      {:>7} ms",
        t.elapsed().as_millis()
    );

    let contacts: Vec<Contact> = (0..5_000)
        .map(|i| Contact {
            guid: format!("C{i}"),
            first_name: Some(format!("Person{i}")),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some(format!("Surname{}", i % 900)),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: vec![
                ContactMethod {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: format!("+9199{i:08}"),
                },
                ContactMethod {
                    kind: "email".into(),
                    label: None,
                    value: format!("p{i}@example.invalid"),
                },
            ],
        })
        .collect();
    let t = Instant::now();
    lib.put_contacts(&contacts)?;
    println!(
        "storing 5,000 contacts        {:>7} ms",
        t.elapsed().as_millis()
    );

    lib.put_note_folders(&[NoteFolder {
        guid: "F1".into(),
        title: "Notes".into(),
        sort_order: 0,
        parent_guid: None,
    }])?;
    let notes: Vec<Note> = (0..5_000)
        .map(|i| Note {
            guid: format!("N{i}"),
            folder_guid: Some("F1".into()),
            title: format!("Note {i}"),
            snippet: Some("a snippet".into()),
            body: Some("x".repeat(2_000)),
            locked: false,
            editable: true,
            pinned: i % 50 == 0,
            created_at: Some(1_789_000_000),
            modified_at: Some(1_789_000_000 + i),
            etag: None,
        })
        .collect();
    let t = Instant::now();
    lib.put_notes(&notes)?;
    println!(
        "storing 5,000 notes           {:>7} ms",
        t.elapsed().as_millis()
    );

    println!("\n--- what the window asks for on every screen ---");
    let t = Instant::now();
    let n = lib.contacts(None)?.len();
    println!(
        "contacts(None)          {n:>6} rows  {:>7} ms",
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    let n = lib.contacts(Some("Surname7"))?.len();
    println!(
        "contacts(search)        {n:>6} rows  {:>7} ms",
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    let n = lib.notes_in(None)?.len();
    let bytes: usize = lib
        .notes_in(None)?
        .iter()
        .map(|x| x.body.as_deref().unwrap_or("").len())
        .sum();
    println!(
        "notes_in(None)          {n:>6} rows  {:>7} ms   {} KB of bodies",
        t.elapsed().as_millis(),
        bytes / 1024
    );

    let t = Instant::now();
    let n = lib.reminders_in("List/3", true)?.len();
    println!(
        "reminders_in(one list)  {n:>6} rows  {:>7} ms",
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    let n = lib.reminders_due_between(0, i64::MAX)?.len();
    println!(
        "reminders_due_between   {n:>6} rows  {:>7} ms",
        t.elapsed().as_millis()
    );

    std::fs::remove_dir_all(&dir).ok();
    Ok(())
}
