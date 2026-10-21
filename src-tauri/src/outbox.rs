//! Changes go to Apple behind the window, never in front of it. A command records the change
//! (quince_core::outbox) and lays it over the library in one transaction, then returns: every
//! view shows it at once, as a native app's own store would, with or without a network. This
//! worker sends what is queued, in the order it was made, one service at a time:
//!
//! - taken: off the queue; one sync pass afterwards brings Apple's own version of the record;
//! - Apple can't be reached (offline, signed out, a server fault): kept, and tried again with a
//!   growing wait, so a change made offline lands when the connection comes back — even after
//!   a restart, since the queue is in the library;
//! - refused for good (deleted elsewhere, edited in the same place, turned down): kept in the
//!   library marked refused, with what was typed, for the person to send again or let go;
//!   Apple's version read back over ours, and the window told why — `quince://outbox-refused`.
//!
//! Sent exactly once in effect: every change is safe to send twice. A reminder or card made
//! here carries a name chosen before it is queued, and a create finding that name at Apple
//! already does nothing; edits are merges against what Apple holds (a note's text against the
//! base it was typed over, a reminder's fields, a card's fields against the form's base), so
//! one that already landed lands again as no change.
//!
//! A sync pass that lands while changes wait would put Apple's older version over them, so
//! every pass lays the waiting changes over what it read (see [`replay`]).

use crate::{cloudkit_context, Auth, Data};
use quince_core::contacts::Contact;
use quince_core::outbox::Pending;
use quince_core::reminders::Reminder;
use quince_core::Library;
use quince_icloud::ckdocument::Run;
use quince_icloud::reminders::ReminderEdit;
use quince_icloud::Error;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, Notify};

static WAKE: Notify = Notify::const_new();

/// Something was queued: send it now.
pub fn kick() {
    WAKE.notify_one();
}

pub const REMINDERS: &str = "reminders";
pub const NOTES: &str = "notes";
pub const CONTACTS: &str = "contacts";

/// What a waiting change does to a reminder: ticked (`at` the moment) or not.
#[derive(Serialize, Deserialize)]
pub struct Done {
    pub done: bool,
    pub at: i64,
}

/// A reminder made here, under the name it was given here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Create {
    pub list_guid: String,
    pub title: String,
    pub due_at: Option<i64>,
    pub all_day: Option<bool>,
    pub flagged: Option<bool>,
}

/// `Option<Option<T>>` kept whole through JSON: absent is "leave it", null is "clear it".
mod double {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer, T: Serialize>(
        v: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

/// A reminder edit as queued: [`ReminderEdit`], with "clear" and "leave" kept apart, and the
/// list the reminder was in before any of it — what a move is made from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double")]
    pub notes: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double")]
    pub due_at: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_day: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flagged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double")]
    pub parent_guid: Option<Option<String>>,
    pub from_list: String,
}

impl Edit {
    fn new(e: &ReminderEdit, from_list: &str) -> Self {
        Self {
            title: e.title.clone(),
            notes: e.notes.clone(),
            due_at: e.due_at,
            all_day: e.all_day,
            flagged: e.flagged,
            completed: e.completed,
            priority: e.priority,
            list_guid: e.list_guid.clone(),
            parent_guid: e.parent_guid.clone(),
            from_list: from_list.to_string(),
        }
    }

    /// A later edit laid over this one: what it says wins, what it leaves stays.
    fn then(mut self, later: Self) -> Self {
        macro_rules! over {
            ($($f:ident),*) => { $( if later.$f.is_some() { self.$f = later.$f; } )* };
        }
        over!(
            title,
            notes,
            due_at,
            all_day,
            flagged,
            completed,
            priority,
            list_guid,
            parent_guid
        );
        self
    }

    fn to_apple(&self) -> ReminderEdit {
        ReminderEdit {
            title: self.title.clone(),
            notes: self.notes.clone(),
            due_at: self.due_at,
            all_day: self.all_day,
            flagged: self.flagged,
            completed: self.completed,
            priority: self.priority,
            list_guid: self.list_guid.clone(),
            parent_guid: self.parent_guid.clone(),
        }
    }

    /// The edit, made to the stored reminder.
    fn lay_on(&self, r: &mut Reminder) {
        if let Some(t) = &self.title {
            r.title = t.trim().to_string();
        }
        if let Some(n) = &self.notes {
            r.notes.clone_from(n);
        }
        if let Some(due) = self.due_at {
            r.due_at = due;
            r.all_day = self.all_day.unwrap_or(false) && due.is_some();
        }
        if let Some(f) = self.flagged {
            r.flagged = f;
        }
        if let Some(c) = self.completed {
            r.completed_at = if c {
                r.completed_at.or(Some(jiff::Timestamp::now().as_second()))
            } else {
                None
            };
        }
        if let Some(p) = self.priority {
            r.priority = match p {
                1..=3 => 1,
                4..=6 => 5,
                7..=9 => 9,
                _ => 0,
            };
        }
        if let Some(l) = &self.list_guid {
            r.list_guid.clone_from(l);
        }
        if let Some(p) = &self.parent_guid {
            r.parent_guid.clone_from(p);
        }
    }
}

/// A reminder deleted here, with the subtasks that go with it, as they were.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delete {
    pub reminder: Reminder,
    pub children: Vec<Reminder>,
}

