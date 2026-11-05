# Good first issues: drafts

Fifteen issues to file on GitHub on launch day, each checked against the code on 2026-10-25 and sized for about an afternoon.
Every issue carries **good first issue**, or **help wanted** where it needs some specialist knowledge, plus area labels picked from the repo's existing set: `packaging`, `windows`, `rust`, `javascript`, `accessibility`, `sync`, `notes`, `drive`, `contacts`, `design`, `bug`, `enhancement`, `documentation`.
Most of them can be tested without an Apple Account. The `.uiverify/` Playwright suites run the real interface against `npx vite preview --port 4173` with a stubbed IPC and sample data (`.uiverify/stub.mjs`, plus the invented demo library in `.uiverify/readme-fixture.mjs`), and the Rust parsers have plain unit tests.

---

## 1. `skykeep --help` and `--version` print and exit

**Labels:** good first issue, enhancement, rust

**Why it matters:** At the moment, `skykeep --help` starts the app, or wakes the copy that's already running. People who live in a terminal expect a short list of flags, and packagers expect `--version`.

**Where in the code**
- `src-tauri/src/main.rs`, `fn main()` (around line 5924). The first call is `updates::after_update()`, and the single-instance plugin is registered at about line 5959.
- Flags already read: `--app` and `--new` in `src-tauri/src/apps.rs` (`asked`, `new_asked`), `--background` and `--search-provider` in `src-tauri/src/login.rs`, and the `.ics`/`.vcf` paths or `file://` URIs in `apps::files_asked`.

**Acceptance criteria**
- [ ] `skykeep --help` (and `-h`) prints usage covering `--app <notes|reminders|calendar|contacts|drive|photos>`, `--new <note|reminder|event>`, `--background` and opening `.ics`/`.vcf` files, then exits 0. It opens no window and doesn't hand anything to a running Skykeep.
- [ ] `skykeep --version` (and `-V`) prints `Skykeep 0.1.0`, taking the version from the build so it can't drift, and exits 0.
- [ ] The check runs first in `main()`, before `updates::after_update()` and the single-instance plugin.
- [ ] A unit test in `apps.rs` covers the parsing (`--help`, `-h`, and `--help` combined with other flags).
- [ ] On Windows a release build has no console (`windows_subsystem = "windows"`). It's fine if Windows prints nothing; attaching to the parent console is a nice extra.

**How to test:** `cargo test -p quince-app apps`, then `cargo run -p quince-app -- --help` with and without Skykeep already running.

---

## 2. A man page for `skykeep`

**Labels:** good first issue, documentation, packaging

**Why it matters:** `man skykeep` should explain the launcher flags and where Skykeep keeps its files, like any other well-packaged Linux app.

**Where in the code**
- The flags are listed in issue 1. `packaging/README.md`, under "Opening files and quick capture", describes `--new` and `--app` in prose.
- Installs: `packaging/arch/PKGBUILD` and `packaging/aur/PKGBUILD` (`package()`), `bundle.linux.deb.files` / `rpm.files` in `src-tauri/tauri.conf.json`, and `packaging/flatpak/io.github.yashwanthkamireddi.Skykeep.yml`.

**Acceptance criteria**
- [ ] `packaging/skykeep.1`, written in plain roff or scdoc, with NAME, SYNOPSIS, DESCRIPTION, OPTIONS, FILES and SEE ALSO sections. It agrees with the `--help` text from issue 1.
- [ ] The Arch and AUR PKGBUILDs and the deb/rpm file maps install it to `/usr/share/man/man1/skykeep.1`.
- [ ] Nothing is claimed that the code doesn't do. Check each flag in `apps.rs` and `login.rs`.

**How to test:** `man ./packaging/skykeep.1`, `mandoc -T lint packaging/skykeep.1` (or `groff -man -ww`), then `makepkg` in `packaging/arch` and `pacman -Ql skykeep | grep man1`.

---

