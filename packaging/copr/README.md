# Fedora COPR

`skykeep.spec` builds Skykeep from source, with the same steps and files as
`packaging/arch/PKGBUILD`: `npm ci`, `npm run build`, `cargo build --release --locked`, then
the program, launchers, icons, metainfo and search providers under `/usr`. `.copr/Makefile` (at
the repository root) is COPR's "make srpm" method: it makes the source RPM from the commit COPR
cloned, with `Version` set from `src-tauri/tauri.conf.json`, so a tag needs no edit to the spec.

Why this route: COPR builds from source RPMs, and a spec that repackages the released `.rpm` is
a prebuilt binary, which COPR's rules leave unclear. A fully offline build would need every
crate and npm package vendored into the source RPM (hundreds of MB) and regenerated each
release. Instead the project allows network access during builds, a standard COPR project
setting, so npm and cargo fetch exactly what `package-lock.json` and `Cargo.lock` pin.

The build uses Fedora's own Rust (`rust-toolchain.toml` is for rustup and is ignored here),
which must be at least the workspace's `rust-version`.

Proven by `.github/workflows/packages.yml` (Actions → Packages → Run workflow): on Fedora 43 and
the latest Fedora, `make -f .copr/Makefile srpm` as COPR runs it, `dnf builddep`, `rpmbuild`,
then the package installed in a clean container of that Fedora and checked by
`packaging/linux-compat/check.sh` (libraries, GStreamer, HEIC, video, keyring, launch,
launchers).

## Account

A Fedora Account (FAS), free at <https://accounts.fedoraproject.org>. Log in to
<https://copr.fedorainfracloud.org> with it.

## Setting it up, once

1. **New Project**, named `skykeep`.
   - Chroots: `fedora-43-x86_64`, `fedora-44-x86_64` and every newer `fedora-*-x86_64`, and
     the same Fedoras' `-aarch64` chroots (Fedora Asahi Remix on Apple Silicon Macs).
   - Check **Enable internet access during builds** (Other options). Without it npm and cargo
     cannot fetch and the build fails.
   - Description: "Your iCloud on Fedora: Notes, Reminders, Calendar, Contacts, Drive and
     Photos. Not affiliated with Apple." Instructions: `sudo dnf copr enable
     <your FAS name>/skykeep && sudo dnf install skykeep`.
2. **Packages → New package → SCM**:
   - Package name `skykeep`; Clone URL `https://github.com/YashwanthKamireddi/Skykeep.git`;
     Committish empty; Subdirectory empty; Spec File `packaging/copr/skykeep.spec`.
   - Build SRPM with: **make srpm**.
   - Check **Auto-rebuild**.
   - Save, then **Rebuild** to build the current `main` once.
3. **The webhook**, so each tag builds itself: in COPR, Settings → Integrations, copy the
   GitHub webhook URL (`https://copr.fedorainfracloud.org/webhooks/github/<id>/<uuid>/`). In
   GitHub, the repository's Settings → Webhooks → Add webhook: that URL as Payload URL, content
   type `application/json`, "Let me select individual events" → only **Branch or tag
   creation**. Pushing a `v*` tag then builds that tag in every chroot.

## Each release

Nothing: the tag's webhook builds it, at the version in `tauri.conf.json`. Check the build
on the project's Builds page. To build by hand instead: Packages → skykeep → Rebuild, with the
tag as committish. When a new Fedora is released, add its chroot under Settings.

People install it with:

```sh
sudo dnf copr enable <your FAS name>/skykeep
sudo dnf install skykeep
```

iPhone photos (HEIC) and HEVC videos need RPM Fusion, exactly as for the release's `.rpm`
(the main README's "iPhone photos (HEIC) on Fedora or openSUSE").
