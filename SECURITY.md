# Security

Skykeep asks for your Apple Account password. That is a lot of trust to ask of an app made by
one person, so this page says exactly what it does with it: what it talks to, what it keeps,
and where. Every line here is true of the code in this repository; if one ever stops being
true, that is a bug, and a serious one.

## What Skykeep talks to

Apple, and to calendars you subscribe to — nothing else:

- `idmsa.apple.com` — signing in (Apple's own SRP sign-in and two-factor codes).
- `setup.icloud.com`, `www.icloud.com` — opening the session and finding your services.
- `*.icloud.com` service addresses Apple gives the session (CloudKit, Contacts, Calendar,
  iCloud Drive, Photos), `gateway.icloud.com` and `cvws.icloud-content.com` (downloads).
- `calendars.icloud.com` — Apple's public holiday calendars.
- The `.com.cn` equivalents of the above, for accounts in mainland China.
- The host of each calendar you subscribe to by address (Calendar ▸ + ▸ New Subscription…),
  and only once you have: a `webcal://` or `https://` address of a public calendar file.

This is enforced, not only promised: every network client Skykeep makes is built by
`quince_auth::apple_only()`, whose resolver refuses to look up any host outside `apple.com`,
`icloud.com` and `icloud-content.com` (and their `.com.cn` twins) — a request anywhere else
fails before a connection exists. A URL that names an address instead of a host
(`https://1.2.3.4/`) is never looked up, so each request's URL is checked too, a redirect's
included: anything that is not Apple's by name (or this computer, `127.0.0.1`) is refused the
same way. The test `crates/quince-auth/tests/apple_only.rs` holds it to that.

A subscribed calendar is fetched by a separate client, `quince_auth::listed_only()`, built for
each fetch with exactly the hosts of the addresses you subscribed to — the same two locks, a
resolver that looks up no other name and a check of every request's URL and redirect, so it
can reach those hosts and nothing else (`listed_tests` in `crates/quince-auth/src/lib.rs`).
Apple's client is not widened by it. A new install subscribes to nothing, so until you add a
subscription Skykeep contacts no host but Apple's. A subscription is read-only: nothing is
ever sent to it but the request for its file, and no Apple data goes with that request.

There are no Skykeep servers. There is no telemetry, analytics, crash reporting or update
check. Nothing is sent anywhere but Apple (and the requests for the calendars you subscribe
to), and only what is needed to show and change your
own data. The window itself cannot reach the network at all: its content security policy
(`src-tauri/tauri.conf.json`) lets it talk only to the app's own Rust side, which does all
talking to Apple.

Links you click in a note or an event open in your own browser, mail or map app; a photo's
place opens in this computer's own map app (a `geo:` link), never a website.

## What Skykeep keeps, and where

Everything is in your user's data folder (`~/.local/share/io.github.yashwanthkamireddi.Skykeep` on Linux),
which is readable by you alone (mode 700):

- **Your password, Apple Account and session tokens:** in the system keyring (GNOME
  Keyring, KWallet — the Secret Service), never in a file. The password is kept so a session
  that lapses can sign itself back in without asking; Apple asks for a code only when it
  stops trusting this computer.
- **Session cookies:** `cookies.json`, readable by you alone (mode 600).
- **Your library:** `library.db` (SQLite) — notes, reminders, events, contacts, the Drive
  and Photos listings — and caches of pictures and files you opened. These are not encrypted
  by Skykeep; use full-disk encryption, as you would for the same data on a Mac.

Signing out asks whether to keep a copy on this computer. Removing it deletes the library,
its caches and Skykeep's file-manager thumbnails. Another Apple Account signing in never sees
the last one's data: each account's library is set aside whole and brought back only for
that account.

## What runs on this computer

- **Videos** are played from a server on `127.0.0.1` only (WebKitGTK's player reads only
  http). It answers only a request that carries a secret made fresh at every start and given
  to Skykeep's window alone, is addressed to `127.0.0.1:<its port>` (so a web page that
  rebinds its own name to this computer is refused), and carries no web page's `Origin`. It
  sends no CORS headers. Another program or user on this computer, or a page in your browser,
  cannot read your videos from it.
- **Pictures** reach the window through Skykeep's own `quince://` scheme, which only
  Skykeep's windows can load. Every id in it is checked to be an id — letters, digits,
  dashes — before it names a file, so no request reads anything outside Skykeep's caches.
- **The window** loads only Skykeep's own bundled page, never a website, and may ask
  Tauri for no more than it uses: events, its version and a window title.
- **Files on this computer** are read or written only when you chose them: in an open or save
  dialog Skykeep shows from its Rust side, by dropping them on the window, or by opening them
  with Skykeep. Each choice is recorded there (`src-tauri/src/grants.rs`) for ten minutes, a
  save for one write only, and every command that reads or writes a file (uploads, imports,
  attachments, exports) refuses a path that is not one of them, compared with `..` and links
  resolved. So even script running in the window could not name a file of yours on its own.
- **A note's attachment** you open is saved under a cleaned-up name (never a path, a hidden
  file, or a name that reverses itself to hide its type) in your private data folder, not
  executable, and handed to the app this computer uses for it. A program, script, launcher or
  installer — by its name or its first bytes — is shown in its folder instead of opened, since
  opening it would run it, and a shared note can carry one from anyone.
- **The iCloud Drive and Photos folders** (FUSE) are mounted for you alone (no
  `allow_other`), with no set-user-id or device files, and nothing in them is executable.

## Advanced Data Protection

With Advanced Data Protection on and "Access iCloud Data on the Web" off, Apple does not
let any app outside your devices read your iCloud data, Skykeep included. Skykeep says so
plainly and shows the switch; it does not work around it.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on this repository (Security →
Report a vulnerability). Do not open a public issue for a security problem. You will get an
answer, and credit if you want it.
