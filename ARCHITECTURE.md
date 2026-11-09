# Architecture

How Skykeep is put together, in one page. Where each folder is lives in the
[code map](.github/CONTRIBUTING.md#code-map); this page is how a change moves through them.

## The shape

```text
 window (React, src/)          the app (Tauri, src-tauri/src/)            Apple
 ───────────────────           ────────────────────────────────           ─────
 screens read the library  ──▶ commands ──▶ quince-core (SQLite) 
 a write is a command      ──▶ outbox.rs: record + lay over library
                               worker sends, oldest first ──────────▶  iCloud services
                               push.rs / refresh.rs: changes in  ◀────  push, polls
```

Five crates sit under the app:

- **quince-auth** signs in (Apple's SRP and two-factor), keeps the session in the system keyring,
  and builds every HTTP client. `apple_only()` refuses any host outside Apple's domains before a
  connection exists; `listed_only()` reaches only the calendars you subscribed to;
  `account_only()` only the servers of calendar accounts you added;
  `releases::releases_only()` reaches only GitHub's release servers, for Skykeep's own updates
  (`src-tauri/src/updates.rs`).
- **quince-icloud** speaks each service: CloudKit records for Notes, Reminders and Photos,
  Apple's web APIs for Calendar, Contacts and Drive, and the push channel. It parses and writes;
  it stores nothing.
- **quince-core** is the library: one SQLite file, its migrations (`schema.rs`), the full-text
  search index, the outbox queue, and export.
- **quince-vfs** shows Drive and Photos as folders through FUSE (Linux).
- **quince-media** decodes pictures, HEIC included, and makes thumbnails.

## Reading

Every screen reads the library, never Apple. A sync pass (`refresh.rs`) fetches what changed,
stores it, then lays every change still waiting in the outbox back over what it stored, so a
pass never shows an older version over an edit you made. Then the window hears
`quince://synced` and the open screens read again. Push (`push.rs`) decides *when* a pass runs:
Apple's push socket for Notes, Reminders and Photos; short polls for Calendar and Contacts,
slower when no window is open or the computer is on its battery.

## Writing

A write is recorded and shown in one transaction, then sent behind the window:

1. A command (`main.rs`, `contacts_cmds.rs`, …) calls `outbox::record_*`, which appends the
   change to the queue and lays it over the library at once. Every view shows it.
2. The worker in `outbox.rs` sends each service's queue in the order it was made. Taken: off the
   queue, and one pass reads Apple's own version. Unreachable: kept and retried with a growing
   wait, and at once when the network comes back. Refused for good: kept with what was typed,
   and the window shows why, with Try Again and Discard.
3. Every change is safe to send twice: a new record carries the name it was given here, and
   edits are merges against the version they were made from.

Card addresses, reminder ticks, note moves, event edits with their repeat scope: each is its
own kind of change in the queue, with its own `lay` and `send`.

## Calendar accounts other than iCloud

A Nextcloud or CalDAV account (`src-tauri/src/dav.rs`) is read into the same `calendars` and
`events` tables, so every screen, search, alerts and the outbox work as they do for iCloud. Its
guids begin `quince:dav:`. Beside them, `dav_resources` keeps each event's iCalendar text as
the server holds it. A change is made to that text, a line at a time
(`crates/quince-icloud/src/ical_edit.rs`), so nothing Skykeep has no field for is lost, and
sent with `If-Match`. Each account has its own outbox queue, so a server that is down holds up
only its own changes. `caldav.rs` speaks the protocol; `nextcloud.rs` is the browser sign-in.

## Windows and apps

One process, one library. The whole-library window and each app's own window (Notes, Reminders,
Calendar, Contacts, Drive, Photos; `apps.rs`) are webviews over the same commands. A screen
once opened is kept mounted and inert while another is in front, so coming back is instant.

## Where to look first

| To change | Start in |
|---|---|
| What Apple sends or accepts | `crates/quince-icloud/src/<service>.rs` |
| Calendar accounts other than iCloud | `src-tauri/src/dav.rs`, `crates/quince-icloud/src/caldav.rs` |
| What is stored | `crates/quince-core/src/schema.rs`, then the service's file there |
| How a write behaves offline | `src-tauri/src/outbox.rs` |
| When syncing happens | `src-tauri/src/push.rs`, `refresh.rs` |
| Advanced Data Protection approvals | `crates/quince-auth/src/pcs.rs`, `src-tauri/src/data_access.rs` |
| A screen | `src/screens/`, its types in `src/lib/ipc.ts`, its suite in `.uiverify/` |
