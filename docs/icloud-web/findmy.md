# Find My (icloud.com `find-my-iphone`), static pass 2026-10-19
Bundle: `~/.cache/quince-icloud-re/find-my-iphone/raw/main.js` (minified, single line; quoted by string).

## Endpoints (VERIFIED-IN-CODE)
Base: `urlForServiceName("findme") + "fmipservice/client/web"`, the `findme` host from the webservices map.
All calls are `POST {base}/{action}` with a JSON body.
- Every body starts from `generateBaseRequestBody`:
  `{serverContext?: <hash from the last reply>, clientContext: {appName: "iCloud Find (Web)", appVersion: "2.0", apiVersion: "3.0", deviceListVersion: 1, fmly: true, timezone: "US/Pacific" (or local), inactiveTime: ms since last use}}`.
- `initClient`: the first device list, with `serverContext` returned.
- `refreshClient`: polled; returns the full device list (`processFullResp`).
- `playSound`: `{device: <id>, subject: "<localised 'Find My … alert'>", userAction?, channels? (AirPods), userConfirmedSafetyWarning?}`.
- `lostDevice`, `lostDeviceWithFamilyAuth`: Lost Mode (message, phone number, passcode).
- `remoteWipe`, `remoteWipeWithFamilyAuth`, `remoteWipeWithUserAuth`: Erase. Reply code `"2403"` is handled specially.
- `remove`, `removeWithFamilyAuth`: remove a device from the account.
- `POST /fmipwebauthenticate` returns a short-lived token that is required **before** any destructive action (Lock, Erase).
  The UI refuses with "returned empty or null token" when it is missing.
- Device model keys: `deviceClass`, `features.LST` (Lost Mode supported), `snd` (play-sound activity),
  `lostDevice` (Lost Mode activity), `owner` (family device), plus location and battery fields from the device list.
- Sessions: `SessionStatusCode.SESSION_INVALID` and others mean the client signs in again.

## Feasibility for Quince
Read-only device locations, battery and Play Sound need only `initClient`/`refreshClient` plus `playSound`. They are small and low-risk.
Lock, Erase and Remove are destructive and need `fmipwebauthenticate` (a password re-prompt). Leave them out, or
gate them behind an explicit confirmation. AirTags/Items and People are not in this bundle; they are only on Apple devices.

## Needs live capture
The exact device-list reply shape (location fields, timestamps, battery), the `refreshClient` cadence, and the `fmipwebauthenticate` body.
