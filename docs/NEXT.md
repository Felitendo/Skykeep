# Next (2026-11-05, ~13:00 IST) — 0.1.5 released (verify it published)

main b235af9 "Skykeep 0.1.5" (45 commits: store automation, bug hunt ×12, data-safety, sign-in + ADP
+ one session state model + Photos zone discovery, folder download, limits ×10). Gate: 148/150 in a
netns (FUSE + one timing check failed only there); outside it: all 1,275 Rust tests green, notes-long
17 ms. CI green. Tag v0.1.5 pushed → release run 41985353284.

FIRST next session:
1. If the v0.1.5 draft isn't published: verify (17+ assets incl. Skykeep-0.1.5-x86_64.flatpak,
   latest.json = 0.1.5, checksums, skykeep.exe imports no MSVCP/VCRUNTIME) then
   `gh release edit v0.1.5 --notes-file ~/Documents/Skykeep/release-notes-0.1.5.md --draft=false --latest`.
   Publishing triggers stores.yml (Snap, Chocolatey, Scoop push, winget via Komac, MSIX artifact).
2. Check stores.yml run: first real run of it — unproven parts listed in the agent's report.
   winget job fails until PR #470370 is merged (expected).
3. Reply on issues #4 and #5 (fixed in 0.1.5; #5: test with SKYKEEP_LOG=1 if still failing).
4. His dev app = target/debug/quince-app at b235af9 (rebuild after every batch: memory note).