/// A note's text saved here: what was typed, and what it was typed over.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteSave {
    pub body: String,
    pub base: Option<String>,
    pub runs: Option<Vec<Run>>,
    pub base_runs: Option<Vec<Run>>,
}

/// A card saved here: as the form left it, and as it opened (`None` for a new card).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactSave {
    pub contact: Contact,
    pub base: Option<Contact>,
    pub create: bool,
}

fn json<T: Serialize>(v: &T) -> quince_core::Result<String> {
    serde_json::to_string(v)
        .map_err(|e| quince_core::Error::Io(std::io::Error::other(e.to_string())))
}

/// The newest change waiting for `record` in `service`, if it is still the last word on that
/// record (nothing of another kind queued after it) and of `kind`.
fn last_word(
    library: &Library,
    service: &str,
    record: &str,
    kind: &str,
) -> quince_core::Result<Option<Pending>> {
    Ok(library
        .outbox_pending(service)?
        .into_iter()
        .rfind(|p| p.record == record)
        .filter(|p| p.kind == kind))
}

// --- Recording: the command's half. Each records the change and lays it over the library as
// one transaction, so a crash leaves both or neither.

/// A note's text, saved here. Several saves before one is sent become one, sent from the
/// text the first was typed over; one already on its way is followed by the next, typed over
/// what it sends.
pub fn record_note_save(
    library: &mut Library,
    guid: &str,
    body: &str,
    base: Option<String>,
    runs: Option<Vec<Run>>,
    base_runs: Option<Vec<Run>>,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let stored = l.note_body(guid)?;
        let waiting: Option<(Pending, NoteSave)> = l
            .outbox_waiting(NOTES, guid, "save")?
            .into_iter()
            .next_back()
            .and_then(|p| serde_json::from_str(&p.payload).ok().map(|s| (p, s)));
        let save = |base, base_runs| NoteSave {
            body: body.to_string(),
            base,
            runs: runs.clone(),
            base_runs,
        };
        match waiting {
            Some((p, before)) if p.tries == 0 => {
                let merged = save(before.base, before.base_runs);
                if !l.outbox_amend(p.id, &json(&merged)?)? {
                    l.outbox_append(NOTES, guid, "save", &json(&merged)?)?;
                }
            }
            Some((_, before)) => {
                l.outbox_append(
                    NOTES,
                    guid,
                    "save",
                    &json(&save(Some(before.body), before.runs))?,
                )?;
            }
            None => {
                // Nothing waiting: the library holds what Apple holds, the base when the
                // editor did not say.
                let base = base.or(stored);
                l.outbox_append(NOTES, guid, "save", &json(&save(base, base_runs))?)?;
            }
        }
        lay_note(l, guid, body, runs.as_deref())
    })
}

fn lay_note(
    l: &mut Library,
    guid: &str,
    body: &str,
    runs: Option<&[Run]>,
) -> quince_core::Result<()> {
    let (title, snippet) = quince_icloud::notes::title_and_snippet(body);
    l.set_note_text(
        guid,
        body,
        &title,
        Some(snippet.as_str()).filter(|s| !s.is_empty()),
    )?;
    if let Some(runs) = runs {
        l.set_note_runs(guid, &json(&runs)?)?;
    }
    Ok(())
}

/// A reminder made here: named now, shown now, sent behind.
pub fn record_reminder_create(
    library: &mut Library,
    create: &Create,
) -> quince_core::Result<String> {
    let guid = quince_icloud::reminders::new_reminder_name();
    library.atomically(|l| {
        l.outbox_append(REMINDERS, &guid, "create", &json(create)?)?;
        lay_create(l, &guid, create)?;
        Ok(guid.clone())
    })
}

fn lay_create(l: &mut Library, guid: &str, c: &Create) -> quince_core::Result<()> {
    if l.reminder(guid)?.is_some() {
        return Ok(());
    }
    // A list deleted since has nowhere to show it.
    if !l.reminder_lists()?.iter().any(|x| x.guid == c.list_guid) {
        return Ok(());
    }
    l.put_reminder_local(&Reminder {
        guid: guid.to_string(),
        list_guid: c.list_guid.clone(),
        title: c.title.trim().to_string(),
        due_at: c.due_at,
        all_day: c.all_day.unwrap_or(false) && c.due_at.is_some(),
        flagged: c.flagged.unwrap_or(false),
        ..Default::default()
    })
}

/// An edit to a reminder. Edits not yet sent and still the last word on it are one edit.
pub fn record_reminder_edit(
    library: &mut Library,
    reminder: &Reminder,
    edit: &ReminderEdit,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let fresh = Edit::new(edit, &reminder.list_guid);
        let waiting = last_word(l, REMINDERS, &reminder.guid, "edit")?
            .filter(|p| p.tries == 0)
            .and_then(|p| {
                serde_json::from_str::<Edit>(&p.payload)
                    .ok()
                    .map(|e| (p, e))
            });
        let amended = match waiting {
            Some((p, before)) => l.outbox_amend(p.id, &json(&before.then(fresh.clone()))?)?,
            None => false,
        };
        if !amended {
            l.outbox_append(REMINDERS, &reminder.guid, "edit", &json(&fresh)?)?;
        }
        lay_edit(l, &reminder.guid, &fresh)
    })
}

