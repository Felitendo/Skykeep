# Sync engine: Dropbox / Google Photos-grade roadmap (assessment 5 Nov 2026)

Read-only assessment of main, benchmarked with the real quince-core schema on synthetic data
(tmpfs, release build). Correctness is strong (outbox in the same transaction, idempotent sends,
moreComing/expired tokens handled, prune only after a complete walk, checksummed uploads,
conflicted copies, crash-safe caches). What breaks for heavy users is cost per operation: linear
in collection size, often under the single writer lock.

## Blockers, worst first
1. Photo thumbnail fetches take the writer lock (src-tauri/src/photos.rs ~391, ~847); a sync round
   holds it 2.9–8.3 s and photo_counts() costs 4.4 s at 1M, up to 10 rounds per refresh.
2. Every mutation in the mounted Drive folder re-lists the whole folder synchronously
   (fuse.rs:304 change, :496 relist): 1.4 s at 10k items, 19.6 s at 100k → rm -rf of 10k files ≈ hours.
3. Opening a Drive file read-write without O_TRUNC blocks a FUSE thread for the whole download
   (fuse.rs:749–787, rt.block_on); only 4 FUSE threads (fuse.rs:1884).
4. Re-storing unchanged rows rewrites the FTS index (ON CONFLICT DO UPDATE fires AFTER UPDATE OF
   triggers, schema.rs:738): 50k unchanged notes = 27 s. Fix: WHERE change_tag IS NOT excluded…
5. Daily full walk of every service (refresh.rs:89 FULL_SYNC_EVERY): ~2.2M records / ~4,400
   requests a day at 1M photos; new photos wait for the walk.
6. Notes/Reminders walks cap at 100k records and buffer everything in memory (cloudkit.rs:18,21);
   above the cap a full walk never completes, deletions behind an expired token never pruned.
7. Release builds keep no sync log (quiet! debug-only).

## Measured (100k / 1M photos)
put_photos first sync 5.6 s / 101 s · re-store 50k 6.5 / 8.3 s · photo_index 109 ms / 1.99 s ·
index JSON to window 5.1 MB / 50.9 MB · photo_page middle 56 ms / 994 ms (OFFSET) ·
photo_counts 257 ms / 4.4 s · photo_months 206 ms / 2.9 s · search_photos (LIKE) 290 ms / 4.3 s ·
FTS search 8 / 533 ms · DB 87 / 694 MB. Drive folder 10k/100k: store 1.0/10.7 s, relist 1.4/19.6 s,
children read 89 ms/1.27 s. 50k notes: first store 14.5 s, unchanged re-store 27.3 s, DB 803 MB.
Recents/Favorites/Videos have no index (temp B-tree sort).

## Roadmap (independently shippable, each with its scale/chaos test)
1. Stop FTS churn (upsert WHERE / trigger WHEN). Test: unchanged re-put 27 s → <1 s.
2. Thumbnail fetches off the writer lock (Reader connection); no photo_counts in the pass report.
   Test: 1M DB + concurrent pass, thumbnail p99 < 50 ms.
3. Photo indexes (recents/favorites/videos), keyset paging on (taken_at,id), maintained counts
   table, FTS photo search. Test: every call < 20 ms at 1M.
4. Mounted folder: async optimistic mutations, no relist per change; open-for-write without
   block_on. Test: rm -rf / mv of 10k files < 60 s, constant relists.
5. Privacy-safe rotating sync.log (JSONL), Sync Activity panel (pending/active/paused/failed,
   Pause/Cancel), health chip.
6. Streamed zone cursors for every service (ZoneCursor table, per-page transactions, no cap) and
   a separate reconcile cursor with seen_epoch pruning; drop the daily full walk; desiredKeys for
   reconcile; Contacts per-card diff; bulk deletes from the local mirror. Test: 2.5M-record
   wiremock with token expiry mid-walk, RSS cap, no transaction > 200 ms.
7. Batched records/lookup (200) for photo links + viewport priority scheduler (visible > overscan
   > idle), cancellable.
8. Windowed photo grid (recycled tile pool) + compact binary index (~4 MB at 1M vs 51 MB JSON).
   Test: Playwright fling across 1M, bounded DOM, ≥ 55 fps.
9. Outbox v2: next_at, depends_on, group_id, lanes; per-record ordering; 200-op CloudKit batches;
   Drive and Photos operations moved into the outbox; per-change backoff + jitter; poison items
   in a Pending panel. Test: seeded model + kill -9 chaos (no loss/dup, per-record order).
10. TransferManager: durable transfers table; resumable parallel Range downloads (4–8 chunks),
    FUSE read-ahead, version pinning; journaled uploads with per-file retry; priority classes;
    bandwidth token bucket; verify downloads with Apple's salted SHA-1. Test: fault-injecting
    proxy, 5 GB sparse files, resume not restart.
11. Cache index table (cache_entries): O(1) accounting, free-space floor, refuse pins without
    room, sharded evictable thumbnail tiers, pinned "keep offline" albums, checksum verification.
12. Video through the sparse chunk cache with seek (Range to Apple).
13. Resilience: shared AppleBudget (token bucket + per-host breaker), logind PrepareForSleep
    listener, reauth barrier that pauses transfers. Test: 5xx/429 storms + simulated suspend.
14. Windows Drive/Photos folder via the Cloud Files API (cfapi) on quince-vfs's Remote/Store/Cache
    split; longPathAware manifest.
Also: journal each step of a Drive replace (sent → trashed → renamed); notes > 700 KB via
TextDataAsset; index photo_album_items(asset).

## Unknowns (one live check each)
package_token downloads (bug-hunt commit b26489d addresses it) · Range on Photos video URLs ·
Live Photo pairing via the web uploader · partial Contacts changes · Shared Library zones
(SharedSync-*) · moov-atom placement in iPhone videos · crash mid-Drive-replace recovery.