## 3. Brand colours for software centres (`<branding>` in the metainfo)

**Labels:** good first issue, packaging, design

**Why it matters:** GNOME Software and Flathub paint an app's banner in its brand colours, light and dark. Without them Skykeep's page falls back to a plain grey, and Flathub's linter asks for them before submission.

**Where in the code**
- `packaging/io.github.yashwanthkamireddi.Skykeep.metainfo.xml` and `packaging/flatpak/io.github.yashwanthkamireddi.Skykeep.metainfo.xml`: neither has a `<branding>` block yet. Keep the two files in step.
- The logo's own blues are in `packaging/icons/skykeep.svg` (`#1F7FE6` and `#2F9BFF`); please pick from the logo rather than new colours.

**Acceptance criteria**
- [ ] Both metainfo files have `<branding>` with a `primary` colour for `scheme_preference="light"` and one for `"dark"`.
- [ ] The icon stays clearly visible on both colours (a screenshot of each in the PR is perfect).
- [ ] `appstreamcli validate --no-net` passes on both files, and `flatpak-builder-lint appstream` no longer mentions branding if you have it (`flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream <file>`).

**How to test:** Run the validators above. `appstreamcli` can't show the banner, so to see it, hold the icon over a square of each colour in any image viewer, or preview it in GNOME Software from a local Flatpak build (`packaging/flatpak/README.md`).

---

## 4. Send waiting changes as soon as the network returns, inside the Flatpak

**Labels:** good first issue, enhancement, sync, rust

**Why it matters:** When an edit is made offline, Skykeep sends it the moment the connection comes back. In the Flatpak the sandbox hides the system bus, so that wake-up never fires, and changes wait for the next retry or for a window to notice.

**Where in the code**
- `src-tauri/src/outbox.rs`, `watch_network()` (around line 1850). It listens for NetworkManager's `StateChanged` and connman's `State` on the **system** bus, then calls `kick()` and `crate::push::wake()`. `came_online()` and its test (around line 3350) handle the "was offline, now online" edge.
- `packaging/flatpak/io.github.yashwanthkamireddi.Skykeep.yml`, `finish-args`, grants no system-bus names. The window's `online` event (`src/lib/outbox.ts` → `outbox_wake`) is the only fallback.

**Acceptance criteria**
- [ ] A third watcher in `watch_network()` uses the `org.freedesktop.portal.NetworkMonitor` portal on the **session** bus: it listens for `changed` and reads availability with `GetAvailable` or `GetStatus`, using the same zbus proxy pattern as the two watchers already there.
- [ ] It goes through `came_online()`, so one reconnection wakes the outbox once, even if NetworkManager and the portal both report it.
- [ ] No new `finish-args` are needed. Where the portal is missing, it gives up quietly, as the others do.
- [ ] A unit test covers the availability → `came_online` mapping.

**How to test:** Run `cargo test -p quince-app outbox`. `gdbus monitor --session --dest org.freedesktop.portal.Desktop` shows the signal while you run `nmcli networking off` and then `on`. The end-to-end check (an edit made offline arriving after reconnect) needs a **test** Apple Account; it's optional for the PR.

---

## 5. Sidebar: Tab reaches the app list while Settings is open, and the account button says "Settings"

**Labels:** good first issue, accessibility, bug, javascript

**Why it matters:** Keyboard and screen-reader users should always be able to reach the six apps. They should also know that the button with their name on it opens Settings.

**Where in the code**
- `src/screens/Library.tsx`, the `item()` row (around line 452): `tabIndex={selected === key ? 0 : -1}`. With Settings in front, no app row gets `tabIndex` 0, so Tab skips the whole list. This was first noted in `docs/audit/shell-ux.md` #10.
- `src/components/Account.tsx` (around line 139): the `.sidebar-account` button's accessible name is only the person's name. "Settings" is just a `title`.