fn lay_edit(l: &mut Library, guid: &str, e: &Edit) -> quince_core::Result<()> {
    let Some(mut r) = l.reminder(guid)? else {
        return Ok(());
    };
    e.lay_on(&mut r);
    l.put_reminder_local(&r)?;
    // Moved, its subtasks go with it, as on the phone.
    if let Some(to) = &e.list_guid {
        for mut child in l.subtasks_of(guid)? {
            child.list_guid.clone_from(to);
            l.put_reminder_local(&child)?;
        }
    }
    Ok(())
}

/// A reminder deleted here, with its subtasks.
pub fn record_reminder_delete(
    library: &mut Library,
    reminder: &Reminder,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let children = l.subtasks_of(&reminder.guid)?;
        let delete = Delete {
            reminder: reminder.clone(),
            children,
        };
        l.outbox_append(REMINDERS, &reminder.guid, "delete", &json(&delete)?)?;
        lay_delete(l, &delete)
    })
}

fn lay_delete(l: &mut Library, d: &Delete) -> quince_core::Result<()> {
    let mut gone: Vec<String> = d.children.iter().map(|c| c.guid.clone()).collect();
    gone.push(d.reminder.guid.clone());
    l.forget_reminders(&gone, &[])
}

/// A card saved here — made (no id yet: one is given now) or edited. Answers its id.
pub fn record_contact_save(
    library: &mut Library,
    contact: &Contact,
    base: Option<Contact>,
) -> quince_core::Result<String> {
    let create = contact.guid.is_empty();
    let mut contact = contact.clone();
    if create {
        contact.guid = quince_icloud::contacts::new_contact_id();
        contact.etag = None;
    }
    let guid = contact.guid.clone();
    library.atomically(|l| {
        let waiting = last_word(l, CONTACTS, &guid, "save")?.and_then(|p| {
            serde_json::from_str::<ContactSave>(&p.payload)
                .ok()
                .map(|s| (p, s))
        });
        match waiting {
            Some((p, before)) if p.tries == 0 => {
                // One save, from the card as the form first opened it (or still a create).
                let save = ContactSave {
                    contact: contact.clone(),
                    base: before.base,
                    create: before.create,
                };
                if !l.outbox_amend(p.id, &json(&save)?)? {
                    l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
                }
            }
            Some((_, before)) => {
                // Behind one that may have landed: an edit of the card that one sends.
                let save = ContactSave {
                    contact: contact.clone(),
                    base: Some(before.contact),
                    create: false,
                };
                l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
            }
            None => {
                let save = ContactSave {
                    contact: contact.clone(),
                    base: if create { None } else { base.clone() },
                    create,
                };
                l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
            }
        }
        l.put_contact(&contact)?;
        Ok(guid.clone())
    })
}

/// Lay one waiting change over the library.
fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    match (p.service.as_str(), p.kind.as_str()) {
        (REMINDERS, "done") => {
            if let Ok(d) = serde_json::from_str::<Done>(&p.payload) {
                l.set_reminder_completed(&p.record, d.done.then_some(d.at))?;
            }
        }
        (REMINDERS, "create") => {
            if let Ok(c) = serde_json::from_str::<Create>(&p.payload) {
                lay_create(l, &p.record, &c)?;
            }
        }
        (REMINDERS, "edit") => {
            if let Ok(e) = serde_json::from_str::<Edit>(&p.payload) {
                lay_edit(l, &p.record, &e)?;
            }
        }
        (REMINDERS, "delete") => {
            if let Ok(d) = serde_json::from_str::<Delete>(&p.payload) {
                lay_delete(l, &d)?;
            }
        }
        (NOTES, "save") => {
            if let Ok(s) = serde_json::from_str::<NoteSave>(&p.payload) {
                if l.note(&p.record)?.is_some() {
                    lay_note(l, &p.record, &s.body, s.runs.as_deref())?;
                }
            }
        }
        (CONTACTS, "save") => {
            if let Ok(s) = serde_json::from_str::<ContactSave>(&p.payload) {
                l.put_contact(&s.contact)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Lay every change still waiting for `service` over what a sync pass just stored, in the
/// order they were made.
pub fn replay(library: &mut Library, service: &str) -> quince_core::Result<()> {
    for p in library.outbox_pending(service)? {
        lay(library, &p)?;
    }
    Ok(())
}

/// The reminder changes still waiting, laid over what a pass stored.
pub fn replay_reminders(library: &mut Library) -> quince_core::Result<()> {
    replay(library, REMINDERS)
}

// --- Sending: the worker's half.

/// Runs for as long as the app does. Starts by sending whatever an earlier run left.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut wait = Duration::ZERO;
        loop {
            tokio::select! {
                () = WAKE.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
            let _ = app.emit(CHANGED, ());
            let outcomes = [
                drain_reminders(&app).await,
                drain_notes(&app).await,
                drain_contacts(&app).await,
            ];
            let _ = app.emit(CHANGED, ());
            wait = if outcomes.iter().any(|o| matches!(o, Outcome::Later)) {
                // Apple out of reach: wait longer each time, up to two minutes.
                (wait * 2).clamp(Duration::from_secs(4), Duration::from_secs(120))
            } else {
                Duration::from_secs(30)
            };
        }
    });
}

/// Told to the window whenever what waits may have changed.
pub const CHANGED: &str = "quince://outbox-changed";

enum Outcome {
    Clear,
    Later,
}

/// How many times an answer this app does not understand (or a conflict a merge did not
/// settle) is retried before the change is held as refused — a fault that will never clear
/// held every change queued after it forever, each shown as made.
const PROTOCOL_TRIES: i64 = 5;

/// Whether Apple could not be reached (keep it) rather than said no (hold it as refused).
/// `tries` counts this one.
fn transient(e: &Error, tries: i64) -> bool {
    match e {
        Error::Network(_)
        | Error::ReauthRequired
        | Error::StaleCursor
        | Error::Busy
        | Error::Io(_)
        | Error::Storage(_) => true,
        Error::Protocol(_) | Error::Conflict => tries < PROTOCOL_TRIES,
        _ => false,
    }
}

#[derive(Clone, serde::Serialize)]
struct RefusedEvent {
    service: &'static str,
    record: String,
    message: String,
}

/// What one pass over the queue came to.
struct Report {
    outcome: Outcome,
    sent: bool,
    /// Each refused change: its record and Apple's reason.
    refused: Vec<(String, String)>,
}

/// Send what is waiting for `service`, oldest first, with `send`. Stops at the first change
/// Apple can't be reached for, leaving it and everything after it queued, in order.
async fn drain<F, Fut>(library: &Mutex<Library>, service: &str, pass: &Mutex<()>, send: F) -> Report
where
    F: Fn(Pending) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    let mut report = Report {
        outcome: Outcome::Clear,
        sent: false,
        refused: Vec::new(),
    };
    loop {
        // No sync pass between sending and taking it off the queue: one fetched just before
        // the write and stored just after would put the old state back with nothing waiting
        // to lay over it.
        let _pass = pass.lock().await;
        let next = library.lock().await.outbox_take(service);
        let p = match next {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(_) => {
                report.outcome = Outcome::Later;
                return report;
            }
        };
        let (id, tries, record) = (p.id, p.tries, p.record.clone());
        match send(p).await {
            Ok(()) => {
                let _ = library.lock().await.outbox_done(id);
                report.sent = true;
            }
            Err(e) if transient(&e, tries) => {
                let _ = library.lock().await.outbox_retry(id, &e.to_string());
                report.outcome = Outcome::Later;
                return report;
            }
            Err(e) => {
                let _ = library.lock().await.outbox_refuse(id, &e.to_string());
                report.refused.push((record, e.to_string()));
            }
        }
    }
    report
}

