# iCloud Drive and Photos as folders — design (2026-10-20)

The goal: other apps (file managers, terminals, LibreOffice, image editors) see iCloud Drive
and iCloud Photos as ordinary folders, on Linux (FUSE) and Windows (Cloud Files API). Files
download on demand, writes feel local, and nothing is ever lost or silently overwritten.

## Prior art studied

- **rclone mount**, the reference VFS (virtual filesystem layer). It has iCloud Drive and
  iCloud Photos backends, the latter "Tier 4, experimental, read-only".
  - Directory cache with a poll interval.
  - Cache modes off/minimal/writes/full. "full" keeps sparse files with known byte ranges
    and reads ahead.
  - Write-back: upload on close after a 5 s idle delay, up to 4 in parallel; retried with
    backoff; uploaded on the next run if not done before exit.
  - LRU eviction by max-size, max-age and min-free-space; open files are never evicted.
  - attr-timeout of 1 s, because a longer one risks truncated reads after a remote change.
  - Two processes must never share a cache.
  - systemd: Type=notify, absolute paths. AppArmor on Ubuntu can block fusermount3.
- **icloud-linux** (IsmaeelAkram, Python FUSE, 149 stars).
  - Metadata in SQLite, kept separate from hydrated content (content downloaded locally).
  - One hydration transition for lazy reads and bulk "make available offline".
  - Writes durable locally first, then a background queue that survives restarts.
  - Tombstones so deletes are not resurrected.
  - Conflicts kept and shown, never resolved by magic.
  - Filename normalization repair; a CLI for status.
- **Windows: Cloud Files API (cfapi)**, used by OneDrive, iCloud for Windows and Nextcloud.
  - A sync-root registration; placeholders; hydration callbacks (FETCH_DATA).
  - Pin/unpin ("Always keep on this device" / "Free up space").
  - Status icons in Explorer.
  - Rust: the `cloud-filter` crate (0.0.6, young — audit before use).
- **GLib (glocalfile.c):** `is_remote_fs_type` treats only nfs, nfs4, cifs, smb, smb2 and
  fuse.sshfs as remote.
  - A `fuse.quince` mount counts as local, so GNOME Files thumbnails every file it lists,
    which means downloading each one in full. KDE is similar.

## Decisions

1. **One engine, two front ends.** A platform-free core (`quince-vfs`) covers the inode
   table, metadata, hydration, cache, write queue and conflicts. Linux uses `fuser` 0.18;
   Windows uses cfapi later.
2. **Metadata from the library DB, never the network, on the kernel path.**
   - lookup, getattr and readdir are answered from SQLite.
   - A folder never opened before is fetched once, asynchronously, with a timeout. Its
     listing is then cached and refreshed by Apple's push and the existing poll.
   - The kernel is told of remote changes with `notify_inval_entry` / `notify_inval_inode`.
3. **Content: full mode, sparse cache files with ranges** (as in rclone).
   - The first read of a range fetches it with HTTP Range requests, chunking and read-ahead.
   - Whole-file hydration only when asked ("Keep on this computer").
   - LRU eviction with a size budget; pinned and open files are never evicted.
4. **Writes: local first.**
   - Written to the cache file; on close, plus a 5 s idle delay, an upload job goes into the
     existing persistent outbox (it survives restarts).
   - Retries use backoff. Uploads verify with retrieveItemDetails, as Quince's Drive does
     now.
   - Renames, moves, mkdir and delete become outbox jobs keyed by Apple's drivewsid, with
     tombstones until Apple confirms.
5. **Conflicts.** When both sides changed, keep both: "name (conflict on <host> <date>).ext".
   Never overwrite.
6. **Thumbnails without downloads.**
   - Write freedesktop thumbnail-cache entries from Apple's own thumbnails, under
     ~/.cache/thumbnails/{normal,large}/<md5(file URI)>.png with Thumb::URI and Thumb::MTime.
   - Additionally, for a file whose content isn't local, a read from a known thumbnailer
     process is refused (requester pid from FUSE, process name from /proc/<pid>/comm).
     The list: gnome-thumbnail-factory children, totem-video-thumbnailer,
     evince-thumbnailer, ffmpegthumbnailer, gst-thumbnailer, KIO thumbnail workers.
7. **Photos folder: read-only first.**
   - Library/<year>/<month>/, Albums/<album>/, Favorites/, Videos/.
   - Originals on demand, with the original file name and dates.
   - Later, possibly: copying into an album folder uploads (Quince already uploads).
