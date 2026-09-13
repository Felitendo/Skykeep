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
];