fn unreadable(e: &serde_json::Error) -> Error {
    Error::Refused(format!("A queued change could not be read: {e}"))
}

fn tell_refused(app: &AppHandle, service: &'static str, report: &Report) {
    for (record, message) in &report.refused {
        let _ = app.emit(
            "quince://outbox-refused",
            RefusedEvent {
                service,
                record: record.clone(),
                message: message.clone(),
            },
        );
    }
}

/// Whether anything waits for `service` — the answer when Apple can't be asked yet.
async fn waiting(data: &Data, service: &str) -> Outcome {
    if data
        .0
        .lock()
        .await
        .outbox_pending(service)
        .map(|p| p.is_empty())
        .unwrap_or(true)
    {
        Outcome::Clear
    } else {
        Outcome::Later
    }
}

async fn drain_reminders(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, REMINDERS).await.is_clear() {
        return Outcome::Clear;
    }
    let session = if crate::same_account(app).await {
        cloudkit_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        // Not signed in, or the session is being renewed: nothing can go yet.
        return Outcome::Later;
    };
    let report = drain(&data.0, REMINDERS, &crate::REMINDERS_PASS, |p| {
        send_reminder(&data.0, &http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, REMINDERS, &report);
    // Apple's own version of what was sent — its new change tags, and the truth over anything
    // it refused.
    if report.sent || !report.refused.is_empty() {
        let _ = crate::refresh_reminders(&http, &root, params, &data, false).await;
        crate::alerts::changed();
        let _ = app.emit("quince://reminders-synced", ());
    }
    report.outcome
}

impl Outcome {
    fn is_clear(&self) -> bool {
        matches!(self, Outcome::Clear)
    }
}

async fn send_reminder(
    library: &Mutex<Library>,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    use quince_icloud::reminders as r;
    let stored = library.lock().await.reminder(&p.record)?;
    match p.kind.as_str() {
        "done" => {
            let d: Done = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            // Gone from the library (deleted here or elsewhere): nothing left to tick.
            let Some(reminder) = stored else {
                return Ok(());
            };
            // A change made on another device after this one was queued wins; the pass after
            // sending reads Apple's version back.
            if r::tick_overtaken(http, root, params.clone(), &reminder, d.done, d.at).await? {
                return Ok(());
            }
            r::set_completed_cloudkit(http, root, params, &reminder, d.done).await
        }
        "create" => {
            let c: Create = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            r::create_cloudkit_named(
                http,
                root,
                params.clone(),
                &p.record,
                &c.list_guid,
                &c.title,
            )
            .await?;
            if c.due_at.is_some() || c.flagged.is_some() {
                let made = Reminder {
                    guid: p.record.clone(),
                    list_guid: c.list_guid.clone(),
                    title: c.title.clone(),
                    ..Default::default()
                };
                let edit = ReminderEdit {
                    due_at: c.due_at.map(Some),
                    all_day: c.all_day,
                    flagged: c.flagged,
                    ..Default::default()
                };
                // The reminder exists at Apple now: its date or flag failing is noted, never
                // a reason to make it again.
                if let Err(e) = r::edit_cloudkit(http, root, params, &made, &edit).await {
                    quiet!("a new reminder's date or flag was not set: {e}");
                }
            }
            Ok(())
        }
        "edit" => {
            let e: Edit = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            let Some(mut reminder) = stored else {
                return Err(Error::Refused(
                    "This reminder was deleted on another device, so the change wasn’t saved."
                        .into(),
                ));
            };
            // Sent as a move from where it was before any of this edit.
            reminder.list_guid.clone_from(&e.from_list);
            let edit = e.to_apple();
            r::edit_cloudkit(http, root, params.clone(), &reminder, &edit).await?;
            if let Some(to) = e.list_guid.as_deref().filter(|to| *to != e.from_list) {
                let children = library.lock().await.subtasks_of(&p.record)?;
                let follow = ReminderEdit {
                    list_guid: Some(to.to_string()),
                    ..Default::default()
                };
                for mut child in children {
                    child.list_guid.clone_from(&e.from_list);
                    if let Err(e) =
                        r::edit_cloudkit(http, root, params.clone(), &child, &follow).await
                    {
                        quiet!("a subtask did not follow its reminder: {e}");
                    }
                }
            }
            // A date carries its alert, as on the phone; made all-day, it rings none.
            let alarm_change = if e.all_day == Some(true) {
                Some(None)
            } else {
                e.due_at
            };
            if let Some(due) = alarm_change {
                let zone = jiff::tz::TimeZone::system();
                r::set_alarm_cloudkit(
                    http,
                    root,
                    params,
                    &p.record,
                    due,
                    zone.iana_name().unwrap_or("UTC"),
                )
                .await?;
            }
            Ok(())
        }
        "delete" => {
            let d: Delete = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            match r::delete_cloudkit(http, root, params.clone(), &d.reminder).await {
                Ok(()) | Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
            for child in &d.children {
                if let Err(e) = r::delete_cloudkit(http, root, params.clone(), child).await {
                    quiet!("a subtask did not go with its reminder: {e}");
                }
            }
            Ok(())
        }
        other => Err(Error::Refused(format!(
            "Skykeep does not know how to send “{other}”."
        ))),
    }
}

async fn drain_notes(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, NOTES).await.is_clear() {
        return Outcome::Clear;
    }
    let session = if crate::same_account(app).await {
        cloudkit_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        return Outcome::Later;
    };
    let report = drain(&data.0, NOTES, &crate::NOTES_PASS, |p| {
        send_note(&data.0, &http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, NOTES, &report);
    if report.sent || !report.refused.is_empty() {
        let _ = crate::refresh_notes(&http, &root, params, &data).await;
    }
    report.outcome
}

async fn send_note(
    library: &Mutex<Library>,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    let s: NoteSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    let Some(note) = library.lock().await.note(&p.record)? else {
        return Err(Error::Refused(
            "This note was deleted on another device, so the edit wasn’t saved.".into(),
        ));
    };
    let saved = quince_icloud::notes::edit_rich_cloudkit_from(
        http,
        root,
        params,
        &note,
        s.base.as_deref(),
        &s.body,
        s.runs.as_deref(),
        s.base_runs.as_deref(),
    )
    .await?;
    // Apple's change feed does not always carry our own write back on the very next pass:
    // what Apple now holds — ours, or ours with another device's edit merged in — is what the
    // note says, unless more typing waits to go after it.
    let mut l = library.lock().await;
    let more = l
        .outbox_waiting(NOTES, &p.record, "save")?
        .iter()
        .any(|w| w.id != p.id);
    if !more {
        let runs = s.runs.as_ref().map(|runs| {
            if saved == s.body {
                runs.clone()
            } else {
                quince_icloud::ckdocument::carry_runs(&s.body, runs, &saved)
            }
        });
        lay_note(&mut l, &p.record, &saved, runs.as_deref())?;
    }
    Ok(())
}

async fn drain_contacts(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, CONTACTS).await.is_clear() {
        return Outcome::Clear;
    }
    let session = if crate::same_account(app).await {
        crate::contacts_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        return Outcome::Later;
    };
    let report = drain(&data.0, CONTACTS, &crate::CONTACTS_PASS, |p| {
        send_contact(&http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, CONTACTS, &report);
    if report.sent || !report.refused.is_empty() {
        let _one = crate::CONTACTS_PASS.lock().await;
        let _ = crate::resync_contacts(&http, &root, params, &data).await;
    }
    report.outcome
}

async fn send_contact(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    use quince_icloud::contacts as c;
    let s: ContactSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    let (_, tokens) = c::fetch_with_tokens(http, root, params.clone()).await?;
    if s.create {
        return c::create_named(http, root, params, &tokens, &p.record, &s.contact).await;
    }
    let card = c::card(http, root, params.clone(), &p.record).await?;
    c::update_from(
        http,
        root,
        params,
        &tokens,
        &s.contact,
        card.as_ref(),
        s.base.as_ref(),
    )
    .await
}

// --- What the window asks.

/// A change waiting to go.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingView {
    service: String,
    record: String,
    kind: String,
}

/// A change Apple turned down, with what it held, for the person to keep.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusedView {
    id: i64,
    service: String,
    record: String,
    kind: String,
    reason: String,
    made_at: i64,
    /// What was typed, in words: the note's text, the reminder's title, the person's name.
    content: String,
}

#[derive(Serialize)]
pub struct Status {
    waiting: Vec<WaitingView>,
    refused: Vec<RefusedView>,
}

/// The words a refused change held.
fn content_of(p: &Pending) -> String {
    let v: serde_json::Value = serde_json::from_str(&p.payload).unwrap_or_default();
    let s = |ptr: &str| v.pointer(ptr).and_then(|x| x.as_str()).map(str::to_string);
    match (p.service.as_str(), p.kind.as_str()) {
        (NOTES, _) => s("/body"),
        (REMINDERS, "create") => s("/title"),
        (REMINDERS, "edit") => s("/title").or_else(|| s("/notes")),
        (REMINDERS, "delete") => s("/reminder/title"),
        (CONTACTS, _) => {
            let name: Vec<String> = [
                "/contact/firstName",
                "/contact/lastName",
                "/contact/company",
            ]
            .iter()
            .filter_map(|ptr| s(ptr))
            .filter(|x| !x.trim().is_empty())
            .collect();
            Some(name.join(" "))
        }
        _ => None,
    }
    .unwrap_or_default()
}

pub fn status(library: &Library) -> quince_core::Result<Status> {
    Ok(Status {
        waiting: library
            .outbox_all_pending()?
            .into_iter()
            .map(|p| WaitingView {
                service: p.service,
                record: p.record,
                kind: p.kind,
            })
            .collect(),
        refused: library
            .outbox_refused()?
            .into_iter()
            .map(|r| RefusedView {
                id: r.change.id,
                content: content_of(&r.change),
                service: r.change.service,
                record: r.change.record,
                kind: r.change.kind,
                reason: r.reason,
                made_at: r.made_at,
            })
            .collect(),
    })
}

/// What waits for Apple and what Apple turned down.
#[tauri::command]
pub async fn outbox_status(data: State<'_, Data>) -> Result<Status, String> {
    status(&*data.0.lock().await).map_err(|e| e.to_string())
}

/// A refused change, sent again. It is laid over the library again as it goes.
#[tauri::command]
pub async fn outbox_resend(id: i64, app: AppHandle, data: State<'_, Data>) -> Result<(), String> {
    {
        let mut l = data.0.lock().await;
        l.atomically(|l| {
            if l.outbox_resend(id)? {
                if let Some(service) = l
                    .outbox_all_pending()?
                    .into_iter()
                    .next_back()
                    .map(|p| p.service)
                {
                    replay(l, &service)?;
                }
            }
            Ok::<_, quince_core::Error>(())
        })
        .map_err(|e| e.to_string())?;
    }
    let _ = app.emit(CHANGED, ());
    kick();
    Ok(())
}

/// A refused change let go of: Apple's version stands.
#[tauri::command]
pub async fn outbox_discard(id: i64, app: AppHandle, data: State<'_, Data>) -> Result<(), String> {
    data.0
        .lock()
        .await
        .outbox_discard(id)
        .map_err(|e| e.to_string())?;
    let _ = app.emit(CHANGED, ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::reminders::ReminderList;
    use std::sync::{Arc, Mutex as StdMutex};

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        let list = |g: &str| ReminderList {
            guid: g.into(),
            title: g.into(),
            enabled: true,
            ..Default::default()
        };
        lib.put_reminder_lists(&[list("L"), list("M")]).unwrap();
        let r = |g: &str| Reminder {
            guid: g.into(),
            list_guid: "L".into(),
            title: g.into(),
            etag: Some("t".into()),
            ..Default::default()
        };
        lib.put_reminders(&[r("a"), r("b"), r("c")]).unwrap();
        lib
    }
    fn queue(lib: &mut Library, g: &str, done: bool) {
        lib.set_reminder_completed(g, done.then_some(1)).unwrap();
        let payload = serde_json::to_string(&Done { done, at: 1 }).unwrap();
        lib.outbox_push(REMINDERS, g, "done", &payload).unwrap();
    }
    fn note(lib: &mut Library, guid: &str, body: &str) {
        lib.put_notes(&[quince_core::notes::Note {
            guid: guid.into(),
            folder_guid: None,
            title: body.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            pinned: false,
            created_at: None,
            modified_at: None,
            editable: true,
            etag: None,
            runs: None,
        }])
        .unwrap();
    }
    fn person(guid: &str, first: &str) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: Some(first.into()),
            middle_name: None,
            last_name: None,
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            etag: None,
            methods: vec![],
        }
    }

    #[test]
    fn sends_in_order_keeps_what_cannot_reach_apple_and_holds_refusals() {
        tauri::async_runtime::block_on(async {
            let lib = Mutex::new(library());
            let pass = Mutex::new(());
            {
                let mut l = lib.lock().await;
                queue(&mut l, "a", true);
                queue(&mut l, "b", true);
                queue(&mut l, "c", true);
            }
            // Apple turns "b" down; everything goes through in the order it was made.
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let s2 = seen.clone();
            let report = drain(&lib, REMINDERS, &pass, move |p| {
                s2.lock().unwrap().push(p.record.clone());
                async move {
                    if p.record == "b" {
                        Err(Error::Refused("changed on another device".into()))
                    } else {
                        Ok(())
                    }
                }
            })
            .await;
            assert_eq!(*seen.lock().unwrap(), vec!["a", "b", "c"]);
            assert!(report.sent && report.outcome.is_clear());
            assert_eq!(
                report.refused,
                vec![("b".to_string(), "changed on another device".to_string())]
            );
            assert!(lib
                .lock()
                .await
                .outbox_pending(REMINDERS)
                .unwrap()
                .is_empty());
            // Refused is kept, never dropped.
            let refused = lib.lock().await.outbox_refused().unwrap();
            assert_eq!(refused.len(), 1);
            assert_eq!(refused[0].change.record, "b");

            // Offline: the first stays, with all after it, in order, for next time.
            {
                let mut l = lib.lock().await;
                queue(&mut l, "a", false);
                queue(&mut l, "c", false);
            }
            let report = drain(&lib, REMINDERS, &pass, |_p| async {
                Err(Error::Protocol("CloudKit refused the change (503)".into()))
            })
            .await;
            assert!(!report.outcome.is_clear() && !report.sent);
            let left = lib.lock().await.outbox_pending(REMINDERS).unwrap();
            assert_eq!(
                left.iter().map(|p| p.record.as_str()).collect::<Vec<_>>(),
                vec!["a", "c"]
            );
            assert_eq!(left[0].tries, 1);

            // A sync pass that read Apple's older copy is laid over with what is still waiting.
            {
                let mut l = lib.lock().await;
                l.set_reminder_completed("a", Some(5)).unwrap();
                replay_reminders(&mut l).unwrap();
                assert_eq!(l.reminder("a").unwrap().unwrap().completed_at, None);
            }

            // Back online: both go.
            let report = drain(&lib, REMINDERS, &pass, |_p| async { Ok(()) }).await;
            assert!(report.sent && report.outcome.is_clear());
            assert!(lib
                .lock()
                .await
                .outbox_pending(REMINDERS)
                .unwrap()
                .is_empty());
        });
    }

    /// Offline, a reminder is made, edited twice and another deleted: each shows at once, a
    /// pass reading Apple's older state is laid over with them all, and they go in the order
    /// they were made, the two edits as one.
    #[test]
    fn reminder_changes_show_at_once_survive_a_pass_and_go_in_order() {
        let mut lib = library();
        let made = record_reminder_create(
            &mut lib,
            &Create {
                list_guid: "L".into(),
                title: " Buy milk ".into(),
                due_at: Some(100),
                all_day: Some(true),
                flagged: None,
            },
        )
        .unwrap();
        assert!(made.starts_with("Reminder/"), "named here, before Apple");
        let shown = lib.reminder(&made).unwrap().unwrap();
        assert_eq!(
            (shown.title.as_str(), shown.due_at, shown.all_day),
            ("Buy milk", Some(100), true)
        );

        let a = lib.reminder("a").unwrap().unwrap();
        record_reminder_edit(
            &mut lib,
            &a,
            &ReminderEdit {
                title: Some("A renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let a = lib.reminder("a").unwrap().unwrap();
        record_reminder_edit(
            &mut lib,
            &a,
            &ReminderEdit {
                notes: Some(None),
                list_guid: Some("M".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let b = lib.reminder("b").unwrap().unwrap();
        record_reminder_delete(&mut lib, &b).unwrap();

        let a = lib.reminder("a").unwrap().unwrap();
        assert_eq!((a.title.as_str(), a.list_guid.as_str()), ("A renamed", "M"));
        assert!(lib.reminder("b").unwrap().is_none(), "deleted at once");
        assert!(a.etag.is_some(), "Apple's tag kept for the write");

        let p = lib.outbox_pending(REMINDERS).unwrap();
        assert_eq!(
            p.iter()
                .map(|x| (x.record.as_str(), x.kind.as_str()))
                .collect::<Vec<_>>(),
            vec![(made.as_str(), "create"), ("a", "edit"), ("b", "delete")]
        );
        let edit: Edit = serde_json::from_str(&p[1].payload).unwrap();
        assert_eq!(edit.title.as_deref(), Some("A renamed"));
        assert_eq!(edit.notes, Some(None), "clear kept apart from leave");
        assert_eq!(edit.from_list, "L", "moved from where it first was");

        // A full pass reads Apple's state from before any of it.
        lib.retain_reminders(&["a".into(), "b".into(), "c".into()])
            .unwrap();
        lib.put_reminders(&[
            Reminder {
                guid: "a".into(),
                list_guid: "L".into(),
                title: "a".into(),
                etag: Some("t2".into()),
                ..Default::default()
            },
            Reminder {
                guid: "b".into(),
                list_guid: "L".into(),
                title: "b".into(),
                ..Default::default()
            },
        ])
        .unwrap();
        replay_reminders(&mut lib).unwrap();
        assert!(lib.reminder(&made).unwrap().is_some());
        let a = lib.reminder("a").unwrap().unwrap();
        assert_eq!(
            (a.title.as_str(), a.list_guid.as_str(), a.etag.as_deref()),
            ("A renamed", "M", Some("t2"))
        );
        assert!(lib.reminder("b").unwrap().is_none());
    }

    /// An edit already taken to be sent is followed, never rewritten.
    #[test]
    fn an_edit_on_its_way_is_followed_not_rewritten() {
        let mut lib = library();
        let a = lib.reminder("a").unwrap().unwrap();
        let edit = |t: &str| ReminderEdit {
            title: Some(t.into()),
            ..Default::default()
        };
        record_reminder_edit(&mut lib, &a, &edit("one")).unwrap();
        lib.outbox_take(REMINDERS).unwrap();
        record_reminder_edit(&mut lib, &a, &edit("two")).unwrap();
        let p = lib.outbox_pending(REMINDERS).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            serde_json::from_str::<Edit>(&p[0].payload)
                .unwrap()
                .title
                .as_deref(),
            Some("one")
        );
        assert_eq!(
            serde_json::from_str::<Edit>(&p[1].payload)
                .unwrap()
                .title
                .as_deref(),
            Some("two")
        );
    }

    /// Note saves: shown at once; several unsent are one, from the first base; one on its way
    /// is followed by the next typed over what it sends; a pass is laid over.
    #[test]
    fn note_saves_keep_their_base_and_survive_a_pass() {
        let mut lib = Library::in_memory().unwrap();
        note(&mut lib, "N", "Apple's text");
        record_note_save(&mut lib, "N", "Apple's text, mine", None, None, None).unwrap();
        record_note_save(
            &mut lib,
            "N",
            "Apple's text, mine more",
            Some("ignored".into()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Apple's text, mine more")
        );
        let p = lib.outbox_pending(NOTES).unwrap();
        assert_eq!(p.len(), 1, "one save");
        let s: NoteSave = serde_json::from_str(&p[0].payload).unwrap();
        assert_eq!(
            s.base.as_deref(),
            Some("Apple's text"),
            "from what was typed over first"
        );

        lib.outbox_take(NOTES).unwrap();
        record_note_save(
            &mut lib,
            "N",
            "Apple's text, mine more and more",
            None,
            None,
            None,
        )
        .unwrap();
        let p = lib.outbox_pending(NOTES).unwrap();
        let s: NoteSave = serde_json::from_str(&p[1].payload).unwrap();
        assert_eq!(s.base.as_deref(), Some("Apple's text, mine more"));

        // A pass brings Apple's older text: the waiting typing is laid back over it.
        note(&mut lib, "N", "Apple's text");
        replay(&mut lib, NOTES).unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Apple's text, mine more and more")
        );
    }

    /// Contacts: a new card gets its id here, shows at once, and edits before it is sent stay
    /// one create; a pass that does not have it yet keeps it.
    #[test]
    fn a_new_card_shows_at_once_and_stays_one_create() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[person("X", "Old")]).unwrap();
        let guid = record_contact_save(&mut lib, &person("", "Asha"), None).unwrap();
        assert!(!guid.is_empty());
        record_contact_save(
            &mut lib,
            &person(&guid, "Asha Rao"),
            Some(person(&guid, "Asha")),
        )
        .unwrap();
        let p = lib.outbox_pending(CONTACTS).unwrap();
        assert_eq!(p.len(), 1);
        let s: ContactSave = serde_json::from_str(&p[0].payload).unwrap();
        assert!(s.create && s.base.is_none());
        assert_eq!(s.contact.first_name.as_deref(), Some("Asha Rao"));

        // An edit to an existing card keeps the form's base.
        record_contact_save(&mut lib, &person("X", "New"), Some(person("X", "Old"))).unwrap();
        // A pass that read the book before either.
        lib.put_contacts(&[person("X", "Old")]).unwrap();
        replay(&mut lib, CONTACTS).unwrap();
        let names: Vec<(String, Option<String>)> = lib
            .contacts(None)
            .unwrap()
            .into_iter()
            .map(|c| (c.guid, c.first_name))
            .collect();
        assert!(names.contains(&(guid.clone(), Some("Asha Rao".into()))));
        assert!(names.contains(&("X".into(), Some("New".into()))));
    }

    /// Sent twice — the app died after Apple took it, before the queue heard — a change does
    /// what it did once: the create finds its own name and makes nothing more.
    #[test]
    fn a_create_replayed_after_a_crash_is_made_once() {
        tauri::async_runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let made = {
                let mut l = Library::open(dir.path()).unwrap();
                l.put_reminder_lists(&[ReminderList {
                    guid: "L".into(),
                    title: "L".into(),
                    enabled: true,
                    ..Default::default()
                }])
                .unwrap();
                let made = record_reminder_create(
                    &mut l,
                    &Create {
                        list_guid: "L".into(),
                        title: "Once".into(),
                        due_at: None,
                        all_day: None,
                        flagged: None,
                    },
                )
                .unwrap();
                // Taken and sent; the process dies before it is marked done.
                l.outbox_take(REMINDERS).unwrap();
                made
            };
            let lib = Mutex::new(Library::open(dir.path()).unwrap());
            let pass = Mutex::new(());
            // Apple, as a set of names: a create of a name it holds makes nothing.
            let apple = Arc::new(StdMutex::new(std::collections::HashSet::from([
                made.clone()
            ])));
            let creates = Arc::new(StdMutex::new(0));
            let (a2, c2) = (apple.clone(), creates.clone());
            let report = drain(&lib, REMINDERS, &pass, move |p| {
                let (apple, creates) = (a2.clone(), c2.clone());
                async move {
                    if apple.lock().unwrap().insert(p.record.clone()) {
                        *creates.lock().unwrap() += 1;
                    }
                    Ok(())
                }
            })
            .await;
            assert!(report.sent);
            assert_eq!(
                *creates.lock().unwrap(),
                0,
                "already at Apple: not made again"
            );
            assert_eq!(apple.lock().unwrap().len(), 1);
            assert!(lib
                .lock()
                .await
                .outbox_pending(REMINDERS)
                .unwrap()
                .is_empty());
        });
    }

    #[test]
    fn an_edit_round_trips_clear_and_leave() {
        let e = Edit {
            notes: Some(None),
            due_at: Some(Some(5)),
            parent_guid: None,
            from_list: "L".into(),
            ..Default::default()
        };
        let back: Edit = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}