8. **Safety.**
   - The mount point is owner-only. No allow_other.
   - A stale mount ("Transport endpoint is not connected") is detected and lazily unmounted
     at start; AutoUnmount is set.
   - A crash never loses a write: the outbox and cache are on disk before close() returns.
   - Never two engines on one cache: a lock file.
9. **Names.** Apple's names are NFC-normalized for lookup. Names illegal on the platform are
   mapped reversibly. Case: Linux is case-sensitive and iCloud is case-insensitive, so two
   names that differ only by case are surfaced as a conflict, never silently merged.

## Steps (one per tick, each proven on throwaways in a scratch mount)

1. Core plus a read-only Drive mount on Linux: list, stat, read with on-demand ranges.
2. Thumbnail-cache filling and the thumbnailer guard; checked with Nautilus' own thumbnailer.
3. Writes: create, write, truncate, rename, mkdir, delete through the outbox; conflict
   handling.
4. Eviction, pinning ("Keep on this computer"), settings UI, a status line in the app.
5. Photos folder (read-only).
6. Windows cfapi front end.

## Proven live

- 2026-10-20 (probe/examples/drive_range_read.rs, read-only): Drive download URLs honour HTTP
  Range requests. Two ranges of an 11,710-byte docx each came back "206 Partial Content" with
  the correct Content-Range, and both matched the same bytes from a whole download. Reading
  ranges on demand works.
- 2026-10-20 (probe/examples/drive_mount_live.rs): his real iCloud Drive mounted read-only in a
  scratch folder.
  - Root listing: 0.5 s the first time, 4 ms after.
  - A never-opened folder (Downloads): 1.2 s once.
  - An 11,710-byte .docx read byte-exact in 1.4 s; `file` and `unzip` confirm a valid Word
    document.
  - A thumbnailer-named process was refused (EACCES) before the file was local, and allowed
    once it was.
  - Folder dates (Apple lists none) now show the mount time instead of 1970.

## Separate folders, no extra installs (his requirement, 2026-10-20)

- iCloud Drive and iCloud Photos are two folders, each with its own switch, each depending
  only on its own service.
  - A user with iCloud Photos off gets Drive and is never asked about Photos, and the
    reverse. They share the engine, cache and settings.
- Linux needs only FUSE: the kernel module plus fusermount3 (fuse3). Mainstream desktops ship
  it, because gvfs/GNOME and KIO depend on it. Quince checks for it, and Settings says what is
  missing in one line if it is absent.
- Windows needs only the Cloud Files API, part of Windows 10 1709 and later.
- Flatpak: a sandbox cannot mount FUSE without extra permissions. Either the Flatpak asks for
  them, or the folders are offered in native packages only (deb/rpm/AUR/AppImage). To settle
  at packaging.
- 2026-10-20: new folder, rename, move and delete proven live on his Drive through the mount,
  inside a throwaway folder, with plain shell commands (mkdir, mv, rmdir).
  - rmdir of a non-empty folder was refused with ENOTEMPTY.
  - The 3 throwaways were deleted forever from Recently Deleted afterwards; a re-check found 0.
- 2026-10-20: writes proven live on his Drive in a throwaway folder: a new file, an append, and
  an editor's save (GLib-style temporary file renamed over the original) arriving while the
  append's send was still under way.
  - The first live run left "note 2.txt" beside the note. The append's three steps (send,
    delete the old version, rename the new one) were interrupted by the save, which took the
    free name.
  - Fixed with one send at a time per name (the per-name upload lock). The regression test was
    slowed to Apple's real pace and fails without the fix and passes with it.
  - Re-run live: one file, and a fresh mount with an empty cache read the editor's bytes from
    Apple. The vim .swp was never sent. Throwaways were cleared from Recently Deleted; 0 left.
- 2026-10-20: iCloud Photos folder (read-only) checked in the real app, isolated, on a copy of
  his library.
  - Layout: Library/<year>/<MM Month>, Albums (he has none), Favorites (1), Videos (17).
  - Two cached originals read byte-identical (SHA-256) to the app's cache.
  - touch and rm were refused.
  - 36 of 36 photos in a month showed GLib-valid thumbnails from the app's own, with nothing
    downloaded.
  - SIGTERM left nothing mounted.
  - Live Photo motion (.MOV) is not shown yet: its size is not known until it is fetched.
