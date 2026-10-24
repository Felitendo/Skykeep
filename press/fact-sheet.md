# Skykeep fact sheet

Checked against the code and the README on 2026-10-25 (`main` at `bc275a9`). The lines marked
**[launch day]** change with the release; set them on the day.

## What it is

A free, open-source desktop app that brings iCloud Notes, Reminders, Calendar, Contacts,
iCloud Drive and Photos to Linux and Windows as six apps with one sign-in.

## Made by

Yashwanth Kamireddi, an independent developer. Skykeep is a solo project.

## First release

v0.1.0, **[launch day: the release date]**.

## Platforms

- **Windows 10 and 11.** Installs per user, no administrator password.
- **Linux:** Ubuntu 24.04+, Linux Mint 22, Debian 13 (`.deb`); Fedora 41+, openSUSE Tumbleweed
  (`.rpm`); any other distribution (`.AppImage`); Arch-based systems through a PKGBUILD, and
  through the AUR as `skykeep` **[launch day: once the AUR package is live]**.
- GNOME, KDE and other desktops, on Wayland or X11.

## Price, licence, source

Free. GPL-3.0. Source: https://github.com/YashwanthKamireddi/Skykeep

## Key facts

- **Your library lives on your computer.** Every app opens instantly and works offline. Notes,
  reminders, events and contacts edited offline sync when you reconnect.
- **Live.** Notes and Reminders changes from an iPhone usually arrive in about a second, over
  Apple's push channel. Calendar and Contacts are checked every few seconds while Skykeep is in
  use.
- **Merged, not overwritten.** A note edited on the phone and the computer at the same moment
  comes out with both edits.
- **Six real apps.** Each has its own entry in the Linux app launcher. On Windows, each has its
  own Start-menu entry and taskbar button, plus a jump list (New Note, New Reminder, New Event
  and the six apps).
- **At home on the desktop.** On Linux, iCloud Drive and Photos also appear as folders in the
  file manager (in the Flatpak they open inside the app), and Skykeep answers in
  GNOME and KDE search. Quick capture, a tray icon, and reminder alerts with Complete and
  Snooze.
- **Export Everything:** notes to Markdown, events to iCalendar, contacts to vCard, files as
  they are.
- **Privacy.** No Skykeep account, no Skykeep servers, no analytics, no telemetry, no crash
  reporting. Skykeep connects to Apple for your iCloud, to the calendars you subscribe to, and
  to GitHub's release servers to check for its own updates; each connection is locked to its
  own hosts in code. Sign-in uses Apple's SRP protocol, so the password is never sent, not even
  to Apple. The password and session are kept in the system keyring so Skykeep can sign back in
  by itself.
- **Updates.** The Windows installer and the AppImage update themselves with signed,
  signature-checked updates, never mid-sync or while you type. The `.deb`, `.rpm`, AUR and
  Flatpak copies update through their package manager and never contact GitHub.
- **Advanced Data Protection** works with "Access iCloud Data on the Web" turned on. Managed
  Apple Accounts (school or workplace) aren't supported yet.

## Built with

A Rust engine in five crates, with Tauri 2 windows and a TypeScript (React) interface. On
2026-10-25: 461 commits, about 111,000 lines of Rust and 30,000 lines of TypeScript, and over
1,000 Rust tests. **[launch day: re-run the counts]**

```sh
git rev-list --count HEAD
git ls-files '*.rs' | xargs cat | wc -l
git ls-files 'src/*.ts' 'src/*.tsx' | xargs cat | wc -l
git ls-files '*.rs' | xargs grep -E '#\[(tokio::)?test' | wc -l
```

## The name

*Sky* for the cloud; a *keep* is a castle's safest tower. Your cloud, kept safe on your own
computer.

## Contact

[GitHub Discussions](https://github.com/YashwanthKamireddi/Skykeep/discussions). Security
reports: GitHub private vulnerability reporting, as [SECURITY.md](../SECURITY.md) explains.

---

*Skykeep is an independent project and has not been authorized, sponsored or otherwise
approved by Apple Inc. Apple, iCloud and iPhone are trademarks of Apple Inc., registered in the
U.S. and other countries and regions.*
