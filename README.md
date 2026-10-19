# Quince

Your iCloud, on your own computer.

Quince signs into iCloud once and keeps a copy of your calendar, reminders, notes, contacts,
Drive and photos on your machine. Everything opens straight away and works with no connection, and
what you change here reaches your phone.

It is a native Linux application — Rust underneath, a real window, no browser wrapper.

<!-- A screenshot belongs here once there is a public repo to host it. -->

## What works

Everything here is read from and written to your real iCloud account, shows the moment you
make it — before Apple has answered — and is put back with the reason if Apple refuses.

**Notes** — pictures, scans, PDFs, recordings, films and files shown where they sit and opened
in your own apps; tables shown as the grids they are, kept up to date as they change on your
phone; text coloured on the phone in its colour; edited formatted, as on a Mac: titles,
headings, monospaced text, bulleted, dashed and numbered lists (typed as on the phone: "- ",
"* ", "1. ", "[ ] "), checklists you tick, bold, italic, underline and strikethrough, from the
Aa panel, the checklist button or the Mac's keys. Delete opens the note below, as Notes does. Folders: new, rename, nest,
delete. Pin, move, Recently Deleted and Recover. An edit made on your phone at the same time
is merged in, not overwritten, and nothing either side typed is lost.

**Reminders** — lists with Apple's colours and glyphs, groups, subtasks (Tab, ⌘]), repeats,
due dates and all-day dates, flags, priorities, alerts that ring on this computer, drag to
reorder, Today / Scheduled / Flagged / All, and completing a repeating reminder the way your
phone does.

**Calendar** — Day, Week, Month and Year; make, rename, recolour and delete calendars; Go to
Date (⇧⌃T); make an event by dragging or double-clicking;
move and resize by dragging, or nudge with ⌃⌥ and the arrows; alerts; repeats, with Only
This Event, All Future Events and All Events for one day of a series; your reminders on the
schedule in their list's colour; birthdays and holidays; the Mac's keyboard throughout.

**Contacts** — cards with phones, emails, links, birthdays and notes; lists in a sidebar,
with people added by menu or by dragging; select several to add, remove, export or delete
together; labels from Apple's own list; import many contacts at once from vCard or from a CSV
exported by Google Contacts or Outlook, and export; My Card.

**iCloud Drive** — browse in a list or grid with Apple's own thumbnails; sort; upload files
or whole folders by button or by dropping them on the window, each file's progress shown and
any of them cancellable while it is still on its way up; download, rename, duplicate, Get Info; move by
dialog or by dragging onto a folder; Recents; Recently Deleted with the time each item has
left, Recover and Delete Now; your storage.

**Photos** — your whole library, edge to edge and by month, with a zoom; Favorites, Recents,
Videos, Screenshots, Recently Deleted and your albums; a viewer with the arrow keys, details,
Favorite (.), Delete to Recently Deleted and Recover, and the original downloaded. Videos open
in your own player. Pictures are fetched once and kept, so the library scrolls from disk.

**Instant, both ways.** A change made on your phone reaches Quince in about a second: Notes,
Reminders and Photos through Apple's own push channel, Calendar and Contacts through the same quick
"has anything changed" checks icloud.com makes, every few seconds while the window is open.
What you change shows the moment you make it and is put back, with the reason, only if Apple
refuses it — and a refresh landing mid-change never undoes what is still on its way.

**Settings** — your account and its picture from your own card, iCloud storage, when each app
last synced with Sync Now, Automatic / Light / Dark, and switches for reminder and calendar
alerts that the alerts really obey.

**Designed as macOS 27 is.** Liquid Glass on the chrome — the sidebar, toolbars, menus,
popovers and sheets — and never on your content; capsule controls; motion on Apple's own
springs; each app in its own Mac colour, Notes yellow, Calendar red, the rest blue; light and
dark, Reduce Transparency and Increase Contrast honoured.

**Advanced Data Protection.** With it on and "Access iCloud Data on the Web" off, no app
outside your devices can read your iCloud. Quince says exactly that, once, and names Apple's
switch — on your iPhone, Settings, your name, iCloud, Access iCloud Data on the Web — rather
than failing screen by screen.

Not yet: Mail, Find My, calendar invitations, uploading photos. Notes tables are read-only for
now: edits made here were stored by Apple but did not show on an iPhone, so no table is changed
until one is seen there.

## Installing

**Arch, CachyOS, EndeavourOS, Manjaro**

```sh
cd packaging/arch && makepkg -si
```

**Running it from the source**

```sh
./run.sh
```

builds the interface and the app and opens Quince.

**Anything else**

```sh
npm ci && npx tauri build
```

leaves a `.deb`, an `.rpm` and an AppImage under `target/release/bundle/`. The AppImage needs
no package manager.

Quince needs `webkit2gtk-4.1` for its window and `libsecret` for the keyring. It will start
without the second and will not be able to stay signed in, which is most of the point of it.

Flatpak is not done yet — see `packaging/README.md` for what it needs and why it has not been
guessed at.

## Building and checking

```sh
./check.sh
```

Formatting, lints, the Rust tests, TypeScript, the bundle, and then the built application
driven in a real browser: every screen screenshotted and measured, the keyboard path walked, motion sampled frame by
frame (a fold that jumps, a list that flickers, a menu that blinks fails the build),
accessibility audited, and every screen opened again as a German speaker in New York, a
Japanese speaker in Tokyo and a New Zealander on the Chathams — because this was written in
one place with a clock that never moves, and most people are somewhere else.

Nothing lands with that red.

There are also checks that talk to a real iCloud account, which is why they are not in
`check.sh`. They live in `probe/examples` and are run deliberately:

```sh
cargo run -p quince-probe --example notes_audit
```

Each one asks the same question of a different service — not "did the write return 200" but
"do the account and the local copy agree, right now, about real data".

## The honest part

**Quince is not made by Apple, endorsed by Apple, or affiliated with Apple.** It reaches
iCloud through interfaces Apple has not published and does not support. Apple can change them
without notice, and when that happens something here will stop working until it is put right.

**Your credentials never leave your computer.** Quince proves you know your password without
sending it, and keeps the result in your system keyring. There is no Quince account, no Quince
server, and nowhere else for any of this to go. A hosted version of this would be a bad idea
and will not be built.

**Some things cannot be read by any client.** Health, Messages, Keychain, Home, Safari history
and Screen Time are encrypted end to end — Apple's own servers hold no key to them. Quince
shows them as unavailable rather than quietly leaving them out.

**Advanced Data Protection.** With it switched on and web access off, iCloud's data is
encrypted the same way and Quince cannot read it either. It says so, and how web access is
turned on, rather than nudging anybody to turn the security feature itself off.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE).