Then:
- Reel: render at 2× (scripts/render-narrated.sh, ~1–1.5 h CPU), then `node scripts/qc-layout.mjs 15 0.25`
  (now also fails on text overlapping text). Changes made, not rendered: platforms as equal type (no Tux),
  shells removed (real note instead), Drive line true on both OSes, no "Not even to Apple", anti-flicker
  (render scale 2 + lanczos). out/real/* files are the OLD version — don't post.
- Flaky test: folder_download 100k bounded-memory measures whole-process RSS → measure its own.
- Folder download small items: Download button in Drive toolbar; "name 2" everywhere; resume by version
  not size; real full-disk test (tiny tmpfs as Downloads).
- Complete quince → skykeep rename (2,653 occurrences / 358 files; keep PREVIOUS_IDENTIFIERS; migrate
  localStorage keys and "quince:" stored ids).
- 0.2.0: docs/sync-roadmap.md steps 4–14 + shared Drive folders, Shared Photo Library, People, shared
  albums contents, shared reminders/notes, Notes folder export, security-key sign-in, headless backup + Docker.
- Store reviews pending: winget #470370 (Validation-Completed, moderator), Chocolatey (moderation; VirusTotal
  1–4 false positives "Note"), Microsoft Store certification. Snap forum keyring post: his, once activated.
- Reddit account banned (spam); appeal submitted; never make a new account.

---

# Next (2026-11-04, 12:25 IST) — stores

**main 4233947 pushed** (gate 140/140, workspace fmt+clippy clean). Since v0.1.3: lapse reason in
the toast, RateLimited wording, calendar-undo suite, resend/SMS errors via apple_error, session
rename waits for Defender, Snap-specific keyring message, actions on Node 24, Flatpak + Snap files.
Next release 0.1.4 = these + merge `packages` and `msix` branches (BOTH edit updates.rs
install_kind: combine — msix → Store, flatpak → Flatpak, scoop on windows → PackageManager; keep
every test).

Store status:
- **Snap**: snap.yml dispatched for v0.1.3 publish=true (run 41788122338). password-manager-service
  is deny-auto-connection in snapd → users must `snap connect` until the store grants auto-connect.
  Request text: ~/Documents/Skykeep/snap-keyring-request.md (HE posts on forum.snapcraft.io).
- **Flathub**: BLOCKED for AI help. flathub/flathub AGENTS.md + Generative AI policy: manifests must
  not contain AI-generated/assisted content even if disclosed; AI must not open/write the PR or
  replies. Our manifest is AI-written, so it can't be submitted as is. Branch add-skykeep was pushed
  to his fork (YashwanthKamireddi/flathub) before I read the policy; no PR opened. HIS call; he
  must write the manifest + PR himself and disclose AI-generated app code. Flatpak CI build + start
  test green (run 41785901664).
- **winget** PR #470370: CLA signed; awaiting install validation + moderator.
- **Chocolatey**: CHOCOLATEY_API_KEY secret set; publish after packages branch merges (moderation days).
- **Scoop**: create YashwanthKamireddi/scoop-bucket at 0.1.4 (0.1.3 can't detect Scoop).
- **COPR**: his Fedora login hit "System error" (new-account sync); retry with username, cleared cookies.
- **MSIX**: agent adding StartupTask; last run 41786702402 failed on the startup-task check. Needs his
  3 Partner Center identity values.
- README: add each channel only once it is live (literally true).

---

# Next (2026-11-04, 10:35 IST)

**v0.1.2 published 05:01 UTC** (tag d797b45 "Skykeep 0.1.2"). latest.json = 0.1.2 (AppImage + nsis + msi, signed);
all download links 200. Full local gate 137/137 before push; CI + release dry run green.
In it: SRP fix (pad A/B/S, lowercase account name in H(I)); locked keyring asked again; Apple 5xx/429
not a sign-out; p256 0.14; AppImage built on Ubuntu 22.04 (glibc 2.35, own libheif 1.19.8); Windows
drags inside windows (src/lib/drag.ts), rename retries for Defender (quince-core/files.rs); Photos copy
PNG+bitmap on Windows; Drive paste from file manager; Hidden album lock; Drive Keep Both/Replace;
Calendar month bars; Photos Date Added levels. README: Linux|Windows table, .deb/.rpm update wording,
copyright notice.
- AppImage catalog PR #11157 (opened by probonopd): commented /retest.
- Reddit user "ninjainNight": sign-in 403 with a lowercase Apple ID. If 0.1.2 still fails for them,
  get the full error; consider showing Apple's full reply (currently cut at 220 chars). Never auto-retry
  a rejected proof (account lockout risk).
- Needs his Windows PC: in-window drag (reminders/contacts/photos), Photos copy → Paint.
- Next: bump GitHub Actions off Node 20 (warnings only); winget submit on his go
  (~/Documents/Skykeep/winget, update to 0.1.2 hashes first); 3 agent worktrees still locked in .claude/worktrees.
- Posting: ~/Documents/Skykeep/GO.md; poster ~/Documents/Skykeep/images/poster.png.

---

# Next (2026-11-04, end of day)

- Posting sheet: ~/Documents/Skykeep/GO.md (only one). Step 5 = 19 Reddit posts, rules read from
  each sub; start Wed with r/linux + r/software; 2 posts/day max.
- Sent/asked tonight (him): HN mod email about the flagged Show HN; r/iCloud modmail (their
  "not vibecoded" rule; post only on a yes).
- HN: account 7 months, 1 karma. Don't repost. Earn ~31+ karma by commenting; later a link to a
  write-up in his own words.
- README: "six real desktop apps on a native Rust engine" first (7385d50); copyright notice for
  Yashwanth Kamireddi (2e4af23). He said NO to a "Use of AI" section and NO to publishing
  docs/icloud-web for now. Comparison page stays local (docs/comparison-draft.md).
- Film: ~/Documents/Skykeep/videos/film-real.mp4 (v3, 44 s, real app in 4K); source
  skykeep-launch src/films/LandscapeReal.tsx.
- Proof of authorship: Software Heritage save accepted 2026-11-03 18:46 UTC (running); Wayback
  save sent, not yet indexed: recheck.
- Next for me: README feature table (Linux | Windows ticks, every one verified); recheck Wayback;
  prove updater with 0.1.2; winget submit only on his go (manifests in ~/Documents/Skykeep/winget).

---

# Next (2026-11-04)

**Launched 2026-11-03:** v0.1.1 published (v0.1.0 tag burned, no release). Repo public; profile
public. main = 6af50a7 (README: "Not icloud.com in a window"), pushed. Windows tested by him on a
real PC.

**Launch state:** day one got almost no traction. HN Show HN flagged (new account; email sent to
hn@ycombinator.com). r/SideProject 200 views, 0 votes. X/Threads ~0 views. Press emailed:
OMG! Ubuntu, OMG! Linux, It's FOSS. Tauri Discord done.
Posting sheet = ~/Documents/Skykeep/GO.md (the only current one). Reddit step 5 in
progress (rules scraped from Reddit in headless Chromium: /tmp/rr/).

**Open (product):**
1. Prove the updater: cut 0.1.2 and watch a 0.1.1 AppImage + Windows install update.
2. Photos Copy "random characters" report: not confirmed; retest.
3. winget (manifest being prepared, not submitted), Flathub (not submitted), AUR (registration
   closed), SignPath code signing (not applied).
4. GitHub: Discussions off; no pinned "what works / next" or "install reports" issues; no
   good-first-issues filed.
5. Honest comparison page draft: docs/comparison-draft.md (being written, not committed).

**Later (after launch work):** a real demo film (real iPhone + real screen, real story) good
enough for big YouTubers. He uses only film-landscape-music.mp4 until then.

---

# Next (2026-10-30)

GitHub main = 8952141, pushed. EVERYTHING from the 28-29 Oct agent passes is merged and pushed:
Calendar pass, Drive live updates (all zones), outbox randomized test, Notes/Reminders, Photos/Drive,
Contacts/shell, Drive copy/paste, startup trim, updater without zip, Notes list pictures + long-list
walking + Calendar all-day <-> hours drag, Photos Years/Months/All Photos (+ WebKit jump fix),
Liquid Glass tokens + popover/segmented motion (blur only in Chromium; WebKitGTK frosted, no blur).

Gate on 8952141: rust 1085/0, clippy, fmt, tsc, node 71, browser 478 runs both engines: 475 ok, then
the 3 failures fixed (calendar-edits month-boundary test, photos-levels-scale WebKit jump = real fix,
photos-slideshow load) and re-run clean (86 photos/calendar runs). The Notes "tick race" was a test
timing artefact (shortcut 10 ms after End), not an app bug; sync-quiet's redraw count read stale fiber
halves (fixed, mutation-checked).

Worktrees left: skykeep-integrate (branch integrate = main), skykeep-linuxci (linux-ci-green).

## Next
1. Real-app check by him of the new build (launcher binary rebuilt from 8952141).
2. Open items from agents: Photos Date Added sort vs Years/Months (by date taken); Drive Ctrl+Alt+V
   onto a taken name (Finder asks Keep Both/Replace); Ctrl+D naming vs Paste naming; app-library
   copy/paste; clipboard paste of files on Windows/macOS; invitations accept/decline (no write path);
   Month-view bar stretching; WebKit Contacts/Drive per-step style cost (~4-6 ms, CSS).
3. Release pass (after repo is public): linux-ci-green CI, v0.1.0, tag, draft release.

---

# Next (2026-10-25, evening)

Local main = 457ae9d (polish-all fast-forwarded: design language, 44+ bug fixes, live-sync
smoothness, merge fixes). NOT pushed yet: 53 commits ahead of origin/main. Gate on the merge:
1054 Rust tests, fmt/clippy/tsc, 68 node tests, 418/428 browser runs then all fixed (54/54 on
affected suites), photos-paging/scroll + notes-follow both engines, a11y clean.

## Now: one day of real use (soak) on his library
Skykeep restarted 22:04 from this build. Library backup before it:
scratchpad/realbackup-polish/library.db. He uses it normally (photos from iPhone, notes typed on
the phone, events, Drive) and reports anything off.

## Then
1. If the soak is clean: push main; remove worktrees skykeep-{design,bugs-nrc,bugs-pdcs,sync,polish}
   and their branches.
2. Open questions: Apple all-day alert time (9 AM vs midnight) - check on his iPhone.
3. Release pass: linux-ci-green branch needs CI (only after the repo is public, since Actions
   minutes are used up), version 0.1.0 + metainfo date, tag, draft release, attach the films.
4. His part: back up ~/.tauri, make repo public, social preview, then rulesets + release.
