# Security

Quince asks for your Apple Account password. That is a lot of trust to ask of an app made by
one person, so this page says exactly what it does with it: what it talks to, what it keeps,
and where. Every line here is true of the code in this repository; if one ever stops being
true, that is a bug, and a serious one.

## What Quince talks to

Apple, and nothing else:

- `idmsa.apple.com` — signing in (Apple's own SRP sign-in and two-factor codes).
- `setup.icloud.com`, `www.icloud.com` — opening the session and finding your services.
- `*.icloud.com` service addresses Apple gives the session (CloudKit, Contacts, Calendar,
  iCloud Drive, Photos), `gateway.icloud.com` and `cvws.icloud-content.com` (downloads).
- `calendars.icloud.com` — Apple's public holiday calendars.
- The `.com.cn` equivalents of the above, for accounts in mainland China.

There are no Quince servers. There is no telemetry, analytics, crash reporting or update
check. Nothing is sent anywhere but Apple, and only what is needed to show and change your
own data. The window itself cannot reach the network at all: its content security policy
(`src-tauri/tauri.conf.json`) lets it talk only to the app's own Rust side, which does all
talking to Apple.

Links you click in a note or an event open in your own browser, mail or map app; a photo's
place opens in this computer's own map app (a `geo:` link), never a website.

## What Quince keeps, and where

Everything is in your user's data folder (`~/.local/share/dev.quince.client` on Linux),
which is readable by you alone (mode 700):

- **Your password, Apple Account and session tokens:** in the system keyring (GNOME
  Keyring, KWallet — the Secret Service), never in a file. The password is kept so a session
  that lapses can sign itself back in without asking; Apple asks for a code only when it
  stops trusting this computer.
- **Session cookies:** `cookies.json`, readable by you alone (mode 600).
- **Your library:** `library.db` (SQLite) — notes, reminders, events, contacts, the Drive
  and Photos listings — and caches of pictures and files you opened. These are not encrypted
  by Quince; use full-disk encryption, as you would for the same data on a Mac.

Signing out asks whether to keep a copy on this computer. Removing it deletes the library,
its caches and Quince's file-manager thumbnails. Another Apple Account signing in never sees
the last one's data: each account's library is set aside whole and brought back only for
that account.

## Advanced Data Protection

With Advanced Data Protection on and "Access iCloud Data on the Web" off, Apple does not
let any app outside your devices read your iCloud data, Quince included. Quince says so
plainly and shows the switch; it does not work around it.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on this repository (Security →
Report a vulnerability). Do not open a public issue for a security problem. You will get an
answer, and credit if you want it.
