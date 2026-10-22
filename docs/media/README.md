# Media for the README

Every image the top-level `README.md` shows lives here. They're taken from the real interface by `.uiverify/readme-shots.mjs`, using the demo library in `.uiverify/readme-fixture.mjs` and the pictures drawn by `.uiverify/readme-pictures.mjs`.

```sh
node .uiverify/readme-shots.mjs                  # every still, plus the hero animation
node .uiverify/readme-shots.mjs notes calendar   # only those scenes
PORT=4330 node .uiverify/readme-shots.mjs        # preview on another port
```

Needs ImageMagick and ffmpeg. Each still is a 1200 × 760 window drawn at 2x, framed, then written 1760 px wide (sharp at the README's 880 on a 2x screen), stripped, and optimised with pngquant and oxipng when installed, or the same engines through `uv`. The script waits for fonts and every picture in view before it takes a still.

## Files

| File | Shows |
|---|---|
| `notes-light.png`, `notes-dark.png` | Folders, pinned notes, and a note with a #tag, a link, a picture, a checklist and a table |
| `reminders-light.png`, `reminders-dark.png` | A list with sections, subtasks, a priority, flags, tags, a link and a place; smart lists and tags beside it |
| `calendar-light.png`, `calendar-dark.png` | A week with four calendars, an all-day trip, a birthday and reminders |
| `contacts-light.png`, `contacts-dark.png` | Contacts with photos, lists, and a full card with related names |
| `drive-light.png`, `drive-dark.png` | A folder in grid view with picture and document thumbnails |
| `photos-light.png`, `photos-dark.png` | The library by month, with videos, favourites and albums |
| `settings-light.png`, `settings-dark.png` | Account, storage and sync |
| `signin-light.png`, `signin-dark.png` | Signing in with a demo Apple Account |

## Rules for every image

- **Demo data only.** Invented names, `example.com` emails and 555-01xx phone numbers. Nothing real from anyone's library, no school or workplace names.
- **Pictures** are all drawn by the harness (`readme-pictures.mjs`): photos, thumbnails, the note's picture and the contacts' illustrated portraits. No photo file is read. Never a personal photo.
- Look over every frame before committing: sidebars, avatars, toasts and window titles.
