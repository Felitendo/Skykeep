# Media for the README

Every image the top-level `README.md` shows lives here. They're taken from the real interface by `.uiverify/readme-shots.mjs`, using the demo library in `.uiverify/readme-fixture.mjs` and the pictures drawn by `.uiverify/readme-pictures.mjs`.

```sh
node .uiverify/readme-shots.mjs                  # every still, plus the hero animation
node .uiverify/readme-shots.mjs notes calendar   # only those scenes
PORT=4330 node .uiverify/readme-shots.mjs        # preview on another port
```

Needs ImageMagick and ffmpeg. Each still is a 1200 × 760 window drawn at 2x, framed, then written 1760 px wide (sharp at the README's 880 on a 2x screen) as WebP at quality 90, about half the size of an optimised PNG with the same look. The script waits for fonts and every picture in view before it takes a still.

## Files

| File | Shows |
|---|---|
| `notes-light.webp`, `notes-dark.webp` | Folders, pinned notes, and a note with a #tag, a link, a picture, a checklist and a table |
| `reminders-light.webp`, `reminders-dark.webp` | A list with sections, subtasks, a priority, flags, tags, a link and a place; smart lists and tags beside it |
| `calendar-light.webp`, `calendar-dark.webp` | A week with four calendars, an all-day trip, a birthday and reminders |
| `contacts-light.webp`, `contacts-dark.webp` | Contacts with photos, lists, and a full card with related names |
| `drive-light.webp`, `drive-dark.webp` | A folder in grid view with picture and document thumbnails |
| `photos-light.webp`, `photos-dark.webp` | The library by month, with videos, favourites and albums |
| `settings-light.webp`, `settings-dark.webp` | Account, storage and sync |
| `backup-light.webp`, `backup-dark.webp` | Back Up iCloud part way through, Photos and iCloud Drive into a drive |
| `signin-light.webp`, `signin-dark.webp` | Signing in with a demo Apple Account |

## Rules for every image

- **Demo data only.** Invented names, `example.com` emails and 555-01xx phone numbers. Nothing real from anyone's library, no school or workplace names.
- **Pictures** are all drawn by the harness (`readme-pictures.mjs`): photos, thumbnails, the note's picture and the contacts' illustrated portraits. No photo file is read. Never a personal photo.
- Look over every frame before committing: sidebars, avatars, toasts and window titles.

## Android

`android/` holds the Android app's stills: the app over an invented demo account (the same Jamie Rivera library as above), signed in and offline, its photos drawn on the phone. The status bar is in Android's demo mode (9:41, full battery, no notifications); each still is the whole screen written 720 px wide as WebP at quality 90.

| File | Shows |
|---|---|
| `android/home-light.webp`, `android/home-dark.webp` | The account and each service's sync, with Drive's storage |
| `android/reminders-light.webp`, `android/reminders-dark.webp` | A list with sub-tasks, priorities, flags and a repeat |
| `android/notes-light.webp`, `android/notes-dark.webp` | Notes grouped by when they changed, pinned first |
| `android/note-light.webp`, `android/note-dark.webp` | A note with headings, a link, a checklist and a list |
| `android/photos-light.webp`, `android/photos-dark.webp` | The library by month |
| `android/viewer-light.webp`, `android/viewer-dark.webp` | A photo open |
| `android/drive-light.webp`, `android/drive-dark.webp` | A folder of pictures and documents |
| `android/settings-light.webp`, `android/settings-dark.webp` | Photo backup, how often it syncs, privacy |
| `android/welcome-light.webp`, `android/welcome-dark.webp` | The first screen |
| `android/signin-light.webp`, `android/signin-dark.webp` | Signing in with the demo Apple Account |
