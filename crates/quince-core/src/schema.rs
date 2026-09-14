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
];
