# Getting Quince onto a machine

Quince is only software if somebody other than its author can install it. This directory is
what turns a build directory into that.

## Arch, and anything built on it — CachyOS, EndeavourOS, Manjaro

```sh
cd packaging/arch
makepkg -si
```

That builds from the checkout this file sits in and installs Quince the way any other program
on the machine is installed: `quince` on the path, an entry in the application menu, an icon,
and the licence where a package manager expects to find it.

For the AUR the same PKGBUILD gains a `source=("git+…")` and a `pkgver()`; the build and
package steps are already the ones that version would use.

## Debian, Ubuntu, Fedora, or anything at all

```sh
npx tauri build --bundles deb,rpm,appimage
```

leaves them under `target/release/bundle/`. The AppImage needs no package manager and runs
anywhere with WebKitGTK.

## Flatpak

Not written yet, and deliberately not guessed at. It is the channel that matters most —
Flathub is the only place a non-technical person installs anything with one click on any
distribution — but it is also the one with real work in it: the GNOME runtime with Rust and
Node SDK extensions, every crate and npm package vendored ahead of time by
`flatpak-cargo-generator` and `flatpak-node-generator` so the build runs offline, `/app`
instead of `/usr` throughout, and `--talk-name=org.freedesktop.secrets` in `finish-args` or
the session cannot be kept inside the sandbox.

Writing a manifest on a machine with no `flatpak-builder` to try it on would produce something
that looks finished and has never been run. It is next, on a machine that can build it.

## What Quince needs to run

`webkit2gtk-4.1` for the window, `libsecret` for the keyring. The app starts without the
second and cannot stay signed in, which is most of the point of it.

## Opening files and quick capture

Calendar (`.ics`) and contact (`.vcf`) files open in Pomander's import sheet
(`src-tauri/src/intake.rs`): read on this computer, shown, and added only when confirmed.

- **Linux:** the main entry declares `MimeType=text/calendar;text/vcard;text/x-vcard;` and takes
  `%U`; the metainfo files list the same types under `<provides>`. The per-app entries do not
  claim them, so "Open With" offers one "Pomander", not a second "Calendar" beside GNOME's.
- **Windows:** `bundle.fileAssociations` in `tauri.conf.json` registers `.ics`/`.ical` and
  `.vcf`/`.vcard` in both the NSIS and MSI installers; Explorer passes the path on the command
  line, and a second launch hands it to the running Pomander (single instance).

`pomander --new reminder` and `pomander --new note` open the quick-capture window. They are
`Actions=` on the main entry and on the Reminders and Notes entries (right-click in a dock or
launcher).

**Windows jump-list tasks are not set up.** A jump list's tasks are written by the running app
through the shell's `ICustomDestinationList` COM interface; neither Tauri nor its NSIS/MSI
templates have a way to declare them. Adding them means a Windows-only call at startup
(`windows` crate, `Win32_UI_Shell`) registering two `IShellLink` tasks with the arguments
above. Until then, the same commands work from a shortcut's Target field.
