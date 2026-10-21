# Media for the README

Every image the top-level `README.md` shows lives in this folder. Until a file exists, its spot in the README shows a broken image, so capture these before the repository goes public.

> [!IMPORTANT]
> **Use a demo account only.** Every screenshot and clip uses synthetic data: invented notes, reminders, events, contacts and files. No real names, emails, phone numbers, account pictures, and nothing that identifies a person or a school, including in window titles, the tray, notifications and file paths. Check every frame of each GIF.

## General settings

- **Window size:** 1440 × 900 logical pixels (the app window, not the whole screen), captured at 2× scale, so files are **2880 × 1800**. Then export a copy scaled to **1440 × 900** and commit that one (keeps the repo small, still sharp on GitHub).
- **Capture the window only,** with its own rounded corners and shadow, on a plain background. No desktop wallpaper or other apps, except where noted.
- **Light and dark:** set the appearance in Settings → Appearance (Light / Dark), not by theme hacks.
- **Format:** PNG for screenshots, optimised (`oxipng -o 4` or `pngquant --quality 80-95`). Aim for under 600 KB each.
- **Cursor:** hidden in screenshots unless it explains something.
- **Time and date:** a plausible weekday morning (for example 9:41), with the calendar on a week that has events.

## Screenshots the README expects

| File | App | Mode | What to show |
|---|---|---|---|
| `notes-light.png` | Notes | Light | A note with a heading, a checklist (some ticked), a small table and a picture. Folders in the sidebar, a few pinned notes in the list. |
| `notes-dark.png` | Notes | Dark | Same scene. |
| `reminders-light.png` | Reminders | Light | **Today** with 5–7 reminders: one with a subtask, one flagged, one with a due time, one priority. Lists with colours in the sidebar. |
| `reminders-dark.png` | Reminders | Dark | Same scene. |
| `calendar-light.png` | Calendar | Light | **Week** view with 8–12 events in three or four calendar colours, a reminder on the schedule, and a birthday. |
| `calendar-dark.png` | Calendar | Dark | Same scene. |
| `drive-light.png` | Drive | Light | Skykeep Drive in grid view with thumbnails, **beside** a file manager window (Nautilus or Dolphin) open on `~/iCloud Drive` showing the same files. Capture both windows together on a plain background. |
| `drive-dark.png` | Drive | Dark | Same scene, file manager also in dark mode. |
| `contacts-light.png` | Contacts | Light | A card with a (stock, licence-free or generated) photo, several phones/emails, an address and a birthday; lists in the sidebar. |
| `photos-dark.png` | Photos | Dark | The library by month with a mix of photos, one video and one Live Photo badge. Use licence-free images. Labelled **preview** in the README. |
| `settings-light.png` | Settings | Light | Account (demo name and generated avatar), storage bar, per-app sync times with Sync Now, appearance. This is the trust screenshot. |
| `signin-light.png` | Sign-in | Light | The two-factor code step, showing the flow is Apple's own sign-in. Demo email only. |

Optional, for later:

| File | What |
|---|---|
| `logo-dark.png` | A dark-mode variant of the mark, if the transparent mark ever reads poorly on GitHub's dark theme. Swap it into the first `<source>` of the README hero. |
| `windows-light.png` | Skykeep on Windows 11, once Windows leaves beta. |
| `social-preview.png` | The 1280 × 640 social preview (see `docs/github-setup.md`). Kept here as the source; it's uploaded in GitHub's settings, not shown in the README. |

## GIFs

All GIFs: **under 8 seconds, looping, 1200 px wide (or less), under 5 MB**, 15–20 fps. Record as video first (OBS or `wf-recorder` at 60 fps), then convert:

```sh
ffmpeg -i in.mp4 -vf "fps=18,scale=1200:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4" -loop 0 out.gif
gifsicle -O3 --lossy=60 out.gif -o out.gif
```

| File | Used where | What |
|---|---|---|
| `hero-sync.gif` | README hero (required) | The magic moment. Storyboard below. |
| `offline.gif` | Optional, release notes | Wi-Fi off, edit a reminder, Wi-Fi on, it syncs; the pending indicator shows the state. |
| `reminders.gif` | Optional, release notes | Tick a reminder, make a subtask with Tab, a desktop notification rings with Complete and Snooze. |
| `drive-folder.gif` | Optional, release notes | Drag a file into `~/iCloud Drive` in the file manager; it appears in Files on the iPhone. |
| `calendar-keys.gif` | Optional, release notes | Calendar driven by keys: Ctrl+Shift+T Go to Date, Ctrl+1–4 views, T for today. |

### Hero GIF storyboard (`hero-sync.gif`, about 7 s)

Real devices, no mockups: a real iPhone filmed on a tripod (or its screen mirrored), beside the desktop. Composite side by side: **iPhone on the left (about 30% of the width), Skykeep Notes on the right**, on a plain background, 1200 × 600.

| Time | Shot |
|---|---|
| 0.0–1.0 s | Both screens at rest. The same note ("Weekend plans") is open on the iPhone and in Skykeep, light mode. |
| 1.0–3.0 s | On the iPhone, type a new checklist line: "Book the train". |
| 3.0–4.0 s | About a second later, the line appears in Skykeep. No cut: one continuous take, so the timing is real. |
| 4.0–5.5 s | In Skykeep, tick "Book the train". |
| 5.5–6.5 s | The tick appears on the iPhone. |
| 6.5–7.0 s | Brief hold, then loop. |

Optional caption bar under the frame: *"Edit on your phone. See it here in a second."* Keep it out of the GIF if it makes the file too large; the README's alt text carries the message.

## After capturing

1. Put the files here with exactly the names above.
2. Run the privacy check on every image: zoom in on sidebars, title bars, avatars and notifications.
3. Strip metadata: `exiftool -all= -overwrite_original docs/media/*.png docs/media/*.gif`.
4. Commit only this folder's media and this README (`git add docs/media`). The rest of `docs/` holds internal notes that are not meant for the public repository.