**Acceptance criteria**
- [ ] When the selected screen isn't one of the apps, the first app row takes `tabIndex` 0. The arrow keys still move between rows as they do now.
- [ ] The account button's accessible name includes "Settings" (for example `aria-label={`${name}, Settings`}`), and `aria-current` still works.
- [ ] axe stays clean.

**How to test:** Run `npx vite preview --port 4173 --strictPort &`, then `node .uiverify/keyboard.mjs`, `node .uiverify/a11y-keys.mjs` and `node .uiverify/a11y.mjs`, and each again with `ENGINE=webkit`. Extend `keyboard.mjs` to open Settings and then Tab into the app list. The test should fail on the old code.

---

## 6. A screen-reader walkthrough with Orca or NVDA

**Labels:** help wanted, accessibility

**Why it matters:** axe passes and the lists are proper list boxes, but nobody has used Skykeep end to end with a screen reader yet. One honest report from someone who uses Orca (Linux) or NVDA (Windows) every day would steer the next accessibility pass.

**Where in the code:** `src/styles/a11y.css`, `src/lib/focus.ts`, `src/lib/screen.ts` (`overlayOpen`), and the screens in `src/screens/`. Existing checks are `.uiverify/a11y.mjs` (axe) and `.uiverify/a11y-keys.mjs`.

**Acceptance criteria**
- [ ] A comment, or a Markdown report attached to the issue, covering: signing in, moving between apps, reading and writing a note, ticking a reminder, adding an event, opening a contact, and browsing a Drive folder.
- [ ] For each problem: what was said, what you expected, and the steps to reproduce. Include the screen reader, its version, and your desktop or Windows version.
- [ ] Use a test Apple Account, and don't paste real personal data into the report.

**How to test:** It's a report rather than code. I'll turn each finding into its own issue.

---

## 7. Ctrl+Shift+N makes a new folder in Notes and Drive

**Labels:** good first issue, enhancement, notes, drive, javascript

**Why it matters:** Notes and Finder both use ⌘⇧N for New Folder. Skykeep follows Apple's keys with Ctrl for ⌘ (and Reminders already uses Ctrl+Shift+N for New List), but in Notes and Drive the shortcut does nothing yet.

**Where in the code**
- `src/screens/NotesView.tsx`: the key handler (around lines 1075–1110) only takes `mod && !e.shiftKey && k === "n"`. `newFolder(parent)` (around line 787) already makes a folder instantly, ready to be named.
- `src/screens/DriveView.tsx`: the Finder-style key handler (around lines 1220–1295). The toolbar's New Folder button calls `setCreatingFolder(true)` (around line 1416).
- `src/components/ShortcutsSheet.tsx`: the `SHORTCUTS` rows for Notes and Drive.

**Acceptance criteria**
- [ ] In Notes, Ctrl+Shift+N creates a folder at the top level and starts renaming it, exactly as the sidebar's add button does.
- [ ] In Drive, Ctrl+Shift+N opens the New Folder field in the folder being shown. It does nothing in Recents or Recently Deleted, or while renaming or moving, following the guards the handler already has.
- [ ] Neither fires while a sheet or menu is open (`overlayOpen()`), or while you're typing in a field.
- [ ] The Shortcuts sheet lists both.

**How to test:** Extend `.uiverify/notes-keys.mjs` (or `notes-folders.mjs`) and `.uiverify/drive-keys.mjs`, then run each in Chromium and with `ENGINE=webkit`. Run `npx tsc --noEmit` too.

---

## 8. Contacts: ask before throwing away a card that's being edited

**Labels:** good first issue, bug, contacts, javascript

**Why it matters:** If you press Ctrl+N or the New Contact button while editing a card, or open a contact from global search, the text you've typed disappears without a word. Skykeep never loses someone's typing silently anywhere else.

