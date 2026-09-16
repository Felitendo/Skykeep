# Quince

Your iCloud, on your own computer.

Quince signs into iCloud once and keeps a copy of your calendar, reminders, notes, contacts
and Drive on your machine. Everything opens straight away and works with no connection, and
what you change here reaches your phone.

It is a native Linux application — Rust underneath, a real window, no browser wrapper.

<!-- A screenshot belongs here once there is a public repo to host it. -->

## What works

| | Read | Write | Syncs to your devices |
|---|---|---|---|
| **Calendar** | yes | create, edit, delete | yes |
| **Reminders** | yes | lists, subtasks, repeats, priority, alerts | yes |
| **Notes** | yes | create, edit, move, delete | yes |
| **Contacts** | yes | create, edit, delete | yes |
| **iCloud Drive** | yes | upload, download, rename, move, delete | yes |
| Photos | not yet | | |
| Mail, Find My | not yet | | |

## Installing

**Arch, CachyOS, EndeavourOS, Manjaro**

```sh
cd packaging/arch && makepkg -si
```

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
driven in a real browser: every screen screenshotted and measured, the keyboard path walked,
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

**Advanced Data Protection.** With it switched on, the services above are encrypted the same
way and Quince cannot read them either. It says so rather than nudging anybody to turn a
security feature off.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE).
