//! Schema migrations, applied in order and tracked by SQLite's `user_version`.
//!
//! Append only. Editing a shipped migration would leave existing installs on a schema that
//! no longer matches what the code expects.

/// Every migration, oldest first. The index plus one is the `user_version` it produces.
pub const MIGRATIONS: &[&str] = &[
    // 1 — calendars and events.
    r#"
    CREATE TABLE calendars (
        guid        TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        colour      TEXT,
        -- Apple's collection tag. Unchanged ctag means nothing in this calendar moved, so
        -- the whole collection can be skipped on the next sync.
        ctag        TEXT,
        sort_order  INTEGER NOT NULL DEFAULT 0,
        enabled     INTEGER NOT NULL DEFAULT 1,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE events (
        guid          TEXT PRIMARY KEY,
        calendar_guid TEXT NOT NULL REFERENCES calendars(guid) ON DELETE CASCADE,
        title         TEXT NOT NULL,
        location      TEXT,
        -- Unix seconds, UTC. The originating zone is kept separately so an all-day event or
        -- a meeting booked in another timezone still renders the way Apple shows it.
        starts_at     INTEGER NOT NULL,
        ends_at       INTEGER NOT NULL,
        all_day       INTEGER NOT NULL DEFAULT 0,
        timezone      TEXT,
        url           TEXT,
        notes         TEXT,
        etag          TEXT,
        updated_at    INTEGER NOT NULL
    ) STRICT;

    -- The calendar view is always "what is happening between these two instants".
    CREATE INDEX events_by_start ON events (starts_at);
    CREATE INDEX events_by_calendar ON events (calendar_guid, starts_at);

    -- Where each service got to, so a sync resumes instead of refetching everything.
    CREATE TABLE sync_state (
        service        TEXT PRIMARY KEY,
        token          TEXT,
        last_synced_at INTEGER,
        last_error     TEXT
    ) STRICT;
    "#,
    // 2 — reminder lists and reminders.
    r#"
    CREATE TABLE reminder_lists (
        guid       TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        colour     TEXT,
        ctag       TEXT,
        sort_order INTEGER NOT NULL DEFAULT 0,
        enabled    INTEGER NOT NULL DEFAULT 1,
        updated_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE reminders (
        guid         TEXT PRIMARY KEY,
        list_guid    TEXT NOT NULL REFERENCES reminder_lists(guid) ON DELETE CASCADE,
        title        TEXT NOT NULL,
        notes        TEXT,
        -- Unix seconds, UTC. Null means no due date, which is the common case.
        due_at       INTEGER,
        -- Null while outstanding; the completion instant once ticked off.
        completed_at INTEGER,
        priority     INTEGER NOT NULL DEFAULT 0,
        etag         TEXT,
        updated_at   INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX reminders_by_list ON reminders (list_guid, completed_at, due_at);
    CREATE INDEX reminders_by_due ON reminders (due_at);
    "#,
    // 3 — keep Apple's original payload for each reminder.
    //
    // Writes must echo the whole task back. Reconstructing one from parsed columns would
    // silently drop fields Apple sent but we do not model, which is how a sync corrupts
    // someone's data. Storing the original makes a write a mutation of what Apple gave us.
    r#"
    ALTER TABLE reminders ADD COLUMN raw TEXT;
    "#,
    // 4 — notes and their folders.
    r#"
    CREATE TABLE note_folders (
        guid       TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE notes (
        guid          TEXT PRIMARY KEY,
        folder_guid   TEXT REFERENCES note_folders(guid) ON DELETE SET NULL,
        title         TEXT NOT NULL,
        -- The first line or so, which Apple sends separately so a list can render without
        -- decoding every body.
        snippet       TEXT,
        body          TEXT,
        -- A note the owner locked is genuinely encrypted; we can show that it exists and
        -- nothing more, so say so rather than rendering it blank.
        locked        INTEGER NOT NULL DEFAULT 0,
        pinned        INTEGER NOT NULL DEFAULT 0,
        created_at    INTEGER,
        modified_at   INTEGER,
        etag          TEXT,
        updated_at    INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX notes_by_folder ON notes (folder_guid, pinned DESC, modified_at DESC);
    CREATE INDEX notes_by_modified ON notes (modified_at DESC);
    "#,
    // 5 — contacts, with their phone numbers and addresses normalised alongside.
    r#"
    CREATE TABLE contacts (
        guid        TEXT PRIMARY KEY,
        first_name  TEXT,
        last_name   TEXT,
        company     TEXT,
        is_company  INTEGER NOT NULL DEFAULT 0,
        -- What to sort and search on, so neither depends on which name fields exist.
        sort_name   TEXT NOT NULL,
        birthday    TEXT,
        notes       TEXT,
        etag        TEXT,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX contacts_by_name ON contacts (sort_name);

    -- Phones, emails and the rest share a shape, so they share a table.
    CREATE TABLE contact_methods (
        contact_guid TEXT NOT NULL REFERENCES contacts(guid) ON DELETE CASCADE,
        kind         TEXT NOT NULL,
        label        TEXT,
        value        TEXT NOT NULL,
        position     INTEGER NOT NULL DEFAULT 0
    ) STRICT;

    CREATE INDEX contact_methods_by_contact ON contact_methods (contact_guid, kind, position);
    "#,
    // 6 — iCloud Drive, stored as the tree Apple exposes.
    r#"
    CREATE TABLE drive_items (
        -- Apple's own identifier, which encodes the kind and zone: FOLDER::com.apple.CloudDocs::root
        drivewsid   TEXT PRIMARY KEY,
        parent_id   TEXT,
        name        TEXT NOT NULL,
        extension   TEXT,
        -- FOLDER, APP_LIBRARY or FILE.
        kind        TEXT NOT NULL,
        size        INTEGER,
        child_count INTEGER,
        created_at  INTEGER,
        modified_at INTEGER,
        etag        TEXT,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    -- Browsing is always "what is inside this folder", ordered folders first.
    CREATE INDEX drive_by_parent ON drive_items (parent_id, kind, name);
    "#,
    // 7 — flagging, which Apple surfaces as one of its smart lists.
    r#"
    ALTER TABLE reminders ADD COLUMN flagged INTEGER NOT NULL DEFAULT 0;
    CREATE INDEX reminders_flagged ON reminders (flagged, completed_at);
    "#,
    // 8 — recurrence, so completing a repeating reminder can advance it.
    //
    // Ticking a repeating reminder does not mean "set completed" — Apple already holds it
    // completed while the record points at the next occurrence. It means "move to the one
    // after", which needs the rule.
    r#"
    ALTER TABLE reminders ADD COLUMN recur_frequency INTEGER;
    ALTER TABLE reminders ADD COLUMN recur_interval INTEGER;
    "#,
    // 9 — whether a note can be saved without damage.
    //
    // A note body is Apple's document protobuf: text surrounded by version vectors, folder
    // links and merge state. Saving means rewriting that document in place. Where the
    // document cannot be reproduced byte for byte, this app will not write it, and the
    // interface has to be able to say so before someone starts typing rather than after.
    r#"
    ALTER TABLE notes ADD COLUMN editable INTEGER NOT NULL DEFAULT 1;
    "#,
    // 10 — when a service last did a full pass, rather than a delta.
    //
    // A delta is cheap and is what almost every refresh should be, but it can only ever
    // apply what Apple chose to send. A tombstone is delivered once; a sync that consumed
    // one before the code knew what to do with it left that record stranded forever. A full
    // pass now and then is the floor under that, and this is what schedules it.
    r#"
    ALTER TABLE sync_state ADD COLUMN last_full_at INTEGER;
    "#,
    // 11 — subtasks.
    //
    // Apple nests a reminder under another with a ParentReminder reference. Without it,
    // every subtask showed as a sibling of the thing it belongs to, which reads as a flat
    // list of fragments rather than as one piece of work broken down.
    r#"
    ALTER TABLE reminders ADD COLUMN parent_guid TEXT;
    CREATE INDEX reminders_by_parent ON reminders (parent_guid);
    "#,
    // 12 — a contact's middle name.
    //
    // Apple keeps one and four people in a real address book have one; without it they were
    // shown under a name that was not quite theirs.
    r#"
    ALTER TABLE contacts ADD COLUMN middle_name TEXT;
    "#,
    // 13 — where a contact's photo lives.
    //
    // The URL is Apple's; the signature changes when the picture does, which is what tells
    // a cached copy it has gone stale. The bytes themselves are not kept here: an image
    // belongs on disk where the webview can load it directly, not in a row that has to
    // cross an IPC boundary every time a list scrolls.
    r#"
    ALTER TABLE contacts ADD COLUMN photo_url TEXT;
    ALTER TABLE contacts ADD COLUMN photo_signature TEXT;
    "#,
    // 14 — the square of a picture its owner chose to show.
    //
    // Apple stores the photo whole and records the crop beside it. One picture on a real
    // account is 1696x2528 with a crop of 1279x1279 at 251,1249 — a portrait whose face sits
    // near the bottom. Centring it, which is what any viewer does without this, shows a chest.
    // Kept as `x,y,width,height` because it is only ever read and written together.
    r#"
    ALTER TABLE contacts ADD COLUMN photo_crop TEXT;
    "#,
    // 15 — what Apple says about each calendar.
    //
    // A subscribed calendar — a holiday feed, a sports fixture list — and a calendar shared
    // with you for reading only both come back with readOnly set. Offering to put an event in
    // one produces a refusal that reads as a bug in this app rather than as the truth.
    //
    // And Apple names the default calendar outright. Picking the first of the list instead is
    // the same mistake that put every new reminder in whichever list happened to sort first.
    r#"
    ALTER TABLE calendars ADD COLUMN writable INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE calendars ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
    "#,
    // 16 — a notes folder can sit inside another one.
    //
    // Apple Notes has nested folders and keeps the link on the folder record as
    // `ParentFolder`. This account has only the two Apple makes for itself, neither of which
    // is nested, so the field was never seen — but a folder created with one is accepted and
    // handed straight back, which is how it was confirmed. Flattened, somebody's filing comes
    // out as one long list with every level jumbled together.
    r#"
    ALTER TABLE note_folders ADD COLUMN parent_guid TEXT;
    "#,
    // 17 — where a reminder's time is kept.
    //
    // Apple records a named zone beside the due date, because nine in the morning means nine
    // in the morning wherever the person is rather than a fixed instant. Without it a
    // repeating reminder was stepped in UTC, where a day is always exactly twenty-four hours —
    // so on the two days a year a clock moves, a daily reminder at nine moved to ten and six
    // months later back again. This account is in one of the few zones that keeps a single
    // offset all year, which is why it was never visible here.
    r#"
    ALTER TABLE reminders ADD COLUMN timezone TEXT;
    "#,
    // 18 — somewhere to keep a thing learned from the account.
    //
    // The first of these is a note document Apple wrote, kept so a new note can be given the
    // same shape. It cannot be fetched on demand: notes sit far enough into the zone walk that
    // the first page of a hundred and thirty-nine records held none of them, and walking the
    // whole zone to write one note is not a trade worth making. Sync already reads every note,
    // so sync is where it is noticed.
    r#"
    CREATE TABLE app_state (
      key   TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
    "#,
    // 19 — sections inside a reminders list.
    //
    // A section is its own record, but which reminder sits in which one is not on the
    // reminder: it is a file hung off the list, a JSON array of { memberID, groupID } where
    // groupID is the section. A reminder with no groupID sits outside every section.
    r#"
    CREATE TABLE reminder_sections (
        guid        TEXT PRIMARY KEY,
        list_guid   TEXT NOT NULL,
        title       TEXT NOT NULL,
        created_at  INTEGER,
        updated_at  INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX reminder_sections_by_list ON reminder_sections (list_guid, created_at);
    ALTER TABLE reminders ADD COLUMN section_guid TEXT;
    "#,
    // 20 — what a reminders list looks like and where it sits.
    //
    // Apple keeps a list's glyph (`BadgeEmblem`), the group it is filed under (`ParentList`),
    // whether it is pinned beside Today and Scheduled (`PinnedDate`) and how its reminders
    // are ordered (`SortingStyle`). None of it was read, so every list drew as the same white
    // dot in one flat column. Groups are folders of lists, kept apart from the lists
    // themselves so nothing that counts or offers lists can mistake one for a list.
    r#"
    ALTER TABLE reminder_lists ADD COLUMN emblem TEXT;
    ALTER TABLE reminder_lists ADD COLUMN group_guid TEXT;
    ALTER TABLE reminder_lists ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE reminder_lists ADD COLUMN sorting TEXT;
    CREATE TABLE reminder_groups (
        guid        TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        sort_order  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    ) STRICT;
    "#,
    // 21 — the order a list keeps its reminders in.
    //
    // A list set to manual order holds it as `ReminderIDs`, every reminder in it by id in the
    // order they were dragged into. Ignored, every list came out sorted by due date, which is
    // only right for the lists set to sort that way.
    r#"
    ALTER TABLE reminder_lists ADD COLUMN reminder_order TEXT;
    "#,
    // 22 — read every reminders list again, once.
    //
    // 20 and 21 added what a list looks like and how it orders its reminders, but a library
    // made before them only ever asks Apple what changed since its last pass — and a list
    // nobody has touched never changes, so its new columns stayed empty and it kept drawing
    // as a white dot with its colour stored as raw JSON. Forgetting the position makes the
    // next pass a whole one, which fills them in.
    r#"
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // 23 — reminder alarms, so this app can ring when the phone would.
    //
    // A reminder's alert is not on the reminder: it lists `AlarmIDs`, each an `Alarm` record
    // naming an `AlarmTrigger`, which holds the wall-clock moment to fire. The two are kept as
    // Apple has them because either can change without the other in a pass. `alarm_fired`
    // remembers every alert already shown — by alarm and moment, so an alarm moved to a new
    // time rings again and one already shown never does twice, across restarts. Snoozes are
    // this device's own and never go to Apple. The position is forgotten once more so the
    // next pass reads every alarm already on the account.
    r#"
    CREATE TABLE reminder_alarms (
        alarm_id        TEXT PRIMARY KEY,
        reminder_guid   TEXT NOT NULL,
        trigger_id      TEXT NOT NULL,
        acknowledged_at INTEGER
    ) STRICT;
    CREATE INDEX reminder_alarms_by_trigger ON reminder_alarms (trigger_id);
    CREATE TABLE alarm_triggers (
        trigger_id TEXT PRIMARY KEY,
        fires_at   INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX alarm_triggers_by_time ON alarm_triggers (fires_at);
    CREATE TABLE alarm_fired (
        alarm_id TEXT NOT NULL,
        fires_at INTEGER NOT NULL,
        fired_at INTEGER NOT NULL,
        PRIMARY KEY (alarm_id, fires_at)
    ) STRICT;
    CREATE TABLE alarm_snoozes (
        reminder_guid TEXT PRIMARY KEY,
        fires_at      INTEGER NOT NULL
    ) STRICT;
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // All-day reminders: Apple's `AllDay`, so a reminder due on a day shows as that day and
    // not as midnight. The position is forgotten so the next pass reads it on every reminder.
    r#"
    ALTER TABLE reminders ADD COLUMN all_day INTEGER NOT NULL DEFAULT 0;
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // A note's formatting — Apple's attribute runs — kept beside its body, and the notes zone
    // read again from the start so every note has them.
    r#"
    ALTER TABLE notes ADD COLUMN runs TEXT;
    UPDATE sync_state SET token = NULL WHERE service = 'notes';
    "#,
    // iCloud Photos: every photo's description, mirrored through the change feed, so each
    // view is a query here. The images are cached on disk, not in here.
    r#"
    CREATE TABLE photos (
        id         TEXT PRIMARY KEY,
        master     TEXT NOT NULL,
        taken_at   INTEGER NOT NULL,
        added_at   INTEGER,
        kind       TEXT NOT NULL,
        width      INTEGER NOT NULL,
        height     INTEGER NOT NULL,
        duration   REAL,
        filename   TEXT,
        size       INTEGER,
        uti        TEXT,
        favorite   INTEGER NOT NULL DEFAULT 0,
        hidden     INTEGER NOT NULL DEFAULT 0,
        deleted    INTEGER NOT NULL DEFAULT 0,
        deleted_at INTEGER,
        subtype    INTEGER NOT NULL DEFAULT 0,
        change_tag TEXT
    ) STRICT;
    CREATE INDEX photos_by_taken ON photos (deleted, hidden, taken_at DESC);
    CREATE TABLE photo_albums (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        position   INTEGER NOT NULL DEFAULT 0,
        change_tag TEXT
    ) STRICT;
    CREATE TABLE photo_album_items (
        album    TEXT NOT NULL,
        asset    TEXT NOT NULL,
        position INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (album, asset)
    ) STRICT;
    "#,
    // Photos uploads, kept until each is done: quitting halfway resumes where it stopped, and
    // a file is never thought sent until Apple has made the photo.
    r#"
    CREATE TABLE photo_uploads (
        id       TEXT PRIMARY KEY,
        batch    TEXT NOT NULL,
        path     TEXT NOT NULL,
        name     TEXT NOT NULL,
        state    TEXT NOT NULL,
        error    TEXT,
        asset    TEXT,
        added_at INTEGER NOT NULL
    ) STRICT;
    "#,
    // Which photo is in which album, for a library made before the Photos migration carried
    // it: that one had already run there, so the table never came. Photos is read again
    // from the start so every album fills.
    r#"
    CREATE TABLE IF NOT EXISTS photo_album_items (
        album    TEXT NOT NULL,
        asset    TEXT NOT NULL,
        position INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (album, asset)
    ) STRICT;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
];