**Where in the code** (`src/screens/ContactsView.tsx`)
- `startNew()` (around line 224) and the Ctrl+N branch of the key handler (around line 820) replace `draft` without checking it.
- The New Contact button (around line 961) is only `disabled={busy}`.
- The `useReveal("contacts", …)` callback (around line 258) calls `setDraft(null)`.
- `draftBase` (around line 194) holds the card as it was before editing. `useConfirm()` (around line 161) is the app's existing confirmation sheet.

**Acceptance criteria**
- [ ] If the draft differs from what it started as, Skykeep asks first (for example "Discard changes?" with Discard and Keep Editing). An untouched draft is replaced without asking.
- [ ] Keep Editing leaves the draft and the focus where they were.
- [ ] Escape and Cancel still leave a draft as they do today.

**How to test:** Extend `.uiverify/contacts-keys.mjs` or `contacts-fields.mjs`: type into a new card, press Ctrl+N, and expect the question. Then do the same with an untouched card and expect no question. Run them in Chromium and with `ENGINE=webkit`.

---

## 9. Contacts: Outlook CSV imports keep the right title, birthday and phone labels

**Labels:** good first issue, bug, contacts, rust

**Why it matters:** For many people, a CSV from Outlook or Google is the way out of an old address book. Three details currently get lost or mixed up on the way in.

**Where in the code:** `crates/quince-icloud/src/contacts.rs`, `csv_to_vcards()` (around line 1516). Its test is `a_csv_export_becomes_vcards` (around line 2035).

**What happens now**
1. `title` is matched by `col(&["job title", "title", …])`, which takes the **first** matching column. Outlook's export starts with a `Title` column holding the name prefix ("Dr."), so "Dr." becomes the job title. It should go to the N prefix instead, and `Job Title` to TITLE.
2. Birthdays are copied as they are when they're 8 or more characters long. Outlook's `3/14/1990` ends up as `BDAY:3/14/1990`, which isn't a vCard date, and a birthday with no year (`--03-14`) is dropped.
3. Every number becomes `TEL;TYPE=CELL`, so "Home Phone" and "Business Phone" turn into mobiles.

**Acceptance criteria**
- [ ] When a header row has both `Title` and `Job Title`, `Title` fills the name prefix and `Job Title` fills TITLE.
- [ ] Birthdays come out as `YYYY-MM-DD` from `YYYY-MM-DD`, `M/D/YYYY` and `--MM-DD`. Outlook's empty `0/0/00` and anything unreadable are left out, never guessed. For a birthday with no year, match how a vCard exported from iCloud writes one (Settings → Export Everything makes one to compare against).
- [ ] Phone and email types follow the heading: home, work/business or mobile, and for Google, a `… - Label` / `… - Type` column next to the value.
- [ ] Unit tests use synthetic rows only: invented names, 555-01xx numbers, example.com addresses. For example:
  ```text
  Title,First Name,Last Name,Job Title,Home Phone,Business Phone,Mobile Phone,E-mail Address,Birthday
  Dr.,Mira,Okafor,Pilot,(555) 010-0101,(555) 010-0102,(555) 010-0103,mira@example.com,3/14/1990
  ```

**How to test:** `cargo test -p quince-icloud csv`. The import flow in the interface is covered by `.uiverify/contacts-vcard.mjs`.

---

## 10. Contacts: import CSV and vCard files that aren't UTF-8, and CSVs separated by semicolons

**Labels:** good first issue, enhancement, contacts, rust

**Why it matters:** Excel and older Outlook versions often save CSVs as UTF-16 or Windows-1252, and in many regions they separate fields with `;`. Today those files either fail with a read error or come in as one long column.

**Where in the code**
- `src-tauri/src/main.rs`, `contacts_import` (around line 3895), reads each file with `std::fs::read_to_string`, which fails on anything that isn't valid UTF-8.
- `src-tauri/src/intake.rs`, `load()` (around line 198), the path for opening a file from the file manager, uses `String::from_utf8_lossy`, which turns accented letters in a Windows-1252 file into `�`.
- `crates/quince-icloud/src/contacts.rs`, `csv_rows()` (around line 1467), only splits on `,`.

