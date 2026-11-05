# Roadmap: one place for everything open

This file merges all the research and audit docs into one list. Each item names the doc it came from, so the detail is never lost. **Updated 2026-10-21.**

## Decisions already made (don't reopen)
- **No translations** or RTL for now. Locale *formats* (week start, 12/24h) come from the system.
- **Apple-only by default.**
  - Add features Apple's own apps have when users need them, opt-in, with privacy copy that stays literally true.
  - **No third-party calendar accounts** (CalDAV or Google) for now. Only webcal subscriptions, which need no sign-in.
- **Positive, true copy:** no "…" on labels, no rows that carry no value, never imply Apple affiliation.
- **No motion on large layers.** WebKitGTK on Hyprland flickers. The photo viewer opens and closes instantly.
- **No fruit imagery** in brand or logo (Apple v. Prepear, 2020).
- **Calendar invitations / RSVP** wait until after launch.

## Shipped on 2026-10-21 (commits 0f30b3e, 1d2d344 and later)
- **Sync and speed**
  - Concurrent sync; each screen fills as its own service lands.
  - Fast app windows; tray down to four items.
- **Data safety:** offline outbox, atomic applies and migrations, the note-duplication fix, merge fixes.
- **Security:** apple_only covers IPs and redirects; attachments that would run code are never run; tighter capabilities; TLS updated.
- **Windows:** AUMID, reminder chime, toast buttons, HEIC via vcpkg, Edge UI suites in CI.
- **Design:** one design system across all apps (Toast, Confirm, EmptyState, PaneTitle and the rest).
- **Per-app work**
  - Calendar pass; Contacts/Drive pass; Settings with storage.
  - Export Everything; GNOME/KDE search provider; .ics/.vcf import; quick capture.
  - Contacts structured addresses.
  - Reminders: tags, smart lists, sections shown (writes off).
- **Locked notes:** a read-only unlock backend.
- **App ID** is now io.github.yashwanthkamireddi.Pomander, with a crash-safe migration.

## Waiting on the founder
1. **Name.** Shortlist in naming-final-2026-10-21.md, plus the corrected list in chat: Nearline, Pilvi, Skykeep, Lowcloud, Hushfall. The rename is one script; see the app-ID notes in packaging/README.md.
2. **Logos.** Prompts are in the chat history; they need rewriting for the chosen name.
3. **Live tests**, each behind a flag, each needing his phone:
   - **Notes table writes:** notes-tables-crdt.md. One cell in the "Table Test" note, then check the iPhone and a second device.
   - **Note attachments:** `NOTE_ATTACH_WRITES`, on a throwaway note.
   - **Reminders details:** `REMINDER_DETAIL_WRITES`, in both the frontend and `crates/quince-icloud/src/reminders_details.rs`. The section-move checksum is unverified.
4. **Windows retest** of the next installer.
5. **Trademark search and code signing:** only if he wants them; the project is open source.
6. **iCloud cleanup:** delete the broken five-copy note. If doubled text shows in the "Colors" note, recreate its two tables.

## Open engineering, by source
- **gaps-2026-10-21.md** (top 20)
  - **Done:** #1 session lapse, #2 week start, #5 Windows alert buttons, #8 .ics/.vcf open, #10 notification app name, #16 quick capture, #17 export, #20 search provider.
  - **Open:**
    - #3 offline editing for events (calendar outbox);
    - #4 invitations (deferred);
    - #6 a Flatpak build plus portals (the manifest exists but has never been built);
    - #7 Drive/Photos folders on Windows (Cloud Files API);
    - #9 Reminders writes (flag);
    - #11 note attachments (flag);
    - #13 shortcuts (partly done per app);
    - #14 locked notes UI (in progress);
    - #15 contact duplicate merge;
    - #18 accessibility pass;
    - #19 sharing.
- **Security leftovers** (from the security audit): `note_export_markdown`, `drive_upload`, `photos_upload` and `contacts_import` should accept only paths from a dialog or a drop.
- **Durability** (from the data-integrity audit):
  - repeating-reminder ticks, reminder repeat changes and contact delete still need a network;
  - a calendar conflict on a repeating occurrence still errors.
- **Performance** (from the perf audit): page the Photos IPC for libraries over 20k; thumbnails on Windows use the slow custom protocol (measure on real Windows first).
- **Bugs to fix**
  - ~~Contacts Birthday field renders empty in WebKit~~ (fixed 2026-10-22, 1fb59b5).
  - WebKit timing suites: drive-select at 126ms vs 100ms, photos-scale. Recheck when the machine is idle.
- **Release**
  - Install-test deb, rpm, AppImage and AUR.
  - The first tagged release (.github/workflows/release.yml, a draft release).
  - README and GitHub page (marketing-2026-10-21.md §5).
  - Remove the Apple icons from git history once the new icons exist.
- **Product ideas** (apple-parity.md, product-review.md, research-2026-10-21.md): see those docs. Most parity items are done; re-audit after the per-app passes.

## Marketing (marketing-2026-10-21.md)
- **Fix before use:** the taglines must say Linux **and** Windows, and lead with the native Rust engine (instant, local-first, private). The doc's hero line "at home on Linux" is too narrow.
- **Launch hook:** a GIF of ~1s two-way sync, plus the "how we reverse-engineered CloudKit" story. Earlier "iCloud on Linux" HN posts scored 1–6.
- **Claims allowed today:** Windows = beta until tested on real Windows; Photos = preview until tested on a large library.
