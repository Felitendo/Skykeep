# Privacy policy

Skykeep is a free, open-source app that shows your own iCloud data on Windows and Linux. It is
made by Yashwanth Kamireddi, an independent developer, and is not affiliated with Apple.

## What Skykeep handles

To show your iCloud, Skykeep signs in to your Apple Account and reads and changes your iCloud
Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos, as you use it. That is personal
information, so here is exactly where it goes.

- **Only between your computer and Apple.** Skykeep talks to Apple's iCloud servers directly.
  There are no Skykeep servers and no Skykeep account, so nothing you do in Skykeep is sent to
  its developer or to anyone else.
- **Your password** is never sent anywhere, not even to Apple: sign-in uses Apple's own SRP
  protocol and Apple's two-factor codes. Your password and session are kept in your system's
  keyring (Credential Manager on Windows, GNOME Keyring or KWallet on Linux) so Skykeep can
  stay signed in. Signing out erases them.
- **Your library** (a copy of your iCloud data, so the apps open instantly and work offline)
  is kept in your own user folder on your computer. Signing out asks whether to keep it or
  delete it.

## What Skykeep doesn't do

No analytics, no telemetry, no crash reporting, no advertising, and no tracking of any kind.
Skykeep doesn't sell, share or upload your data.

## Other connections

- **Calendar subscriptions:** only the calendars you subscribe to yourself, to fetch them.
- **Updates:** a copy installed from the Windows installer, the `.msi` or the AppImage asks
  GitHub for the latest version's details about once a day, sending nothing about you (you can
  turn this off in Settings ▸ Software Update). Copies from the Microsoft Store, the Snap Store,
  Scoop, COPR, a `.deb` or an `.rpm` are updated by those stores and never contact GitHub
  themselves. GitHub, like any website, sees the request come from your network address.

Every one of these is enforced in the code; [SECURITY.md](SECURITY.md) lists each address
Skykeep can reach and the code that holds it to that.

## Contact

Questions about privacy: open an issue at
https://github.com/YashwanthKamireddi/Skykeep/issues

Last updated: 4 November 2026