**Acceptance criteria**
- [ ] One small decoding function, shared by both paths and unit-tested: UTF-8 (with or without a BOM), UTF-16 LE/BE with a BOM, and Windows-1252 as the fallback for bytes that aren't UTF-8. The standard library is enough here; if you'd like to add a crate, mention it in the issue first.
- [ ] `csv_rows` uses `;` when the header line contains semicolons and no commas outside quotes. Fields quoted with `"` keep working the same way.
- [ ] Tests use synthetic sample bytes, such as a `José` written in Windows-1252 and a UTF-16 file with a BOM.

**How to test:** `cargo test -p quince-icloud csv` and `cargo test -p quince-app intake`.

---

## 11. Keep the winget manifest current on every release

**Labels:** good first issue, packaging, windows

**Why it matters:** `winget install` is how many Windows users install software. I'll make the first submission by hand on launch week. After that, each release should reach winget without anyone having to remember.

**Where in the code**
- `.github/workflows/release.yml` builds and publishes `*-setup.exe` and `*.msi` with `SHA256SUMS` (the `publish` job at the end).
- `src-tauri/tauri.conf.json`: `bundle.windows.nsis.installMode` is `currentUser`, so the winget manifest's scope is user.

**Acceptance criteria**
- [ ] A job that runs after a release is published and opens the winget-pkgs pull request for the new version using `komac` or `wingetcreate update`, with the package identifier from the first submission.
- [ ] It uses a token stored as a repository secret, and does nothing (successfully) when the secret isn't set, so forks stay green.
- [ ] Actions are pinned to a commit SHA, as the existing workflows do.
- [ ] A short note in `packaging/README.md`.

**How to test:** Do a dry run on a fork against a test release (`komac update --dry-run` or `wingetcreate update` without `--submit`), and attach the generated manifest to the PR.

---

## 12. A Scoop manifest, in a personal bucket for now

**Labels:** good first issue, packaging, windows

**Why it matters:** Scoop users install without admin rights or installers. Scoop Extras asks for 100 stars first, so a manifest that's ready to go can live in a personal bucket until then.

**Where in the code**
- Release assets come from `.github/workflows/release.yml` (`.msi`, `-setup.exe`, `SHA256SUMS`).
- The Start-menu entries for the six apps are `skykeep.exe --app notes` and so on (`packaging/README.md`, "The six apps on Windows").
- `src-tauri/src/updates.rs`: `install_kind()` decides who updates a copy. A program taken out of the `.msi` still reports `BundleType::Msi`, so it would see itself as `Install::Here` and offer its own updates, alongside Scoop's.

