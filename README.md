<div align="center">

<img src="packaging/icons/skykeep-1024.png" alt="Skykeep" width="128">

# Skykeep

**Your iCloud, at home on Linux and Windows.**

Notes, Reminders, Calendar, Contacts, Drive and Photos as fast desktop apps that stay in sync with your iPhone.

[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-2f6fd6)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20Windows-4a5568)](#install)
[![Local-first](https://img.shields.io/badge/local--first-works%20offline-2f6fd6)](#privacy)
[![Built with Rust and Tauri](https://img.shields.io/badge/built%20with-Rust%20%2B%20Tauri%202-b7410e)](#contributing)
<!-- Dynamic badges: enable once the repository is public.
[![Latest release](https://img.shields.io/github/v/release/YashwanthKamireddi/Skykeep?include_prereleases&sort=semver&color=2f6fd6)](https://github.com/YashwanthKamireddi/Skykeep/releases)
[![Release build](https://img.shields.io/github/actions/workflow/status/YashwanthKamireddi/Skykeep/release.yml?label=release%20build)](https://github.com/YashwanthKamireddi/Skykeep/actions/workflows/release.yml)
[![Stars](https://img.shields.io/github/stars/YashwanthKamireddi/Skykeep?style=flat&color=e3b341)](https://github.com/YashwanthKamireddi/Skykeep/stargazers)
[![AUR](https://img.shields.io/aur/version/skykeep-git?logo=archlinux&label=AUR)](https://aur.archlinux.org/packages/skykeep-git)
[![Flathub](https://img.shields.io/flathub/v/io.github.yashwanthkamireddi.Skykeep?logo=flathub)](https://flathub.org/apps/io.github.yashwanthkamireddi.Skykeep)
-->

[Install](#install) · [Features](#features) · [Privacy](#privacy) · [FAQ](#faq) · [Contributing](#contributing)

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/media/notes-dark.png">
  <img src="docs/media/notes-light.png" alt="Skykeep Notes showing a trip note with a picture, a checklist and a table" width="880">
</picture>

</div>

## Six apps, one sign-in

<table>
  <tr>
    <td width="50%" align="center">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/reminders-dark.png">
        <img src="docs/media/reminders-light.png" alt="Skykeep Reminders showing Today with coloured lists, priorities and flags">
      </picture>
      <b>Reminders</b>
    </td>
    <td width="50%" align="center">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/calendar-dark.png">
        <img src="docs/media/calendar-light.png" alt="Skykeep Calendar in week view with events from four calendars">
      </picture>
      <b>Calendar</b>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/contacts-dark.png">
        <img src="docs/media/contacts-light.png" alt="Skykeep Contacts showing a contact card with phones, emails and an address">
      </picture>
      <b>Contacts</b>
    </td>
    <td width="50%" align="center">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/drive-dark.png">
        <img src="docs/media/drive-light.png" alt="Skykeep Drive showing a folder of pictures in grid view">
      </picture>
      <b>iCloud Drive</b>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/photos-dark.png">
        <img src="docs/media/photos-light.png" alt="Skykeep Photos showing the library by month">
      </picture>
      <b>Photos</b>
    </td>
    <td width="50%" align="center">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/media/settings-dark.png">
        <img src="docs/media/settings-light.png" alt="Skykeep Settings showing the account, storage and when each app last synced">
      </picture>
      <b>Settings</b>
    </td>
  </tr>
</table>

## Features

| App | What you can do | Coming |
|---|---|---|
| **Notes** | Write with headings, styles, lists and checklists. See pictures, scans, PDFs and tables in place. Folders, pinned notes, search and Recently Deleted. Edits from your phone merge with yours. | Adding attachments, editing tables |
| **Reminders** | Lists with colours and icons, Today, Scheduled, Flagged and your smart lists. Subtasks, due dates, repeats, flags and priorities. Alerts with **Complete** and **Snooze**. | Editing tags, links, early alerts and sections |
| **Calendar** | Day, Week, Month and Year. Drag to create, move and resize. Repeating events, alerts, time zones, birthdays and holidays. Subscribe to any `webcal://` calendar and open `.ics` files. | |
| **Contacts** | Every field, with Apple's labels. Lists, My Card, find and merge duplicates. Import vCard or a Google or Outlook CSV; export vCard. | |
| **iCloud Drive** | List or grid with thumbnails. Upload files and folders, rename, move, duplicate and preview with Space. On Linux, iCloud Drive is also a real folder in your file manager. | |
| **Photos** | Your library by month, albums and shared albums, Favourites and Videos. A viewer for Live Photos and videos, slideshow, upload and original-quality download. | |
| **Everywhere** | Search across every app. Quick capture for a note or reminder. Results in GNOME and KDE search, a tray icon, light and dark. **Export Everything** to Markdown, iCalendar, vCard and your original files. | |

## Install

Packages arrive with the first release on the [Releases page](https://github.com/YashwanthKamireddi/Skykeep/releases), each with SHA-256 checksums.

| Platform | Package |
|---|---|
| **Debian, Ubuntu, Mint** | `.deb` |
| **Fedora, openSUSE** | `.rpm` |
| **Any Linux** | `.AppImage` |
| **Arch, CachyOS, EndeavourOS, Manjaro** | AUR `skykeep-git` <sub>coming to the AUR</sub> |
| **Any Linux, sandboxed** | Flatpak <sub>coming to Flathub</sub> |
| **Windows 10 and 11** | `.exe` or `.msi` |

<details>
<summary><b>Debian and Ubuntu</b></summary>

```sh
sudo apt install ./Skykeep_*_amd64.deb
```
</details>

<details>
<summary><b>Fedora and openSUSE</b></summary>

```sh
sudo dnf install ./Skykeep-*.x86_64.rpm       # Fedora
sudo zypper install ./Skykeep-*.x86_64.rpm    # openSUSE
```
</details>

<details>
<summary><b>AppImage</b></summary>

```sh
chmod +x Skykeep_*_amd64.AppImage
./Skykeep_*_amd64.AppImage
```

Uses the WebKitGTK 4.1 and libsecret already on most desktops.
</details>

<details>
<summary><b>Arch and derivatives</b></summary>

Once it's on the AUR:

```sh
paru -S skykeep-git    # or: yay -S skykeep-git
```

Today, build the same package from a checkout:

```sh
git clone https://github.com/YashwanthKamireddi/Skykeep.git
cd Skykeep/packaging/arch && makepkg -si
```

Add `fuse3` to see iCloud Drive and Photos as folders in your file manager.
</details>

<details>
<summary><b>Flatpak</b></summary>

The manifest lives in [`packaging/flatpak`](packaging/flatpak). Once it's on Flathub:

```sh
flatpak install flathub io.github.yashwanthkamireddi.Skykeep
```

In the Flatpak, iCloud Drive and Photos open inside the app.
</details>

<details>
<summary><b>Windows</b></summary>

Run `Skykeep_*_x64-setup.exe` (installs for you alone, no admin needed) or the `.msi`. Skykeep opens `.ics` and `.vcf` files, and its alerts come with **Complete** and **Snooze**.
</details>

**First run:** open Skykeep, sign in with your Apple Account, and enter the code from your iPhone. Each app fills as your library arrives, then opens instantly from then on.

## Privacy

- **Local-first.** Your library lives on your computer, so every app opens instantly and works offline. Changes you make offline are sent when you're back online.
- **Credentials stay in your system keychain.** You sign in through Apple's own two-factor flow, and your password and account tokens are kept in the keychain, never in a file: Secret Service on Linux, Credential Manager on Windows.
- **Talks only to Apple.** Skykeep's network layer reaches Apple's iCloud hosts, plus any calendars you subscribe to. No Skykeep account, no Skykeep server, no telemetry.
- **Independent.** Skykeep is an open-source project, not affiliated with or endorsed by Apple.

The details, with the code behind each one, are in [SECURITY.md](SECURITY.md).

## Performance

Measured on release builds. Numbers land with the first release.

| | Linux | Windows |
|---|---|---|
| Launch to a usable window | *to be measured* | *to be measured* |
| Open a note from the list | *to be measured* | *to be measured* |
| A change on iPhone shown here | *to be measured* | *to be measured* |
| Memory with all six apps open | *to be measured* | *to be measured* |
| Download size | *to be measured* | *to be measured* |

## FAQ

<details>
<summary><b>Is Skykeep made by Apple?</b></summary>

No. It's an independent, open-source app that works with iCloud. Apple, iCloud and iPhone are trademarks of Apple Inc.
</details>

<details>
<summary><b>Does it work offline?</b></summary>

Yes. Everything you've synced is on your disk. Notes, reminders, events and contacts can be edited offline and sync when you reconnect.
</details>

<details>
<summary><b>How fast do changes from my iPhone appear?</b></summary>

Notes and Reminders arrive over Apple's push channel, usually in about a second. Calendar and Contacts are checked every few seconds while a window is open. Your own edits show the moment you make them.
</details>

<details>
<summary><b>I use Advanced Data Protection. Will it work?</b></summary>

Yes, with **Access iCloud Data on the Web** turned on: on your iPhone, Settings → your name → iCloud. Skykeep tells you when it's needed.
</details>

<details>
<summary><b>Can I take my data with me?</b></summary>

Any time. Settings → Export Everything saves your notes as Markdown, events as iCalendar, contacts as vCard and CSV, and your files as they are.
</details>

<details>
<summary><b>Which desktops does it run on?</b></summary>

Any Linux desktop with WebKitGTK 4.1 and a keyring (GNOME Keyring or KWallet), on Wayland or X11, and Windows 10 and 11.
</details>

## Contributing

Bug reports, packaging recipes, design ideas and code are all welcome. Start with [CONTRIBUTING.md](.github/CONTRIBUTING.md) for setup and the checks, and run `./check.sh` before a pull request.

```sh
git clone https://github.com/YashwanthKamireddi/Skykeep.git
cd Skykeep && npm ci && ./run.sh
```

Found a security issue? Please report it privately, as described in [SECURITY.md](SECURITY.md).

## License

[GPL-3.0](LICENSE). Not affiliated with Apple Inc.
