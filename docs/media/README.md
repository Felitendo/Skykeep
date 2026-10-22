# Media for the README

Every image the top-level `README.md` shows lives here. They are made from the real interface by `.uiverify/readme-shots.mjs`, on an invented demo library (`.uiverify/readme-fixture.mjs`) with drawn pictures (`.uiverify/readme-pictures.mjs`).

```sh
node .uiverify/readme-shots.mjs                  # every still, plus the hero animation
node .uiverify/readme-shots.mjs notes calendar   # only those scenes
PORT=4330 node .uiverify/readme-shots.mjs        # preview on another port
```

Needs ImageMagick and ffmpeg. PNGs are written 1600 px wide, stripped, and optimised with pngquant and oxipng when installed, or the same engines through `uv`.

## Files

| File | Shows |
|---|---|
| `notes-light.png`, `notes-dark.png` | A note with a picture, a checklist and a table |
| `reminders-light.png`, `reminders-dark.png` | Today, with coloured lists, priorities and flags |
| `calendar-light.png`, `calendar-dark.png` | A week with four calendars, reminders and a birthday |
| `contacts-light.png`, `contacts-dark.png` | A full contact card and lists |
| `drive-light.png`, `drive-dark.png` | A folder in grid view with thumbnails |
| `photos-light.png`, `photos-dark.png` | The library by month, with a video and favourites |
| `settings-light.png`, `settings-dark.png` | Account, storage and sync |
| `signin-light.png`, `signin-dark.png` | Signing in with a demo Apple Account |

## Rules for every image

- **Demo data only.** Invented names, `example.com` emails and 555-01xx phone numbers. Nothing real from anyone's library, no school or workplace names.
- **Pictures** are drawn by the harness, plus four Unsplash images from the harness's `UNSPLASH` list, resized with metadata stripped. Never a personal photo.
- Look over every frame before committing: sidebars, avatars, toasts and window titles.