**Acceptance criteria**
- [ ] `packaging/scoop/skykeep.json`, with the MSI extracted (Scoop handles that), `bin`, `shortcuts` for Skykeep and the six apps (with their `--app` arguments), `checkver` and `autoupdate` from GitHub Releases, and the hash read from `SHA256SUMS`.
- [ ] A copy installed by Scoop leaves updates to Scoop. For example, `this_install` recognises a program under `scoop\apps\`, with a case added to the `who_updates_each_kind_of_install` test.
- [ ] Notes in `packaging/README.md`, including the WebView2 runtime that Windows 10 may need.

**How to test:** `scoop install .\packaging\scoop\skykeep.json` in Windows Sandbox or a VM, then launch Skykeep and one app shortcut, check that Settings offers no in-app update, and `scoop uninstall skykeep`. Run `cargo test -p quince-app updates`.

---

## 13. A Chocolatey package

**Labels:** good first issue, packaging, windows

**Why it matters:** Chocolatey is how many people and IT departments script Windows installs. A package that's reviewed and ready makes the community-repository submission quick.

**Where in the code:** Release assets from `.github/workflows/release.yml`. The NSIS installer installs per user (`installMode: currentUser`) and accepts `/S`. The uninstall entries and shortcuts it makes are covered by `packaging/windows/installer-hooks.nsh`.

**Acceptance criteria**
- [ ] `packaging/chocolatey/` holds `skykeep.nuspec` (ProjectUrl, LicenseUrl for GPL-3.0-only, an icon from `packaging/icons/`, and a description that says Skykeep isn't affiliated with Apple) plus `tools/chocolateyinstall.ps1` and `chocolateyuninstall.ps1`.
- [ ] The installer is downloaded from GitHub Releases with a SHA-256 checksum, not embedded, so no VERIFICATION.txt is needed. The install is silent.
- [ ] Uninstalling leaves no Start-menu entry behind.
- [ ] The description says that Skykeep's installer copy can also update itself (`src-tauri/src/updates.rs`).

**How to test:** `choco pack` in `packaging/chocolatey`, then `choco install skykeep -s . -y` and `choco uninstall skykeep -y` in Windows Sandbox. Attach the output to the PR.

---

## 14. A Nix flake

**Labels:** help wanted, packaging

**Why it matters:** NixOS users can't use the deb, rpm or AppImage comfortably. A flake would give them `nix run github:YashwanthKamireddi/Skykeep`, and later a path into nixpkgs.

**Where in the code**
- Build dependencies are listed in `.github/CONTRIBUTING.md` under "Set up": webkit2gtk-4.1, gtk3, libsecret, libheif ≥ 1.17, GStreamer base/good/libav, fuse3 and librsvg.
- The Rust version is pinned in `rust-toolchain.toml` (1.98.1).
- The frontend is built with `npm ci && npm run build`.
- The files to install are the same ones `packaging/arch/PKGBUILD` installs: the binary as `skykeep`, the desktop entries in `packaging/` and `packaging/apps/`, the metainfo, the icons, and the search-provider files.
- On Linux, a copy built without Tauri's bundler reports `Install::PackageManager` (`updates.rs`, `install_kind`), so the in-app updater stays out of the way automatically.

**Acceptance criteria**
- [ ] `flake.nix` (and `flake.lock`) builds Skykeep with nixpkgs' Tauri tooling (for example `cargo-tauri.hook` with `fetchNpmDeps`) and wraps it with the GStreamer and GIO paths it needs.
- [ ] `nix run . -- --app notes` opens Notes, videos play, HEIC thumbnails show, and the keyring keeps the sign-in.
- [ ] Short instructions in `packaging/README.md`.

**How to test:** `nix build` and `nix flake check` on NixOS or with Nix on another distro. Launch it and sign in with a **test** Apple Account.

---

## 15. Fedora COPR

**Labels:** help wanted, packaging

**Why it matters:** The release's `.rpm` installs on Fedora, but it doesn't update itself. COPR would let Fedora users get updates through `dnf upgrade` like everything else.

**Where in the code**
- The Fedora build dependencies are in `.github/CONTRIBUTING.md` ("Set up").
- `packaging/linux-compat/check.sh` and `.github/workflows/linux-compat.yml` build and install on Fedora 42 in a clean container already.
- The installed file list should match the deb/rpm maps in `src-tauri/tauri.conf.json` (`bundle.linux`).

**Acceptance criteria**
- [ ] `packaging/fedora/skykeep.spec` builds from a release source tarball. Cargo crates and npm packages are vendored, or fetched with COPR's network option switched on; please explain which you chose and why.
- [ ] It installs the binary as `skykeep`, plus the desktop entries, metainfo, icons and search-provider files.
- [ ] It builds in `mock` for the current Fedora release and Rawhide, and `rpmlint` has no errors.
- [ ] Steps for setting up the COPR project go in `packaging/README.md`. I'll create the COPR project myself.

**How to test:** `fedpkg --release f42 mockbuild` (or `mock -r fedora-42-x86_64 --rebuild <srpm>`), then install the rpm in a clean `fedora:42` container and run `packaging/linux-compat/check.sh` against it.
