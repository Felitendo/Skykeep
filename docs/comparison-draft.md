<!--
DRAFT, not published. Written 2026-11-03.
Every number below was checked on 2026-11-03 (GitHub API, Snapcraft API, Microsoft Store
listing, Apple Support, project READMEs). Re-check stars, dates and versions before publishing.
-->

# Every way to use iCloud on Linux and Windows

You have an iPhone. Your computer runs Linux or Windows. You want your notes, reminders,
calendar, contacts, files and photos on both.

There are several ways to do that today. Some are from Apple, some are open-source projects,
and some are standard protocols that many apps speak. Each one is good at something. This page
lists them side by side, says what each covers, and links to the source for every claim, so you
can pick what fits you.

Skykeep is one of the options. I make it, so I've tried to be fair to everyone else and plain
about what Skykeep does and doesn't cover. If something here is out of date or wrong, please
[open an issue](https://github.com/YashwanthKamireddi/Skykeep/issues).

## At a glance

| Option | What it covers | Platforms | Offline | Notes |
|---|---|---|---|---|
| [iCloud for Windows](#icloud-for-windows) (Apple) | Photos, iCloud Drive, Passwords; Calendars and Contacts through classic Outlook; Bookmarks | Windows 11 | Drive files and photos you choose to keep on the device | From Apple. The Store listing doesn't list Notes or Reminders |
| [icloud.com in a browser](#icloudcom-in-a-browser) (Apple) | Photos, Drive, Notes, Mail, Calendar, Contacts, Reminders, Pages, Numbers, Keynote, Find My and more | Any computer with a supported browser | No | From Apple. Apple names Firefox, Chrome, Edge and Opera on Windows |
| [Web app wrappers](#web-app-wrappers) | Whatever icloud.com offers, in its own window | Linux or Windows, per project | No | You sign in on Apple's own page |
| [icloud-for-omarchy and its sibling apps](#icloud-for-omarchy-and-its-sibling-apps) | Notes, Photos, Find My; Calendar, Contacts, Reminders and Mail in sibling apps | Arch Linux and Omarchy | Notes are Markdown files on disk | Sign-in on Apple's own page; CLI with JSON output |
| [icloud-drive-for-linux](#icloud-drive-for-linux) | iCloud Drive | Linux | Files in folders you choose are saved to disk | Checks for changes about every five minutes |
| [icloud-linux](#icloud-linux) | iCloud Drive as a FUSE folder | Linux | Local cache of names and files | Command-line setup |
| [icloudpd](#icloudpd) | Downloads iCloud Photos | Linux, Windows, macOS, NAS | Downloads are ordinary files | Command line. Runs well unattended |
| [rclone iCloud backend](#rclone-icloud-backend) | iCloud Drive; iCloud Photos read-only | Linux, Windows, macOS | Whatever you copy or sync | Command line. Marked experimental by rclone |
| [CalDAV and CardDAV clients](#caldav-and-carddav-clients) | Calendars and contacts | Linux, Windows, macOS | Depends on the app | App-specific password. Reminders created in the iOS 13 format aren't on CalDAV |
| [Skykeep](#skykeep) | Notes, Reminders, Calendar, Contacts, iCloud Drive, Photos | Linux and Windows 10/11 | Yes: the library is on your disk | iCloud Drive and Photos as folders on Linux |

## Each option

### iCloud for Windows

Apple's own app, from the Microsoft Store. Its listing describes iCloud Photos in the Microsoft
Photos app, iCloud Drive in File Explorer, iCloud Passwords with extensions for Chrome, Edge and
Firefox, iCloud calendars and contacts "with classic Outlook", and Safari bookmarks in Chrome,
Edge or Firefox. The latest version needs Windows 11 or later and Outlook 2021 or later. Apple
says it doesn't support Managed Apple Accounts. You can pin Drive files and photos so they stay
downloaded.

**Good for:** Windows users who want iCloud Drive in File Explorer, photos in the Photos app and
their iCloud passwords in the browser, from Apple itself.

Sources: [Microsoft Store listing](https://apps.microsoft.com/detail/9PKTQ5699M62),
[Apple: iCloud for Windows requirements](https://support.apple.com/en-us/103232),
[Apple: keep iCloud Drive files downloaded](https://support.apple.com/guide/icloud-windows/keep-files-downloaded-icw8531ad6b7/icloud)

### icloud.com in a browser

Apple's web apps. Signed in on a Windows PC, you get Photos, iCloud Drive, Notes, Mail,
Calendars, Contacts, Reminders, Pages, Numbers, Keynote and Find My, among others. On Windows,
Apple recommends the latest Firefox, Chrome, Edge or Opera. It needs a
connection. With Advanced Data Protection on, access to your data on icloud.com is off by
default, and you can turn it on.

**Good for:** everyone, on any computer, with nothing to install. It also includes Mail, Find My,
and Pages, Numbers and Keynote.

Sources: [Apple: what you can do on iCloud.com](https://support.apple.com/en-us/105119),
[Apple: Advanced Data Protection](https://support.apple.com/en-us/102651)

### Web app wrappers

Small open-source apps that open icloud.com in a window of its own, so it gets a launcher entry
and sometimes a tray icon. You sign in on Apple's own page inside the window. A few examples:

- [cross-platform/icloud-for-linux](https://github.com/cross-platform/icloud-for-linux): a Snap
  for Linux, about 578 stars. Version 0.26 is in the stable channel
  ([Snapcraft](https://snapcraft.io/icloud-for-linux)); the repository was last updated in
  October 2025.
- [cgillinger/icloud-electron](https://github.com/cgillinger/icloud-electron): Linux. Version 2
  moved from Electron to a bundled, self-updating Chromium window, and its README says "Sign in
  with iPhone" and passkeys work. It says plainly that it has no file sync.
- [taylorivanoff/icloud-notes-windows](https://github.com/taylorivanoff/icloud-notes-windows):
  icloud.com/notes in a Tauri window on Windows, with a tray icon and start with Windows.
- [Swe-HimelRana/icloud-notes](https://github.com/Swe-HimelRana/icloud-notes): iCloud Notes on
  Linux as a Snap or `.deb`, built on Electron.

**Good for:** people who like icloud.com and want it in its own window, with Apple's sign-in
page doing the sign-in.

### icloud-for-omarchy and its sibling apps

[ferdousbhai/icloud-for-omarchy](https://github.com/ferdousbhai/icloud-for-omarchy) is a set of
apps for Omarchy and Arch Linux that share one iCloud sign-in. Notes keeps your Apple Notes as a
folder of Markdown files, synced both ways, so you can edit them in the app, in Neovim or in
Obsidian. Photos lets you browse, download, upload and delete. Find My shows your devices on a
map. Every action also works from the command line, with JSON output. You sign in on Apple's own
page, and the apps never see your password. Packages come from a signed pacman repository.

The same author has sibling apps that reuse that sign-in:
[Cloud Calendar](https://github.com/ferdousbhai/cloud-calendar) (iCloud, Google and HEY
calendars), [Cloud Contacts](https://github.com/ferdousbhai/cloud-contacts) (iCloud and Google),
[Cloud Tasks](https://github.com/ferdousbhai/cloud-tasks) (iCloud Reminders and Google Tasks) and
[Cloudmail](https://github.com/ferdousbhai/cloud-mail). These were created in late September and
October 2026.

**Good for:** Arch and Omarchy users who want their notes as plain Markdown files, scriptable
iCloud from the terminal, or Find My on the desktop.

### icloud-drive-for-linux

[taylorhayis/icloud-drive-for-linux](https://github.com/taylorhayis/icloud-drive-for-linux) is a
Linux client for iCloud Drive, created in October 2026. You choose which folders to keep, and
their files are saved into `~/iCloud Drive`, an ordinary folder that stays when you quit. It has
a GTK window, a panel icon, an Activity tab and a conflict screen. Deleting from iCloud is off
until you turn it on. Its README says iCloud doesn't push changes to it, so it checks about every
five minutes.

**Good for:** people who want chosen iCloud Drive folders as ordinary files on disk, with a
careful approach to deletes and conflicts.

### icloud-linux

[IsmaeelAkram/icloud-linux](https://github.com/IsmaeelAkram/icloud-linux) mounts iCloud Drive as
a FUSE folder, with a local cache of names and file contents, so browsing and `find` work
against the disk. If a file changed on both sides, your copy is kept as a conflict copy. Remote
changes come in on a timer or when you run `icloudctl sync`. Setup is a few commands in a
terminal. About 159 stars.

**Good for:** people comfortable in a terminal who want iCloud Drive mounted as a folder, with
control over which folders are downloaded.

### icloudpd

[icloud_photos_downloader](https://github.com/icloud-photos-downloader/icloud_photos_downloader)
is a command-line tool that downloads your iCloud Photos. It runs on Linux, Windows, macOS and
NAS boxes, and can keep watching for new photos. It has existed since 2016 and has about 12,370
stars. Its README currently says the project is looking for a maintainer.

**Good for:** backing up a whole photo library to your own disk or NAS, unattended.

### rclone iCloud backend

[rclone](https://rclone.org/iclouddrive/) added an iCloud backend in v1.69. It covers iCloud
Drive, and iCloud Photos read-only. rclone lists it as Tier 4: "Experimental: Use with care;
expect gaps/changes." It works with Advanced Data Protection. It needs your Apple Account
password (app-specific passwords are not accepted), and the trust token lasts 30 days before you
sign in again.

**Good for:** people who already use rclone and want to copy, sync or mount iCloud Drive from
scripts, next to their other clouds.

### CalDAV and CardDAV clients

iCloud calendars and contacts speak the standard CalDAV and CardDAV protocols. Thunderbird,
Evolution and GNOME (through Evolution or GNOME Online Accounts) can connect to them with an
app-specific password. Apple made app-specific passwords so that "your Apple Account password
can't be stored or collected by the app." This route doesn't cover Notes, iCloud Drive or
Photos. Reminders upgraded to the format introduced in iOS 13 no longer use CalDAV.

**Good for:** keeping iCloud calendars and contacts in the mail and calendar app you already
use, without giving that app your Apple Account password.

Sources: [Apple: app-specific passwords](https://support.apple.com/en-us/102654),
[BusyCal: Reminders and CalDAV](https://www.busymac.com/docs/faqs/112990-reminders-in-ios-13-and-macos-catalina-drops-support-for-caldav),
[Mozilla Support: CardDAV in Thunderbird](https://support.mozilla.org/en-US/questions/1347462),
[GNOME Discourse: iCloud in GNOME Settings](https://discourse.gnome.org/t/gnome-settings-carddav-caldav-icloud-account/22859)

### Skykeep

[Skykeep](https://github.com/YashwanthKamireddi/Skykeep) gives you Notes, Reminders, Calendar,
Contacts, iCloud Drive and Photos as six real desktop apps on Linux and Windows 10/11, with one
sign-in, built on a native Rust engine. Your library is kept on your computer, so it opens straight away and works offline:
notes, reminders, events and contacts can be edited offline and sync when you reconnect.

- Notes and Reminders changes from your iPhone usually arrive in about a second, over Apple's
  push channel. Calendar and Contacts are checked every few seconds while you use Skykeep.
- A note edited on both sides at once is merged, not overwritten.
- On Linux, iCloud Drive and Photos are also real folders in your file manager (FUSE).
- Desktop alerts with Complete and Snooze, quick capture, a tray icon, and results in GNOME and
  KDE search.
- On Windows, each app has its own Start-menu entry and taskbar button, plus a jump list.
- Export Everything saves notes as Markdown, events as iCalendar, contacts as vCard, and your
  files as they are.
- It connects to Apple for your iCloud, to the calendars you subscribe to, and to GitHub's
  release servers for its own updates (AppImage and Windows copies, and you can turn that off).
  No Skykeep account, no Skykeep servers, no analytics, no telemetry, no crash reporting.
- Free and open source under the GPL-3.0. A native Rust engine, with Tauri 2 for the windows.

**Good for:** people who want all six iCloud apps on Linux or Windows, kept on the computer and
usable offline.

Sources: [README](https://github.com/YashwanthKamireddi/Skykeep#readme),
[SECURITY.md](https://github.com/YashwanthKamireddi/Skykeep/blob/main/SECURITY.md)

## Good to know before you install

- **Your Apple Account password.** Notes, Reminders, Drive and Photos need a full sign-in
  with two-factor, so Skykeep asks for your Apple Account password, not an app-specific
  password. It never sends the password, not even to Apple (Apple's SRP sign-in). It keeps your
  password and session in your system keyring so it can sign back in by itself. Like any keyring
  entry, programs running as you can read it. Signing out erases it.
- **Advanced Data Protection** works with **Access iCloud Data on the Web** turned on. With it
  off, Apple doesn't let apps outside your devices read your iCloud data, Skykeep included.
- **Managed Apple Accounts** from a school or workplace aren't supported yet. Personal Apple
  Accounts work.
- **Shared notes, shared calendars and replying to calendar invitations** are on the roadmap.
- **iCloud Drive and Photos as folders are a Linux feature.** On Windows, and in the Flatpak, they open
  inside the app.
- **The Windows installer isn't code-signed yet.** Windows SmartScreen asks you to confirm (More
  info, then Run anyway). Releases are built by the repository's release workflow, the release
  page lists SHA-256 checksums, and in-app updates are signature-checked.
- **The library on disk isn't encrypted by Skykeep.** Use full-disk encryption, as you would on
  any laptop.
- **Linux needs a recent system:** WebKitGTK 4.1, a keyring (GNOME Keyring or KWallet), glibc
  2.39 and libheif 1.17 or newer, for example Ubuntu 24.04, Debian 13 or Fedora 41. AUR and
  Flathub packages are coming; for now, Arch users build from the PKGBUILD.
- **It covers six iCloud apps.** It doesn't do Mail, Find My, Passwords, or Pages, Numbers and
  Keynote. For those, icloud.com and the other options above may suit you better.

## Picking one

- Want Mail, Find My, Pages and the rest, with nothing to install? Use **icloud.com**.
- On Windows 11 and want Drive in File Explorer and your passwords in the browser? Use **iCloud
  for Windows**.
- Want iCloud in its own window, signing in on Apple's page? Try a **web app wrapper**.
- On Arch or Omarchy and like Markdown and the terminal? Try **icloud-for-omarchy**.
- Want iCloud Drive as files on disk? Try **icloud-drive-for-linux**, **icloud-linux** or
  **rclone**.
- Want to back up your photos? Use **icloudpd**.
- Want calendars and contacts in Thunderbird or Evolution? Use **CalDAV and CardDAV** with an
  app-specific password.
- Want Notes, Reminders, Calendar, Contacts, Drive and Photos as desktop apps that work offline?
  Try **Skykeep**.

Many of these work well together. For example, you can use icloudpd for photo backups next to any
of the others.

---

Skykeep is an independent, open-source project. It is not affiliated with or endorsed by Apple
Inc. Apple, iCloud and iPhone are trademarks of Apple Inc. The other projects named here are
their authors' own work; star counts and dates were checked on 3 November 2026.
