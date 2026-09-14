//! Throwaway: what a big account costs. Local only — no network, no real data.
use quince_core::contacts::{Contact, ContactMethod};
use quince_core::notes::{Note, NoteFolder};
use quince_core::reminders::{Reminder, ReminderList};
use std::time::Instant;

fn human(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

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

    println!("\n--- what each screen asks for, and what crosses into the window ---\n");

    macro_rules! weigh {
        ($label:expr, $call:expr) => {{
            let t = Instant::now();
            let rows = $call;
            let ms = t.elapsed().as_millis();
            let bytes = serde_json::to_vec(&rows).map(|v| v.len()).unwrap_or(0);
            println!(
                "{:<24} {:>6} rows {:>5} ms   payload {:>9}",
                $label,
                rows.len(),
                ms,
                human(bytes)
            );
            bytes
        }};
    }

    let mut worst = 0usize;
    worst = worst.max(weigh!("contacts(None)", lib.contacts(None)?));
    worst = worst.max(weigh!("contacts(search)", lib.contacts(Some("Surname7"))?));
    worst = worst.max(weigh!("notes_in(None)", lib.notes_in(None)?));
    worst = worst.max(weigh!("note_folders()", lib.note_folders()?));
    worst = worst.max(weigh!(
        "reminders_in(list)",
        lib.reminders_in("List/3", true)?
    ));
    worst = worst.max(weigh!(
        "reminders_due_between",
        lib.reminders_due_between(0, i64::MAX)?
    ));
    worst = worst.max(weigh!("reminder_lists()", lib.reminder_lists()?));
    worst = worst.max(weigh!("calendars()", lib.calendars()?));

    println!("\nlargest single payload: {}", human(worst));

    std::fs::remove_dir_all(&dir).ok();
    Ok(())
}
