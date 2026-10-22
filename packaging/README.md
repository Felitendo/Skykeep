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

`skykeep --new reminder` and `skykeep --new note` open the quick-capture window. They are
`Actions=` on the main entry and on the Reminders and Notes entries (right-click in a dock or
launcher).

**Windows jump-list tasks are not set up yet.** A jump list's tasks are written by the running
app through the shell's `ICustomDestinationList` COM interface; neither Tauri nor its NSIS/MSI
templates can declare them. Adding them means a Windows-only call at startup (`windows` crate,
`Win32_UI_Shell`) registering two `IShellLink` tasks with the arguments above. Until then, the
same commands work from a shortcut's Target field.
