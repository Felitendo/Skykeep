# Packaging Skykeep

Everything that turns a build into something people install.

| Path | What it is |
|---|---|
| `arch/` | A PKGBUILD that builds the checkout it sits in. |
| `aur/` | The AUR package, `skykeep-git`: the same steps, built from GitHub. |
| `flatpak/` | The Flatpak manifest for Flathub, with its own [README](flatpak/README.md). |
| `apps/` | The six per-app launcher entries (Notes, Reminders, Calendar, Contacts, Drive, Photos). |
| `*.desktop.in`, `*.metainfo.xml` | The main launcher entry and the AppStream data software centres show. |
| `*.search-provider.ini`, `*.SearchProvider.service`, `*.krunner.desktop` | Results in GNOME Shell and KRunner search. |
| `icons/` | The app, tray and notification icons. |
| `linux-compat/` | `check.sh`: installs a package in a clean distribution container and checks it runs. |

## Arch, CachyOS, EndeavourOS, Manjaro

```sh
cd packaging/arch
makepkg -si
```

This installs Skykeep like any other program: `skykeep` on the path, launcher entries for the
main app and each of the six apps, the icon, search providers and the licence.

## Debian, Ubuntu, Fedora, AppImage, Windows

```sh
npx tauri build
```

builds every bundle for the platform you're on (`.deb`, `.rpm` and `.AppImage` on Linux;
`.msi` and the `-setup.exe` on Windows) under `target/release/bundle/`. Releases are built
the same way by `.github/workflows/release.yml`, which installs and starts each Linux package
before keeping it.

## Flatpak

See [`flatpak/README.md`](flatpak/README.md): how to build it, the permissions it asks for,
and what's left before Flathub.

## What Skykeep needs to run

- `webkit2gtk-4.1` for the window and `libsecret` for the keyring. Without a keyring the app
  starts but can't stay signed in.
- `libheif` for HEIC photos.
- GStreamer with the base, good and libav plugins for videos.
- Optional: `fuse3`, for iCloud Drive and Photos as folders in the file manager.

## Opening files and quick capture

Calendar (`.ics`) and contact (`.vcf`) files open in Skykeep's import sheet
(`src-tauri/src/intake.rs`): read on this computer, shown, and added only when confirmed.

- **Linux:** the main entry declares `MimeType=text/calendar;text/vcard;text/x-vcard;` and takes
  `%U`; the metainfo files list the same types under `<provides>`. The per-app entries do not
  claim them, so "Open With" offers one "Skykeep", not a second "Calendar" beside GNOME's.
- **Windows:** `bundle.fileAssociations` in `tauri.conf.json` registers `.ics`/`.ical` and
  `.vcf`/`.vcard` in both the NSIS and MSI installers; Explorer passes the path on the command
  line, and a second launch hands it to the running Skykeep (single instance).

`skykeep --new reminder` and `skykeep --new note` open the quick-capture window;
`skykeep --new event` opens Calendar with a new event started. The first two are `Actions=` on
the main entry and on the Reminders and Notes entries (right-click in a dock or launcher).

## The six apps on Windows

What the per-app `.desktop` files and `Actions=` are on Linux:

- **Start menu:** beside "Skykeep", "Skykeep Notes", "Skykeep Reminders" and so on, each running
  `skykeep.exe --app notes` (and so on). The `.exe` installer makes them in
  `windows/installer-hooks.nsh` (`bundle.windows.nsis.installerHooks`), and removes them, unpinned,
  on uninstall; the MSI makes the same six from `windows/app-shortcuts.wxs`
  (`bundle.windows.wix.fragmentPaths`), in one component that its uninstall removes.
- **Taskbar buttons:** each shortcut carries its app's AppUserModelID,
  `io.github.yashwanthkamireddi.Skykeep.Notes` and so on, and each app's window says the same ID
  at runtime (`src-tauri/src/taskbar.rs`), so Notes and Calendar get their own taskbar buttons
  and pin as their own shortcuts. The process says Skykeep's own ID, the one Tauri stamps on
  "Skykeep", so the main window, toasts and the jump list are Skykeep's.
- **Jump list:** Skykeep writes it as it starts (`ICustomDestinationList`; no installer can
  declare one): New Note, New Reminder, New Event, then the six apps. Notes', Reminders' and
  Calendar's own buttons offer their own new item. Each task is one of the command lines above,
  handed to the Skykeep already running.

`.github/workflows/windows.yml` checks all of this after a silent install: each entry's app ID
and command line, each app window's app ID, every jump-list task, and that uninstalling leaves no
entry behind.
