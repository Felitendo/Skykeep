# Skykeep launch playbook

Local working notes, written 2026-10-24. **Never commit `docs/`.** This replaces the launch
sections of `marketing-2026-10-21.md`, `launch-plan.md` and `product-review.md` §7. Those were
written before the rename, the updater and the Windows work, so several of their claims are no
longer true (see §0).

Research for this file was done live on 2026-10-24: Hacker News (Algolia API), GitHub,
creators' channels, and the outlets' own contact pages. **Reddit blocks automated reading, so
every subreddit rule below came from third-party scrapes.** Read each sidebar yourself before
posting. Anything not verified is marked *unverified*.

---

## Update 2026-11-03 (read first; overrides older lines below)

**State:** main 8952141, 579 commits, about 116k lines of Rust, 33k of TypeScript, 1,092 Rust
tests, 255 browser suites. Full history scanned on 2026-11-03: no secrets (gitleaks --all), one
author (the no-reply address), no AI or Claude trailers (the only two Co-authored-by lines are
Dependabot's own), no org strings, no account ids, no real phone numbers or addresses (only
placeholders), no file over 3 MB. Safe to make public.

**New, literally true claims (since 24 Oct):**
- iCloud Drive changes made elsewhere show within seconds, folder by folder, including the Pages,
  Numbers and Shortcuts folders (CloudKit change feed; measured 0.4 s for the check).
- Copy a photo straight to the clipboard (as PNG, never HEIC).
- Photos: Years, Months and All Photos, as on a Mac; smooth on 50,000 photos (test suite).
- Drive: Copy and Paste between folders, as Finder; paste files copied in the Linux file manager.
- Calendar: Ctrl+Z undo; drag between the all-day row and the hours; all-day alerts at 9 AM.
- Notes: search marks the words found; picture thumbnails in the list.
- Reminders: the Completed list; drag a reminder to another list; folding subtasks.
- Glass on menus, popovers and sheets (blurred on Windows; frosted, unblurred on Linux).

**Competition moved (check again on launch day):** two new Rust clients since late Sept, both
single-purpose: taylorhayis/icloud-drive-for-linux (Drive only, created 27 Oct, 4 stars, says
iCloud does not push changes, so edits show on the next check) and ferdousbhai/icloud-for-omarchy
(Notes, Photos, Find My on Arch, 5 stars). cross-platform/icloud-for-linux (577 stars) is a web
wrapper, last pushed Oct 2025. "Six apps in one, live sync, offline" is still unmatched. Never
attack them; credit them if asked.

**Channel rules changed:**
- **Hacker News:** since March 2026 new accounts can't post Show HN (/showlim), and the
  guidelines forbid posting or editing any text with an LLM: the post and every comment must be
  written by Yashwanth himself. Building with AI is fine. So the HN copy in §2.3 is a list of
  facts to use, not text to paste. If his account is new, post the dev-story blog post as an
  ordinary link (it was the stronger route anyway, §2.0).
- **r/opensource:** since 24 Oct 2026 bans AI-built projects and requires a year of maintenance
  and several contributors. Skip it.
- **r/selfhosted:** projects under three months old go in the weekly New Project Megathread only.
- **Product Hunt:** the homepage is now chosen by editors (Useful, Novel, High craft, Creative);
  only Featured launches get traffic, and votes from established accounts count far more. Put a
  direct download link and the SmartScreen note in the maker comment (installs, not upvotes).
- **YouTube:** don't pitch Brodie Robertson (May 2026 controversy). Look at TechHut, The Linux
  Experiment, DistroTube, Linux Lounge via their About-page business emails.
- **Press:** OMG! Ubuntu takes developer pitches at contact@omgubuntu.co.uk (tip form for
  others); OMG! Linux at tips@omglinux.com; It's FOSS through itsfoss.com/contact-us/.

**AI question:** expect "was this vibe-coded?" on HN and Reddit. Never deny it. Have one honest
line ready in his own words: what he designed and decided, that AI tools helped write code, and
what proves it works (1,092 Rust tests, 255 browser suites, a full-history CI gate run locally).

**Recommended order:** make the repo public quietly now (history is clean, Actions minutes become
free, the release pipeline and Flathub need it, and a public timestamp establishes the work);
then the launch gates below (v0.1.0 built by CI and install-tested, updater proven with a test
v0.1.1, a real Windows PC run, films ready); then Day 1 on a Tuesday (17 Nov 2026 is realistic).

## 0. The claim sheet

Every line of copy in this playbook comes from the "true" column. If a sentence isn't backed by
this table, it doesn't ship.

### Literally true (2026-10-24, main at `bf7ac8c`)

| Claim | Evidence |
|---|---|
| Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos, as desktop apps on Linux and Windows 10/11 | README, Features table |
| Six apps, one sign-in. Each app has its own launcher entry on Linux. On Windows each has its own Start-menu entry and taskbar button, plus a jump list (New Note, New Reminder, New Event, the six apps) | `99a400a`, `c6d9d62`, `taskbar.rs`, `apps.rs` |
| Notes and Reminders changes from the iPhone usually arrive in **about a second**, over Apple's push channel | README FAQ, `push.rs` |
| Calendar and Contacts are checked **every few seconds** while Skykeep is in use | README FAQ |
| The library lives on your computer: it opens instantly and works offline. Notes, reminders, events and contacts can be edited offline and sync on reconnect | README Privacy/FAQ, outbox |
| A note edited on both sides at once is **merged**, not overwritten | README Features ("Edits from your phone merge with yours") |
| On Linux, iCloud Drive (and Photos) is also a real folder in the file manager (FUSE) | README, SECURITY.md |
| Quick capture, results in GNOME and KDE search, a tray icon, alerts with Complete and Snooze | README "Everywhere" |
| **Export Everything** to Markdown, iCalendar, vCard and your original files | README |
| It connects to **Apple** for your iCloud, **the calendars you subscribe to**, and **GitHub's release servers** for its own updates. Each has a connection locked to its own hosts in code (`apple_only()`, `listed_only()`, `releases_only()`) | SECURITY.md |
| `.deb`, `.rpm`, AUR and Flatpak copies never contact GitHub; their package manager updates them | SECURITY.md |
| **No Skykeep account, no Skykeep servers, no analytics, no telemetry, no crash reporting** | SECURITY.md |
| Updates are signed and checked (minisign, pinned key), never a downgrade, and never installed mid-sync or while you type | SECURITY.md |
| Your password is never sent, not even to Apple: sign-in is Apple's SRP. Two-factor sign-in is Apple's own | README FAQ, SECURITY.md |
| Your password and session are kept **in the system keyring** so Skykeep signs back in by itself. Cookies are encrypted with a key held in the keyring | SECURITY.md (say this plainly; never imply the password isn't stored) |
| Advanced Data Protection works with **Access iCloud Data on the Web** turned on | README FAQ |
| Free and open source, GPL-3.0. Rust engine plus Tauri 2 | LICENSE, README |
| Installs per user on Windows, no administrator password | README Install |
| Engineering scale: **457 commits; about 111k lines of Rust in 5 crates plus the app; about 30k lines of TypeScript; about 1,000 Rust test functions** | `git rev-list --count HEAD`, line counts, `#[test]` count on 2026-10-24. **Re-run on launch day.** |

### Retired claims: never use these again

- "Talks only to Apple" / "It can only talk to Apple" / "Apple-only, enforced in code". Now it is
  Apple, your calendar subscriptions, and GitHub for updates.
- "No update check". There is now a daily update check in the AppImage and Windows copies, and
  it can be turned off.
- "Beta" / "preview" for Windows or Photos. The labels were removed. Still, don't claim
  "stable" or "production-ready" either: let v0.1.0 speak for itself.
- "Native UI". It is a Rust engine with Tauri webviews. Say **"desktop apps"**, **"a native
  Rust engine"** or **"real apps, not a website in a window"**. HN *will* ask "is Tauri native?"
- "First native iCloud client on Linux". ferdousbhai/icloud-for-omarchy (native Notes, Photos
  and Find My on Arch) came first. Say **"Notes, Reminders, Calendar, Contacts, Drive and Photos
  in one app"**, which is true and unmatched.
- Any regional-account statement. Don't mention it in copy at all.
- Translations or other languages. There are none, and none are planned for launch.
- Anything implying Apple approval, partnership or endorsement. No Apple logo, no fruit, no
  Apple product renders, no "official".

### Stale lines to fix in other docs before launch (I didn't edit them)

- `github-setup.md` §2: the About description still says "(beta)" and "Talks only to Apple".
  Use the one in §5.2 below.
- `github-setup.md` §6–7: the release notes and pinned issue still say "Windows is beta,
  Photos is a preview". Use §2.10 and §5.4 below.
- The README hero `alt` text says "a note that has just arrived from an iPhone". The hero is
  rendered from the demo fixture, not filmed from a phone. This is acceptable as alt text for
  an illustration. But the **launch film must be a real phone and a real desktop, filmed in
  real time** (§4.1). That's the one asset where staging would be a lie.

### Launch gates (all true before Day 1)

1. Repo public; privacy audit, history purge and secrets scan done (`github-setup.md` §9).
2. **v0.1.0 published** with the `.exe`, `.msi`, `.deb`, `.rpm`, `.AppImage`, SHA256SUMS,
   `latest.json` and `.sig` files. Each package install-tested once on a clean system
   (`OPEN-ITEMS` #4 is still open).
3. **The updater is proven end to end:** a v0.1.0 AppImage and a Windows install both pick up a
   test v0.1.1 and update. Only you can cut that fix release if launch day breaks something,
   so prove the path works before you need it.
4. **Windows run on a real Windows PC**, yours or a friend's: install, sign in, 2FA, edit a note,
   Start-menu entries, jump list, update. CI smoke-tests on GitHub's runners; a real machine is
   still the bar before pitching Windows creators.
5. AUR `skykeep-git` live. Flathub submitted (it's a separate launch moment when accepted;
   the manifest has never been built and the metainfo lacks `<screenshots>`, per `OPEN-ITEMS` #6).
6. The real-phone film recorded (§4.1).
7. `~/.tauri` updater key backed up offline. Losing it means no updates, ever.
8. You've blocked out Day 1 from 12:00 to 04:00 IST, and Days 2–3 evenings.

---

## 1. The narrative

### 1.1 One-sentence pitch

> **Skykeep is a free, open-source app that puts your iCloud Notes, Reminders, Calendar,
> Contacts, Drive and Photos on Linux and Windows as fast desktop apps, kept on your computer
> and in sync with your iPhone.**

The ten-word version is for titles and bios: *Your iPhone's iCloud, as real apps on Linux and
Windows.*

### 1.2 The villain is the wall, not a company

The villain is **the wall between your iPhone and your computer**. On Linux and Windows,
iCloud lives in a browser tab. It's online-only, has no alerts, isn't in your file manager, and
gets lost among twenty other tabs. Apple builds superb apps for its own devices; Skykeep exists
because many people love their iPhone *and* choose Linux or Windows.

Rules for the villain:

- **Never attack Apple**, Apple's Windows app or icloud.com. Describe what's missing, never
  what's bad. "iCloud for Windows has no Notes or Reminders app" is a fact. "iCloud for Windows
  is terrible" is an attack, and it invites a pile-on that drowns the launch.
- **Never attack other projects** (web wrappers, icloud-for-omarchy, rclone, icloudpd). Credit
  them when asked; they proved the demand.
- The hero is **the user**, who no longer has to choose. Skykeep is the bridge.

### 1.3 The story arc (Apple-keynote shape, honest)

Use this shape for the film, the blog post, the PH maker comment and every talk. It's the
keynote shape: one message, problem before product, demo before features, rule of three, and a
held-back reveal.

1. **The world as it is.** "My notes live on my iPhone. My work lives on my Linux laptop."
2. **The wall.** "On the desktop, my notes lived in a browser tab: online-only, no alerts, gone
   when the Wi-Fi drops."
3. **The turn.** "So I built the apps Apple makes for the Mac, for my computer."
4. **The magic moment (demo, no words).** Type on the iPhone; a second later it's on the
   desktop.
5. **Three proofs:** *Instant* (it's on your disk), *Offline* (keep working, it syncs later),
   *Yours* (open source, no account, no analytics, export everything).
6. **"And the rest of your iCloud."** Calendar, Contacts, Drive as a real folder, Photos.
7. **One more thing** (only if true on launch day): "And it's on Windows too, with each app in
   your Start menu." For a Linux audience, the Windows reveal is the surprise. For a Windows
   audience, flip it: the reveal is "and Notes and Reminders, which iCloud for Windows doesn't
   have."
8. **The close.** "Free. Open source. Yours."

### 1.4 Emotional beats to film and write toward (all real)

- The note typed on the phone appearing on the desktop. **The core moment: hold on it.**
- A reminder ticked on the laptop, then the phone checks it off.
- Wi-Fi off on a train, still editing; Wi-Fi on, it syncs.
- A Linux file manager showing an "iCloud Drive" folder, then dragging a photo out of it.
- Searching "dentist" from the GNOME/KDE launcher and the reminder appearing.
- The Windows Start menu: typing "Reminders" brings up Skykeep Reminders.

### 1.5 Three taglines, ranked

1. **"Your iCloud, at home on Linux and Windows."** Already the README line. Warm, says who and
   where, works for every audience. Use it on GitHub, PH, the press kit and the social preview.
2. **"Write it on your iPhone. Read it on your PC a second later."** The magic moment as a
   sentence. Best for the film end card and short-form video. True for Notes and Reminders
   only, so only pair it with them.
3. **"iCloud, out of the browser tab."** The villain line. Best for Linux and developer
   audiences and for headlines. Don't use it where it reads as a dig at icloud.com: keep it next
   to the positive claim ("…and onto your desktop: instant and offline").

Also usable: *Instant. Offline. Yours. Open source.* (the rhythm line), and *Your cloud, kept
safe on your own computer* (the name's meaning; good for the About section and press).

### 1.6 Messaging per audience

| Audience | Identity line | Lead claim | Proof to show | Avoid |
|---|---|---|---|---|
| **Linux users with iPhones** (core) | "For people who love their iPhone and their Linux desktop." | Notes and Reminders on Linux as real apps, live in about a second | The split-screen clip; Drive as a folder in Nautilus/Dolphin; GNOME/KDE search; AUR/deb/rpm/AppImage | "Apple on Linux" (they chose Linux on purpose); "native UI" |
| **Windows users** | "Your iPhone and your PC, finally on the same page." | The Notes and Reminders apps iCloud for Windows doesn't include; each app in your Start menu | Start menu → "Reminders" → Skykeep Reminders; the jump list; tick on PC, ticks on phone | Disparaging Apple's Windows app; hiding the SmartScreen prompt (say it up front) |
| **iPhone + PC families** | "Everyone keeps their iPhone. The family PC finally keeps up." | Shared lists and the family calendar on the computer, no new account | Reminders/Calendar on a Windows laptop; one sign-in | Jargon (CRDT, Rust, FUSE); anything about passwords that isn't in SECURITY.md |
| **Privacy-minded people** | "Your data on your disk, in software you can read." | No Skykeep account, no servers, no analytics; every connection listed and locked in code | SECURITY.md's host list; `apple_only.rs`, `releases.rs`; updates off with a switch; Export Everything | "Talks only to Apple" (retired); implying the keyring is unreadable by your own programs (SECURITY.md says it is readable) |
| **Developers** | "A hard problem, solved in the open." | One Rust engine, six apps; reverse-engineered CloudKit for Notes (gzip in `ENCRYPTED_BYTES`, protobuf, a topotext CRDT); push sync; locked connections | The blog post; crate map; ~1,000 tests; `./check.sh`; good-first-issues | Leading with "built fast" or tooling stories (§6.6) |

---

## 2. Launch sequence and copy

### 2.0 Strategy in five lines (why this order)

1. **Hacker News is cold to this category's Show HN posts.** In 2026, every iCloud-on-Linux Show
   HN got about 1 point, and Tether (iMessage on Linux) got about 10 as a Show HN. **But
   Tether's blog post, posted as a plain link, reached 549 points**, and "iCloud Photos
   Downloader" reached 653 in Jan 2026. HN wants the *story and the engineering*, not a repo
   link. So the dev-story post is a first-class launch asset, not an afterthought.
2. **Linux press is the most reliable engine.** It's FOSS covered Tether within a week, and
   OMG! Ubuntu covers polished new Linux apps. Pitch under embargo so Day 1 has articles.
3. **Concentrate Day 1** (repo, r/linux, Show HN, socials, press) so GitHub Trending sees one
   spike of star velocity, not a trickle.
4. **Sunday is the iPhone day.** r/apple allows developer self-promotion only on Sundays
   (Pacific time), and weekend HN posts break out more often than weekday ones (Myriade, 157k
   Show HN posts: Sunday 11.75% vs weekdays ~9.5%).
5. **Windows gets its own wave on Days 9–10**, after a real-PC check and the winget listing.
   It's a second launch moment, not a footnote.

**Day 1 is a Tuesday.** Times are in UTC with conversions. *After 1 Nov 2026 (US standard
time): 13:00 UTC = 08:00 ET = 05:00 PT = 14:00 CET = 18:30 IST.* Before 1 Nov, ET and PT are an
hour later on the clock (09:00 ET). Europe leaves summer time on 25 Oct 2026.

### The bigger plan (2026-11-03): no Reddit karma needed, every kind of user

Ready-to-send messages for subreddit mods, Discord servers, forums and Lemmy: docs/outreach-messages.md.

What actually made comparable launches break out:
- **Beeper Mini** (iMessage on Android, Dec 2023): a launch-day *exclusive* with one big tech outlet
  (TechCrunch), a consumer pain everyone knows ("blue bubbles"), a reverse-engineering story.
  100,000 downloads in 48 hours. Apple cut it off within three days, because it impersonated an
  Apple device. Lesson for us: mainstream tech press reaches far beyond Linux, and the framing
  must stay "uses iCloud.com's own web service, the same one your browser uses", never "hacked".
- **Delta** (emulator, App Store #1): short videos by ordinary users and news accounts (IGN's
  TikTok: 164k likes) in a surprised tone: "wait, you can do this on your phone?".
- **Wealthfolio** (solo, open-source, local-data desktop app): Product Hunt, then the HN front page,
  then GitHub Trending, then a package manager listing (Homebrew), then Google search as the
  steady, lasting source.
- **Tether** (iPhone Continuity on Linux): It's FOSS covered it; that single article was the spread.

Channels by audience, none needing Reddit karma:
1. **Mainstream tech press, one exclusive** (the Beeper move): The Verge, 9to5Mac, MacRumors,
   AppleInsider, Ars Technica, XDA, How-To Geek, Windows Central, Neowin. Offer one of them the
   story 48 hours early under embargo: "a student built the iCloud apps Apple doesn't make for
   Windows and Linux, free and open source". iPhone + Windows is a far bigger audience than Linux.
2. **Linux press** (embargo, same day): OMG! Ubuntu, OMG! Linux, It's FOSS, Linuxiac, GamingOnLinux
   (no), Phoronix (no: kernel/hardware), LWN (only for the engineering post).
3. **Short video, made by him** (the Delta move): 15-30 s vertical clips, phone in hand, real
   screen: "I type on my iPhone... and it's on my Linux laptop." One idea per clip: Notes live;
   iCloud Drive in the file manager; Windows Start menu "Reminders"; offline on a train; copy a
   photo straight from Photos. Post the same clip to YouTube Shorts, TikTok, Instagram Reels, X,
   Threads, Bluesky, LinkedIn. The films in skykeep-launch/out/ are the long versions.
4. **His own accounts, as the builder**: X, Bluesky, Mastodon (Fosstodon), Threads, LinkedIn (the
   student-builder angle also feeds the Masters and placement story). A short thread: problem,
   the 10-second clip, three proofs, link. Written by him.
5. **Federated and forum communities** (no karma): Lemmy (!linux@lemmy.ml, !opensource, a
   !skykeep community of its own), Discourse forums (Ubuntu, Fedora, KDE, GNOME, CachyOS,
   EndeavourOS, Manjaro) in their app-announcement categories, distro Discord servers.
6. **Discovery that compounds** (the Wealthfolio long tail): Flathub (the biggest Linux app store),
   AUR, winget, AlternativeTo (its "iCloud for Linux" page is empty today: list Skykeep there and
   as an alternative to iCloud on Windows), Google (a docs page answering "how to use iCloud
   Notes on Linux / Windows"), honest Stack Exchange and forum answers to the years-old "iCloud on
   Linux?" questions, with disclosure.
7. **Newsletters and podcasts**: This Week in Rust (the engineering post), Console.dev, Changelog
   News, It's FOSS's FOSS Weekly, OMG! Ubuntu's monthly app roundup; Linux Unplugged, Late Night
   Linux, Destination Linux, Linux Matters (pitch as a guest: the story is the hook).
8. **YouTube creators** (they bring the non-technical users): TechHut, The Linux Experiment,
   DistroTube, Linux Lounge, Chris Titus (Windows + Linux audience); Windows-side creators for the
   Windows wave. Email a 30 s clip and a one-line ask; never pay, never script them.
9. **Reddit, without his own karma**: articles from steps 1-2 get posted to Reddit by readers;
   meanwhile he can build karma by genuinely helping in r/linux, r/iphone, r/Windows11 threads
   asking about iCloud for two weeks, and post r/apple on a Sunday later.
10. **Hacker News**: the engineering story as a plain link, written and answered by him.

The order: quiet public repo -> v0.1.0 + gates -> embargoed pitches (Day -5) -> Day 1: exclusive
article and Linux press go live, his clip and thread everywhere, Lemmy and forums, HN link ->
Days 2-14: one new clip a day, creators, Flathub, newsletters, Windows wave -> long tail: listings
and search. No single post decides it.

### What the biggest open-source breakouts did, across every field (2026-11-03)

| Project | How it broke out | What we take |
|---|---|---|
| **OpenClaw** (fastest-growing repo ever, 9k to 195k stars in 66 days) | Other people's demo videos and posts ("it bought a car by itself") spread across social media; stars rose by thousands an hour. Lost its @handle to snipers during a forced rename (Anthropic trademark). | One astonishing, *shareable* moment, shown by others. Secure every handle before going public; no third-party marks in the name (Skykeep is clean). |
| **Supabase** (100k stars) | Growth started the day the tagline became "the open-source Firebase alternative". Launch Weeks: one feature a day for a week, every few months; each one spikes stars. Founder messaged 3,000 developers one by one. | Tagline in the "open-source X alternative" form. **Six apps = a natural Launch Week: one app a day.** Talk to users one by one. |
| **Omarchy** (DHH; 46k stars, 1.3M ISO downloads in year one) | A famous builder's essays, HN (776 points), ThePrimeagen's show; users sharing their setups, themes and plugins. A second, bigger wave at version 4. | The Omarchy crowd left macOS and many keep iPhones: an iCloud-for-Omarchy project already exists (5 stars). Make Skykeep look at home on Hyprland/Omarchy and pitch that community and DHH. Plan a second wave (v0.2/1.0). |
| **Ghostty** | Years of devlogs on the builder's blog, a private beta in Discord (28k members waiting), a pre-announcement two months ahead, first public release already called 1.0, podcasts. | Tell the build story in public before launch (blog, X, Mastodon). A Discord or Matrix community from Day 1. Ship as finished, set expectations openly. |
| **Immich** (87k stars) | Mainstream reviewers wrote first-person "I replaced Google Photos" pieces (XDA, Android Police, MakeUseOf); r/selfhosted and self-hosting YouTubers. | Pitch the *first-person* angle to XDA, MakeUseOf, How-To Geek, Android Police writers: "I finally use iCloud Notes on my Linux PC". |
| **Zen Browser** (500k users) | Word of mouth among power users; its own subreddit; a community vote crowning it; mods and themes by users. | Our own community space early; let users share setups. |
| **PostHog** | Ready-before-HN (strangers succeeding unaided), founder blog posts on everything behind the scenes, an honest "best tools" comparison page (its most-read post ever). | Write the honest "every way to use iCloud on Linux and Windows" page (crediting the others) for search. |
| **Beeper Mini** | Launch-day exclusive in TechCrunch; 100k downloads in 48 h; Apple blocked it in 3 days. | One exclusive with a big tech outlet; say "uses iCloud.com's own web service", never "hacked". |
| **Wealthfolio, LocalSend** | HN front page, then GitHub Trending, then package managers and search; LocalSend hit HN three times over three years. | Package listings are lasting traffic; every big release is another HN moment. |

**The ten moves, in order of leverage for Skykeep**
1. Secure the name everywhere first: X, Bluesky, Mastodon, Threads, Instagram, TikTok, YouTube,
   a Discord server, and a domain. (OpenClaw lost its handle.)
2. Tagline in the proven form: "The open-source iCloud apps for Linux and Windows."
3. One 10-second clip that makes people say "wait, what": phone types, laptop shows it.
4. A Launch Week: Day 1 the suite; then one app a day (Notes, Reminders, Calendar, Contacts,
   Drive, Photos), each with its own clip, post and story. Six launch moments, not one.
5. One mainstream exclusive (The Verge / 9to5Mac / XDA), Linux press the same day.
6. The engineering story on his blog and HN; This Week in Rust.
7. Communities that care most: Omarchy and Hyprland users, Arch/CachyOS, iPhone+Windows people.
8. First-person reviewers and YouTubers (they reach non-technical users).
9. Lasting discovery: Flathub, AUR, winget, AlternativeTo (empty page today), the comparison page.
10. A second wave at the next big release (1.0), as Omarchy, LocalSend and Ghostty did.

### 2.1 Before Day 1

| When | What |
|---|---|
| **T-21** | Gates 2–5 started. Record the real-phone film (§4.1). Submit to Flathub. File 12–15 good-first-issues (§5.5). Write the dev-story blog post (§2.9) **in your own words**. |
| **T-14** | Press kit ready (§4). Cut the 16:9, 9:16 and 1:1 versions. Final README pass (§5.1). |
| **T-7 (Tue)** | Send personal pitches to the Tier 1 creators and press (§3) with: "Public from <T-2>; free to cover from Day 1, 13:00 UTC; happy to answer anything before." **One email each, personalised.** |
| **T-5** | Dry run: two friends install on Ubuntu/Fedora/Windows from the draft-release files and sign in. Fix what they hit. |
| **T-3** | v0.1.0 tagged; draft release checked; `latest.json` and signatures verified; test-update to a v0.1.0-hotfix rehearsal tag on a throwaway fork **or** a local updater test. |
| **T-2 (Sun)** | **Repo goes public quietly.** Publish v0.1.0. Upload the social preview. Pin the two issues. Discussions categories set. No announcement anywhere: press can now try it, and if someone finds it early, the README is already the launch page. |
| **T-1 (Mon)** | Freeze. Every post saved as a draft (HN text written by you). Sleep early. Check the AUR package builds from the public repo. |

### 2.2 Day 1 (Tuesday), hour by hour

| UTC | ET / CET / IST | What |
|---|---|---|
| 07:00 | 02:00 / 08:00 / 12:30 | Wake. Check overnight issues and Discussions. Smoke-test the release downloads once more on your own machine. |
| 11:30 | 06:30 / 12:30 / 17:00 | Final checks: README renders, release assets download, badges live. Phone and laptop ready for live GIF replies. |
| **12:00** | **07:00 / 13:00 / 17:30** | **Show HN** (§2.3). Weekday HN breakouts cluster at 11–13 UTC. Post it and **don't tell anyone to vote**. |
| 12:05 | | Post your first comment on the HN thread: how and why you built it, plus the honest limits (§2.3). |
| **13:00** | **08:00 / 14:00 / 18:30** | **r/linux** (§2.4). Press embargo lifts. Mastodon thread (§2.6) from a mastodon.social or hachyderm account; Bluesky thread; X thread. |
| 13:30 | | LinkedIn post (§2.6). Pin Discussions → Announcements "Skykeep 0.1.0 is out". |
| 13:00–17:00 | US morning, EU afternoon | **Answer every comment within 10 minutes**, HN first, then Reddit, then GitHub issues. Technical, short, humble, and the same facts everywhere. |
| 15:00 | 10:00 / 16:00 / 20:30 | Quick scan of issues by label: anything tagged `sign-in` or `data-loss` jumps the queue (§6.3). Reply to press emails. |
| 17:00 | 12:00 / 18:00 / 22:30 | Thank the outlets that published, and share their articles from your own accounts. (Never post links to your own HN thread anywhere: that reads as vote-seeking.) |
| 19:00 | 14:00 / 20:00 / 00:30 | **Hotfix window.** If a release-blocking bug is confirmed, ship v0.1.1 via the release pipeline (§6.3). Otherwise keep answering. |
| 22:00 | 17:00 / 23:00 / 03:30 | Snapshot numbers (§7). Write three lines on what people asked most; they become the FAQ and Day 2's fixes. |
| 23:00 | | Stop. The thread will keep going; answer the rest in the morning. |

### 2.3 Hacker News

**Read this first.** Since 2026-03-28, HN's own Show HN guidance asks that the post text **not
be written or polished by an LLM**. HN readers spot it instantly, and it would sink the launch
and your account's standing. So this section gives you the **title, structure and facts only**.
Write the prose yourself, in your own voice, the way you'd explain it to a friend. Typos are
fine. Marketing tone is not.

**Title options (pick one; HN titles: no caps-lock, no exclamation marks, max 80 chars):**
- `Show HN: Skykeep – Apple Notes, Reminders and iCloud Drive on Linux and Windows`
- `Show HN: Skykeep – iCloud as real desktop apps for Linux and Windows (open source)`
- `Show HN: I built the iCloud apps I wanted for my Linux desktop`

The URL field is the GitHub repo (people can try it right away, which Show HN requires).

**Text, in this order (bullets for you to write from):**
1. Who you are, one line. The itch: iPhone plus Linux; notes stuck in a browser tab.
2. What it is: six apps on one Rust engine (Tauri for the windows), local SQLite library,
   opens instantly, works offline.
3. The magic: Notes and Reminders come over Apple's push channel, usually in about a second;
   simultaneous edits to a note are merged.
4. One or two engineering details: `ENCRYPTED_BYTES` that turned out to be gzip, then protobuf,
   then a topotext CRDT; tables as a CRDT within a CRDT. Link the blog post.
5. Trust, plainly: SRP sign-in (password never sent), password and session kept in the system
   keyring so it can sign back in; what it connects to (Apple, subscribed calendars, GitHub for
   updates) and that each connection is locked to its hosts in code; no account, no analytics.
   Link SECURITY.md.
6. Limits, plainly: ADP needs "Access iCloud Data on the Web"; Managed Apple Accounts aren't
   supported yet; on Windows, Drive and Photos open inside the app (the folder is Linux-only);
   the Windows installer shows a SmartScreen prompt; not affiliated with Apple.
7. Ask for what you want to learn: "I'd love bug reports on sync edge cases."

**If the Show HN stalls** (under about 5 points after 2 hours): don't delete it, don't repost
it, and don't ask anyone for votes. HN moderators run a "second-chance pool": you can email
**hn@ycombinator.com** once, politely, asking whether it might be considered. Put an email in
your HN profile so they can send a repost invite.

**Day 6 (Sunday), 13:00 UTC:** submit the dev-story post (§2.9) as a **plain link**, not a Show
HN. It's different content, and it's the genre that worked for Tether. Use the article's real
title. Don't post it if the Show HN is still on the front page.

### 2.4 Reddit

**Universal rules:**
- One subreddit per day per audience, and **different text each time**. The same post in
  five subs on one day trips Reddit's spam filter.
- Post from your own account, aged with real participation. Keep self-promotion to about 10% of
  your activity: mods check your profile.
- Reply to every comment. Never ask for upvotes. Never use a second account to comment.
- Read the sidebar and pinned rules yourself first: **all rules below are unverified scrapes.**
- Lead with the video clip uploaded natively (Reddit autoplays native video; links don't).

#### r/linux (Day 1, 13:00 UTC)
Rules (unverified): news-style titles, no blatant adverts. Open source is fine.

> **Title:** Skykeep 0.1.0: Apple Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos as desktop apps on Linux (GPL-3.0)
>
> I use an iPhone and a Linux laptop, and the only way to get to my notes on the desktop was a browser tab. So I built Skykeep.
>
> It keeps a copy of your iCloud on your computer, so everything opens instantly and works offline. Notes and Reminders arrive over Apple's push channel, usually in about a second. iCloud Drive (and Photos) also show up as real folders in your file manager, and Skykeep shows up in GNOME and KDE search.
>
> [the 16:9 clip]
>
> - Packages: .deb (Ubuntu 24.04+, Mint 22, Debian 13), .rpm (Fedora 41+, openSUSE Tumbleweed), AppImage, AUR (`skykeep-git`).
> - Privacy: no Skykeep account, no servers, no analytics. It connects to Apple, the calendars you subscribe to, and (AppImage only) GitHub for updates, and each connection is locked to its hosts in code. SECURITY.md lists every one.
> - Limits: Advanced Data Protection needs "Access iCloud Data on the Web" on; Managed Apple Accounts aren't supported yet. It's an independent project, not affiliated with Apple.
>
> Repo: <link>. Happy to answer anything about how it works, including how Apple Notes stores text.

#### r/software (Day 2, Wednesday)
Rules (unverified): own software only if open source, **Wednesdays only, with the right flair**.

> **Title:** [Open source] Skykeep: your iCloud Notes, Reminders, Calendar and Drive as desktop apps on Windows and Linux
>
> Skykeep puts your iCloud on your computer as six small apps (Notes, Reminders, Calendar, Contacts, iCloud Drive, Photos) with one sign-in. On Windows, each one has its own Start-menu entry. Your library is kept on your computer, so it opens instantly and works offline, and changes from your iPhone show up in seconds.
>
> Free, GPL-3.0, no account, no analytics. Windows 10/11 and Linux. [clip] Repo and downloads: <link>

#### r/coolgithubprojects (Day 2) and r/SideProject (Day 4)
r/coolgithubprojects is link posts to GitHub only. The title is the post:
> **Skykeep – iCloud Notes, Reminders, Calendar, Contacts, Drive and Photos as desktop apps for Linux and Windows (Rust + Tauri)**

r/SideProject wants the maker story. Two paragraphs: why (iPhone plus Linux), what was hardest
(reverse-engineering how Notes stores text; making edits merge with the phone), and what you'd
like feedback on. Clip, then link.

#### r/opensource (Day 3, Thursday)
Rules (unverified): **Promotional flair required**; AI-written posts reportedly treated as
ban-worthy. **Write this one yourself.** Facts to cover: GPL-3.0, why GPL, contribution
on-ramps (good-first-issues, packaging help wanted: Nix, COPR, Flathub), how to run `./check.sh`.

#### r/apple (Day 6, Sunday, Pacific time)
Rules (unverified): developer self-promotion **Sundays only (California time), self-posts
only**, with activity requirements; must relate directly to Apple. Read the detailed rules page
first.

> **Title:** For iPhone owners whose computer isn't a Mac: I made an open-source app that brings Notes, Reminders and the rest of iCloud to Windows and Linux
>
> A lot of us have an iPhone and a Windows or Linux PC. On the PC, Notes and Reminders have meant icloud.com in a browser.
>
> Skykeep is a free, open-source app with Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos. Your library is kept on the computer so it opens instantly and works offline, and a note you type on your iPhone usually appears in about a second. Sign-in is Apple's own, with your usual two-factor code.
>
> [9:16 or 16:9 clip of phone and PC side by side]
>
> It's an independent project, not affiliated with Apple. It works with Advanced Data Protection when "Access iCloud Data on the Web" is on. Free, no account of its own, no analytics. Link: <repo>
>
> I'm the developer, so ask me anything.

#### r/rust (Day 8, Tuesday)
Rules (unverified): self-promotion must clear a bar such as an OSI/FSF license **or** a blog
post about using Rust. Lead with engineering, not features.
> **Title:** Reverse-engineering Apple Notes' CloudKit format in Rust: gzip in a field called ENCRYPTED_BYTES, protobuf, and a CRDT
>
> Body: a 6–8 line summary of the blog post, the crate layout (`quince-auth`, `quince-icloud`, `quince-core`, `quince-vfs`, `quince-media`), what Tauri 2 on WebKitGTK taught you (video through a loopback server; `WEBKIT_GST_USE_PLAYBIN3`), and the link to the post. Repo at the end.

(The crates keep their `quince-` prefix internally; if anyone asks, say it was the codename.)

#### r/Fedora or r/Ubuntu or r/linuxmint (Day 10, one sub only)
Install showcase: "Skykeep on Fedora 41: the .rpm, HEIC via RPM Fusion, and iCloud Drive in
Files". Screenshots on that distro's default desktop. Rules are unverified: modmail first if in
doubt.

#### r/windows / r/Windows11 (Day 9, only after modmail says yes)
> **Title:** Apple Notes and Reminders on Windows: an open-source app with a Start-menu entry for each iCloud app
>
> Body: the Start-menu clip; that it installs per user with no admin password; the SmartScreen
> prompt stated up front ("the installer isn't code-signed yet, so Windows asks you to confirm:
> More info → Run anyway; the checksums are on the release page"); winget command once live.

#### r/selfhosted (optional, Friday megathread only)
Projects younger than 3 months may only appear in the **New Project** megathread, and the fit is
weak (Skykeep isn't a server). If you post, one comment in the megathread: "Not self-hosted, but
local-first: a full offline copy of your iCloud on your own disk, exportable to Markdown/ICS/vCard."

#### Skip
r/productivity (self-promotion and links banned), r/privacy (app promotion on hold; modmail
only if you have a technical post they've approved), r/linux_gaming (off-topic), r/kde and
r/gnome (it's not a KDE or GTK app; only "feedback on integration" posts after modmail),
r/archlinux (the AUR page is the announcement).

### 2.4a Reddit from a new account (14 karma today)

Many subreddits silently remove posts from accounts below a hidden karma or account-age bar
(AutoModerator): the poster still sees the post, nobody else does. Plan around it:

1. **Ask the moderators first (T-7 to T-3).** "Message the mods" on each target subreddit:
   "Hi! I'm the developer of Skykeep, a free, open-source (GPL) app for iCloud Notes, Reminders,
   Calendar, Contacts, Drive and Photos on Linux/Windows. My account is new — may I post it on
   <date>, and is there anything I should follow?" Approval or a whitelist bypasses the filter.
2. **Earn real karma in the week before**, where you have real experience: answer questions and
   leave useful comments in r/linux, r/archlinux, r/hyprland, r/unixporn. A few good comments a
   day reaches 100+ quickly. Never buy karma or accounts — Reddit bans the account and can
   block the GitHub link site-wide.
3. **Order:** lenient, project-friendly subs first (r/opensource — note its no-LLM-text rule,
   r/coolgithubprojects, r/SideProject), then the large ones only after mods have answered.
4. **Check each post is visible** from a logged-out browser window a few minutes after posting;
   if not, message the mods politely with the link.
5. Disclose "I made this", reply to every comment, and never post the same text to many subs at
   once (spread per §2.7).

### 2.5 Product Hunt (Day 12, Saturday, 00:01 PT)

Why Saturday: weekend launches need far fewer upvotes for a top spot (vendor and founder data,
unverified), and PH's audience is weaker for Linux, so the cheaper slot makes sense. Only about
10% of launches get featured since 2024, so treat PH as a bonus.

- **Name:** Skykeep
- **Tagline (≤60):** `Your iCloud, at home on Linux and Windows` (41 chars)
- **Description (≤260):**
  > Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos as fast desktop apps for Linux and Windows. Your library lives on your computer, so it opens instantly and works offline. Free, open source, no account, no analytics. Not affiliated with Apple.

  (255 chars. Re-count after any edit.)
- **Topics:** Productivity, Open Source, Linux, Windows, Privacy.
- **Pricing:** Free.
- **Links:** the GitHub repo, and the release page.
- **Gallery, in order:**
  1. The 16:9 film (or a 30 s cut): the phone-to-desktop moment in the first 3 seconds.
  2. Notes (light), with the phone beside it (the social preview works).
  3. Reminders, with an alert showing Complete and Snooze.
  4. Calendar, week view.
  5. The Windows Start menu with the six Skykeep apps and the jump list.
  6. Drive as a folder in a Linux file manager.
  7. Settings → Account / Software Update (the "what it connects to" trust shot).
  8. Contacts or Photos.
- **First comment (maker), posted the same minute. Edit it into your own voice:**
  > Hi Product Hunt, I'm Yashwanth.
  >
  > I have an iPhone and a Linux laptop, and for years my notes and reminders on the computer meant a browser tab. Skykeep is what I wanted instead: six small apps, Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos, with one sign-in, on Linux and Windows.
  >
  > Three things I cared about:
  > • **Instant.** Your library lives on your computer, so every app opens immediately and works offline.
  > • **Live.** A note or reminder from your iPhone usually shows up in about a second.
  > • **Yours.** Free and open source, with no Skykeep account and no analytics. Export Everything gives you Markdown, calendar and contact files whenever you like.
  >
  > It's an independent project that works with iCloud, not an Apple product. If you use Advanced Data Protection, turn on "Access iCloud Data on the Web" first.
  >
  > I'd love to hear which app you'd use most, and what's missing for you.
- **Rules:** you may ask people to "check it out and leave feedback". You may **not** ask for
  upvotes, in any channel. Hunt it yourself (a third-party hunter gives no advantage).

### 2.6 Social threads (Day 1, 13:00–13:30 UTC)

Post the clip natively on every platform, with alt text written out. Use no more than 3–4
hashtags on Mastodon, and none in the X thread body.

**Mastodon (500 chars per post). Use mastodon.social or hachyderm.io: Fosstodon is invite-only
and bans commercial promotion.**
1. > Skykeep 0.1.0 is out: your iCloud Notes, Reminders, Calendar, Contacts, Drive and Photos as desktop apps on Linux and Windows.
   >
   > Type a note on your iPhone; it's on your desktop about a second later. Your library lives on your disk, so it opens instantly and works offline.
   >
   > Free, GPL-3.0, no account, no analytics.
   >
   > <repo> #Linux #OpenSource #Rust #iPhone
   >
   > [video, alt: "An iPhone and a laptop side by side. Text typed into a note on the phone appears in Skykeep Notes on the laptop a moment later."]
2. > On Linux, iCloud Drive and Photos also appear as real folders in your file manager, and Skykeep answers in GNOME and KDE search. Packages: deb, rpm, AppImage, AUR.
3. > Privacy, in one list: it connects to Apple, the calendars you subscribe to, and (AppImage/Windows only) GitHub for updates. Each connection is locked to its hosts in code. SECURITY.md has every host: <link>
4. > Not affiliated with Apple, just someone who loves his iPhone and his Linux laptop. Bug reports very welcome.

Tag in reply posts, not the first one, and only accounts that are active: @thelinuxEXP@mastodon.social,
@itsfoss@mastodon.social, @omgubuntu@floss.social, @9to5linux@floss.social,
@linuxiac@mastodon.social, @thelinuxcast@fosstodon.org, @latenightlinux@mastodon.social. Once
it's on Flathub, @flathub@floss.social. Tagging once is a courtesy; tagging repeatedly is spam.

**Bluesky (300 chars per post):** posts 1–3 above, trimmed. Tag @itsfoss.bsky.social,
@omgubuntu.co.uk, @9to5linux.com, @brodierobertson.xyz in a reply.

**X:** same as Bluesky. One post plus the clip, with the details in replies.

**LinkedIn (one post, first person, no hashtag wall):**
> Today I'm releasing Skykeep, a free, open-source app that brings iCloud Notes, Reminders, Calendar, Contacts, Drive and Photos to Linux and Windows as desktop apps.
>
> The problem was personal: I use an iPhone and a Linux laptop, and on the computer my notes lived in a browser tab. Skykeep keeps your library on your computer, so it opens instantly and works offline, and a note typed on the iPhone usually appears in about a second.
>
> Under the hood: one Rust engine shared by six apps, a reverse-engineered reading of how Apple Notes stores text (a CRDT inside protobuf inside gzip), and network connections locked in code to the hosts they need.
>
> It's GPL-3.0 and independent, not affiliated with Apple. If you have an iPhone and a Windows or Linux PC, I'd love your feedback: <repo>

### 2.7 Days 2–14

| Day | Main move | Also |
|---|---|---|
| **2 (Wed)** | r/software (Wednesday rule, flair); r/coolgithubprojects | Submit the winget manifest (no popularity bar; installer from GitHub Releases is fine). Fix the top Day-1 issue. |
| **3 (Thu)** | Publish the **dev-story blog post** (§2.9) on your site; cross-post to dev.to `#showdev` with the canonical URL | r/opensource (Promotional flair, your own words). Chocolatey package submitted (human-reviewed). |
| **4 (Fri)** | r/SideProject | Follow up **once** with any Tier 1 creator who replied "maybe". Optional r/selfhosted megathread comment. |
| **5 (Sat)** | **Rest and triage.** Ship v0.1.1 if fixes are ready | Release notes thank reporters by handle (with their OK). The updater turns the release into a visible event for AppImage and Windows users. |
| **6 (Sun)** | **iPhone day:** r/apple Sunday self-post (Pacific Sunday); HN plain link to the dev-story post, 13:00 UTC | Mastodon/Bluesky: the offline clip. |
| **7 (Mon)** | Retrospective against §7 | Update README FAQ with the three most-asked questions. Email hn@ycombinator.com about the Show HN only if it stalled and you haven't yet. |
| **8 (Tue)** | r/rust engineering post; This Week in Rust PR (blog post under community links) | Reply to every new issue within 24 h. |
| **9 (Wed)** | **Windows wave** (gate 4 must be true): r/windows or r/Windows11 after modmail; pitch Tier 2 Windows creators and XDA (§3) | Announce `winget install` once merged. |
| **10 (Thu)** | One distro sub (§2.4); AlternativeTo listing (alternative to iCloud for Windows and the iCloud web apps) | PRs to 0PandaDEV/awesome-windows and cuongtk8/awesome-linux-apps (§5.6). |
| **11 (Fri)** | Newsletters: Changelog News submit; It's FOSS follow-up if they haven't covered it; selfh.st only if the angle fits | Podcasts: LINUX Unplugged and Linux Matters contact forms with the blog post. |
| **12 (Sat)** | **Product Hunt, 00:01 PT** (§2.5) | Be at the keyboard for 08:00–20:00 PT replies (that's 21:30–09:30 IST, so plan sleep). |
| **13 (Sun)** | Thank-you post on Mastodon/Bluesky naming first contributors (with permission) | Write the 14-day numbers. |
| **14 (Mon)** | Release **v0.1.2 / v0.2.0** with the most-requested fix | If ≥50★: awesome-rust PR. If ≥100★: Scoop Extras request (issue first). If Flathub is live, announce it on Mastodon/Bluesky and tip OMG! Ubuntu; not another r/linux post (one a month at most). |

### 2.8 Partner and neighbour outreach (Days 3–10)

These people are genuine neighbours with the same users. Offer, don't ask:
- **Tether** (iMessage/SMS on Linux; zackbartel.com). The obvious "iPhone + Linux kit" pairing.
  Offer a mutual "see also" line in READMEs.
- **LocalSend** (AirDrop alternative) and **iDescriptor** (iPhone device tool, Rust). Same offer.
- Only add a "see also" if they agree, and only for projects you've actually tried.

### 2.9 Dev-story blog post: outline

**Title options:** "How Apple Notes stores your text (and how I got it onto Linux)" or "I wanted my
iPhone notes on Linux. Here's what was inside ENCRYPTED_BYTES." Write it yourself; same HN rule
as §2.3. Host it on your own site (yashwanthkamireddi.dev) so the HN link isn't a repo.

1. **Hook (3 lines):** the browser tab; the moment you decided to build it.
2. **The first wall:** the legacy Notes API returned an Apple placeholder, not your notes. Real
   records live in CloudKit's `changes/zone`.
3. **The puzzle:** `ENCRYPTED_BYTES` that was actually gzip; inside, protobuf; inside that, a
   topotext CRDT. A diagram of the layers. (Use synthetic note text only.)
4. **Merging with a phone typing at the same moment:** what a CRDT gives you, where it bit you.
   Tables as a CRDT within a CRDT.
5. **Making it feel instant:** the local library, optimistic writes with rollback, push for
   Notes and Reminders, the outbox for offline edits.
6. **Locking the network down:** how `apple_only()`, `listed_only()` and `releases_only()` refuse
   other hosts, IP literals and redirects; why the update client can't reach Apple and vice versa.
7. **Tauri on WebKitGTK:** video through a loopback server, the playbin3 stall, why animations
   came out (flicker on Wayland).
8. **What's next and how to help:** good-first-issues, packaging, the honest limits.
9. **Close:** the note appearing on the desktop: the moment it all worked.

### 2.10 GitHub release notes, v0.1.0

```markdown
# Skykeep 0.1.0

Your iCloud, at home on Linux and Windows.

![Skykeep](https://github.com/YashwanthKamireddi/Skykeep/raw/main/docs/media/hero.webp)

Skykeep brings Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos to your computer as
six desktop apps with one sign-in. Your library lives on your computer, so everything opens
instantly and works offline, and changes from your iPhone arrive on their own: Notes and
Reminders usually in about a second.

## Highlights
- **Notes:** headings, styles, checklists; pictures, scans, PDFs and tables shown in place;
  edits from your phone merge with yours.
- **Reminders:** lists, smart lists, subtasks, due dates, repeats, flags; alerts with
  Complete and Snooze.
- **Calendar:** day, week, month and year; drag to create and move; repeats, time zones,
  birthdays; subscribe to any webcal calendar.
- **Contacts:** every field; lists; find and merge duplicates; vCard and CSV import.
- **iCloud Drive and Photos:** browse, upload, preview; on Linux, also real folders in your
  file manager.
- **Everywhere:** search across every app, quick capture, GNOME and KDE search, a tray icon,
  **Export Everything** to Markdown, iCalendar and vCard.
- **Windows:** each app has its own Start-menu entry and taskbar button, plus a jump list.
- **Updates:** the Windows installer and the AppImage update themselves, after a signature
  check, never while you're typing or a change is still on its way.

## Install
| System | Download |
|---|---|
| Windows 10 and 11 | `Skykeep_0.1.0_x64-setup.exe` (or the `.msi`) |
| Ubuntu 24.04+, Debian 13, Linux Mint 22 | `Skykeep_0.1.0_amd64.deb` |
| Fedora 41+, openSUSE Tumbleweed | `Skykeep-0.1.0-1.x86_64.rpm` |
| Arch, CachyOS, EndeavourOS, Manjaro | `paru -S skykeep-git` |
| Any other distribution | `Skykeep_0.1.0_amd64.AppImage` |

Check your download against `SHA256SUMS`. The Windows installer isn't code-signed yet, so
Windows may ask you to confirm: **More info → Run anyway**.

## Good to know
- Advanced Data Protection: turn on **Access iCloud Data on the Web** (iPhone: Settings → your
  name → iCloud).
- Managed Apple Accounts from a school or workplace aren't supported yet.
- On Windows, Drive and Photos open inside the app.
- Skykeep connects to Apple, the calendars you subscribe to, and GitHub for its own updates.
  No Skykeep account, no analytics. [SECURITY.md](SECURITY.md) lists every host.

Skykeep is an independent open-source project, not affiliated with or endorsed by Apple.
```
(Fix the asset file names to the exact names the pipeline produces before publishing.)

---

## 3. Creator and press outreach

### 3.1 Rules

- **Personalise the first line for real:** name a specific recent video or article you
  actually watched or read. The references below were found on 2026-10-24; **open each one
  before citing it.**
- Pitch **T-7** with "free to cover from Day 1, 13:00 UTC". One follow-up at most, after 5 days.
- Never pay, never offer anything for coverage, never ask a creator to tell viewers to star.
- Send the right cut: YouTube long-form gets the 16:9 film plus a download link to raw
  footage; Shorts/Reels creators get the 9:16; news sites get the 1:1 loop or GIF (they embed it).
- Contacts: most YouTube business emails sit behind the About tab's "View email address"
  captcha; open it yourself. "Not found" below means no address is published on their own page.

### 3.2 Prioritised table

| # | Name | Platform | Link | Audience | Why they'd care | Cut | Route |
|---|---|---|---|---|---|---|---|
| 1 | **It's FOSS** (Sourav Rudra, news) | Site, newsletter, socials | [itsfoss.com](https://itsfoss.com) | Newsletter 20K–37K; Mastodon 59.8K; X 110K; IG 40K | Covered **Tether** (iPhone continuity on Linux) on 31 Aug 2026; the contact form has an "open source product that should reach more people" option | 1:1 + 16:9 | [contact form](https://itsfoss.com/contact-us/) |
| 2 | **OMG! Ubuntu** (Joey Sneddon) | Site | [omgubuntu.co.uk](https://www.omgubuntu.co.uk) | Mastodon 17K, Bluesky 11.3K | Writes up polished new Linux apps; the tip page asks for app tips with screenshots | 1:1 + screenshots | [tip form](https://omgubuntu.co.uk/tip) |
| 3 | **Snazzy Labs** (Quinn Nelson) | YouTube | [@snazzylabs](https://youtube.com/@snazzylabs) | 1.27M | Apple power users; posted "Open Source Is Beating Paid Mac Apps" (2 Oct 2026) | 16:9 | About tab |
| 4 | **The Linux Experiment** (Nick) | YouTube + Mastodon | [@thelinuxexp](https://youtube.com/@thelinuxexp) | 432K; Mastodon 32.9K | Weekly news show features new apps; "left the Mac, kept the iPhone" fits | 16:9 | About tab; [Mastodon](https://mastodon.social/@thelinuxEXP) |
| 5 | **LINUX Unplugged** (Jupiter Broadcasting) | Podcast | [show](https://www.jupiterbroadcasting.com/show/linux-unplugged/) | YouTube 53.5K + podcast | Reads listener tips on air; Rust, local-first | 16:9 link | [contact](https://linuxunplugged.com/contact) |
| 6 | **Brodie Robertson** | YouTube | [@BrodieRobertson](https://youtube.com/@BrodieRobertson) | 125K | Daily commentary; likes reverse-engineering stories and "what if Apple changes it" | 16:9 + blog post | About tab; [Bluesky](https://bsky.app/profile/brodierobertson.xyz) |
| 7 | **TechHut** | YouTube | [@TechHut](https://youtube.com/@TechHut) | 284K | App demos; Drive as a real folder | 16:9 | About tab |
| 8 | **The Linux Cast** | YouTube | [@thelinuxcast](https://youtube.com/@thelinuxcast) | 75.4K | Desktop workflow; real apps vs web wrappers | 16:9 | [Mastodon](https://fosstodon.org/@thelinuxcast) |
| 9 | **Switched to Linux** | YouTube | [@switchedtolinux](https://youtube.com/@switchedtolinux) | 90.6K | His audience is people leaving Windows/Mac | 16:9 | About tab |
| 10 | **This Week in Rust** | Newsletter | [repo](https://github.com/rust-lang/this-week-in-rust) | Mastodon 4.7K + newsletter | A shipped Rust + Tauri desktop app; the blog post | blog link | PR (community links) |
| 11 | **9to5Linux** (Marius Nestor) | Site | [9to5linux.com](https://9to5linux.com) | Mastodon 21.1K | Versioned release news; best on a release (v0.1.x/v0.2) | 1:1 | [contact](https://9to5linux.com/contact-us) |
| 12 | **Linux Matters** (Popey, Wimpress, Johnson) | Podcast | [linuxmatters.sh](https://linuxmatters.sh) | Mastodon 1.5K, Bluesky 1.9K | Popey cares about app packaging (Flathub) | link | [contact](https://linuxmatters.sh/contact) |
| 13 | **Late Night Linux** | Podcast | [latenightlinux.com](https://latenightlinux.com) | Mastodon 3.9K | Picks/discoveries style (segment unverified) | link | [contact](https://latenightlinux.com/contact/) |
| 14 | **DistroTube** | YouTube | [@DistroTube](https://youtube.com/@DistroTube) | 276K | GPL-3.0, no account, no analytics | 16:9 | About tab |
| 15 | **Mental Outlaw** | YouTube | [@MentalOutlaw](https://youtube.com/@MentalOutlaw) | 798K | Privacy; covered an open-source Adobe clone on 17 Oct 2026 | 16:9 + SECURITY.md | About tab |
| 16 | **Chris Titus Tech** | YouTube | [@christitustech](https://youtube.com/@christitustech) | 782K | Windows + Linux crossover. **Windows wave** | 16:9 | About tab |
| 17 | **Brian Burgess (XDA)** | Site | [his iCloud-on-Windows article](https://www.xda-developers.com/using-icloud-on-windows-doesnt-need-to-be-a-nightmare/) | XDA | Wrote "Using iCloud on Windows doesn't need to be a nightmare". **Windows wave** | 1:1 + 16:9 | XDA author page |
| 18 | **Britec09** | YouTube | [@Britec09](https://youtube.com/@Britec09) | 905K | Daily Windows; "no account to sign up for" angle. **Windows wave** | 16:9 | About tab |
| 19 | **ThioJoe** | YouTube (+ Shorts) | [@thiojoe](https://youtube.com/@thiojoe) | 3.14M | "The iCloud apps Windows doesn't have". **Windows wave, long shot** | 9:16 + 16:9 | About tab |
| 20 | **Brandon Butch** | YouTube | [@brandonbutch](https://youtube.com/@brandonbutch) | 1.44M | iPhone tips; "your iPhone's Notes on a PC" | 9:16 + 16:9 | About tab |
| 21 | **Windows Central** | Site + video | [@windowscentral](https://youtube.com/@windowscentral) | 420K | iPhone users on Windows. **Windows wave** | 1:1 | site tips |
| 22 | **Linux Tex** | YouTube | [@linuxtex](https://youtube.com/@linuxtex) | 143K | Beginner "apps you need" lists | 16:9 | About tab |
| 23 | **Veronica Explains** | YouTube | [@VeronicaExplains](https://youtube.com/@VeronicaExplains) | 187K | Owning your data | 16:9 | About tab |
| 24 | **Changelog News** | Newsletter/podcast | [submit](https://changelog.com/news/submit) | unpublished | The engineering story | blog link | form |
| 25 | **Linuxiac** | Site | [linuxiac.com](https://linuxiac.com) | Mastodon 3.5K | App releases | 1:1 | contact page (couldn't load it) |

**Short-form (9:16):** reach was the weakest part of the research. TikTok is blocked in India,
so plan for **YouTube Shorts and Instagram Reels**. Most channels above also post Shorts, so the
9:16 cut goes with every YouTube pitch. Small TikTok accounts found (sizes unverified):
@lordgains418 (Windows→Linux apps), @emanschi_games (Arch apps), @28_all_day (Linux Mint).
Low priority; a short DM with the 9:16 is enough. The best short-form channel is **your own**:
post the 9:16 on YouTube Shorts and Reels on Day 1, with the magic moment in the first 2 seconds.

**Skip for now:** ExplainingComputers (distro-focused), Learn Linux TV (sysadmin), Phoronix
(kernel/hardware), GamingOnLinux (gaming), The Verge/Ars (only if it becomes news), Console.dev
(dev tools only), Keep Productive/Ali Abdaal (unverified, weak fit).

### 3.3 Pitch templates (short, respectful, edit each one)

**Press (It's FOSS / OMG! Ubuntu / 9to5Linux / Linuxiac)**
> **Subject:** Skykeep: iCloud Notes, Reminders and Drive as desktop apps on Linux (open source)
>
> Hi <name>,
>
> Your piece on <specific article, e.g. Tether bringing iPhone continuity to Linux> is why I'm writing: Skykeep is the other half of that story.
>
> Skykeep is a free, GPL-3.0 app that brings iCloud Notes, Reminders, Calendar, Contacts, Drive and Photos to Linux (and Windows) as desktop apps. Your library lives on your disk, so it opens instantly and works offline; a note typed on an iPhone usually shows up in about a second; and iCloud Drive appears as a real folder in Files or Dolphin.
>
> [embedded 1:1 loop]
>
> It goes public on <T-2 date>, and you're welcome to cover it from <Day 1>, 13:00 UTC. Press kit (film, screenshots, logo, fact sheet): <link>. Packages: deb, rpm, AppImage, AUR.
>
> Honest limits: Advanced Data Protection users need "Access iCloud Data on the Web" on, and it's an independent project, not affiliated with Apple.
>
> Happy to answer anything.
> Yashwanth Kamireddi

**YouTube, Linux (Linux Experiment / TechHut / Linux Cast / Switched to Linux)**
> **Subject:** iPhone + Linux: Notes and Reminders as real apps (for a news segment?)
>
> Hi <name>, I enjoyed <specific video>. I think your viewers who kept their iPhone after switching might like Skykeep: iCloud Notes, Reminders, Calendar, Contacts, Drive and Photos as desktop apps on Linux, open source, with a note from the phone appearing in about a second.
>
> 60-second film (16:9) and raw real-time footage you're free to use: <press kit>. It's public from <date>; coverage welcome from <Day 1>.
>
> No account, no analytics, GPL-3.0. Thanks either way.
> Yashwanth

**Snazzy Labs (Apple-power-user angle)**
> **Subject:** Open source, from the other direction: iCloud on Linux and Windows
>
> Hi Quinn, your "Open Source Is Beating Paid Mac Apps" video stuck with me. I built something for the people on the other side of the wall: Skykeep, a free, open-source app that puts iCloud Notes, Reminders, Calendar, Contacts, Drive and Photos on Linux and Windows as desktop apps, with a note from the iPhone landing on the PC in about a second.
>
> Film and press kit: <link>. Public from <date>. Not affiliated with Apple; it signs in the way icloud.com does, with Apple's own two-factor.
>
> Thanks for reading,
> Yashwanth

**Privacy channel (Mental Outlaw / DistroTube)**
> **Subject:** An iCloud client with no account, no analytics, and every connection listed
>
> Hi <name>, after <specific video> I thought you might find this one interesting to pick apart. Skykeep brings iCloud Notes, Reminders and Drive to Linux. It has no account and no analytics; SECURITY.md lists every host it connects to (Apple, your calendar subscriptions, GitHub for updates), each locked in code, with tests. It also says plainly what it stores in your keyring and who can read it.
>
> Repo and SECURITY.md: <link>. Press kit: <link>. GPL-3.0.
> Yashwanth

**Windows wave (Chris Titus / Britec / ThioJoe / XDA / Windows Central)**, send on Day 9 only
after the real-PC gate
> **Subject:** Apple Notes and Reminders on Windows, in the Start menu (open source)
>
> Hi <name>, <specific video/article line>. iCloud for Windows covers Drive and Photos, but there's no Notes or Reminders app. Skykeep fills that in: free, open-source desktop apps for Notes, Reminders, Calendar, Contacts, Drive and Photos, each with its own Start-menu entry and taskbar button, installing per user with no admin password.
>
> 16:9 and 9:16 cuts: <press kit>. The installer isn't code-signed yet, so SmartScreen asks once; I'd rather say that up front.
> Yashwanth

**Podcast (LINUX Unplugged / Linux Matters / Late Night Linux)**: three lines via the contact
form: what it is, the one-sentence engineering hook (Apple Notes' text is a CRDT in protobuf in
gzip), and the links.

---

## 4. Press kit

Host it **in the repo** at `press/` (or a GitHub Pages page) once public, so the link is stable
and nothing goes to a third-party cloud. Link it from the README footer and every pitch.

### 4.1 Contents

**Films** (all with burned-in captions, plus an `.srt`; no music needed, or royalty-free with
the licence noted):
1. **Launch film, 16:9, 60–75 s, 1080p/4K 60 fps.** The storyboard in
   `marketing-2026-10-21.md` §5 still works, with **one change**: shot 7 now shows SECURITY.md's
   host list ("Apple, your calendars, GitHub for updates — nothing else") instead of "talks only
   to Apple".
2. **Vertical, 9:16, 20–30 s.** The magic moment in the first 2 seconds: phone in hand, type,
   the laptop behind it updates. Then Reminders tick, then Start menu, then the end card
   "Skykeep. Free and open source." Real time, with a small "real time, not sped up" caption.
3. **Square loop, 1:1, 6–8 s, as MP4 and GIF (<5 MB).** Just the phone-to-desktop moment.
4. **Raw footage**, unedited and real time: phone-to-desktop Notes, phone-to-desktop Reminders,
   offline edit and resync, Windows Start menu and jump list, Drive in a Linux file manager.
   Creators like raw B-roll.

**Filming rules:** a **demo Apple Account with synthetic data only**; a real iPhone filmed on a
tripod and a real screen, never mockups; no Apple logo in frame (cover or angle the phone's
back); no notification banners showing real names; no university or workplace names anywhere.

**Screenshots:** the 14 WebPs in `docs/media/` plus PNG exports at 2560×1600 (light and dark):
Notes, Reminders, Calendar, Contacts, Drive, Photos, Settings, Sign-in. Add: the Windows Start
menu with the six apps; the jump list; Drive in Nautilus and in Dolphin; a reminder alert with
Complete and Snooze; GNOME search showing a Skykeep result.

**Logo:** `packaging/icons/skykeep-1024.png`, plus SVG; the small tray/notification variant;
on light, on dark, and the mark alone. Usage note: don't recolour or pair it with Apple marks.

**Fact sheet** (§4.2), **maker bio and quotes** (§4.3), **FAQ** (link to the README FAQ and
SECURITY.md), and **the disclaimer**.

### 4.2 Fact sheet (all literally true as of 2026-10-24; re-check numbers on launch day)

> **Skykeep — fact sheet**
>
> **What it is:** a free, open-source desktop app that brings iCloud Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos to Linux and Windows as six apps with one sign-in.
>
> **Made by:** Yashwanth Kamireddi, an independent developer. Solo project.
>
> **First release:** v0.1.0, <date>.
>
> **Platforms:** Windows 10 and 11 (installs per user, no administrator password). Linux: Ubuntu 24.04+, Linux Mint 22, Debian 13, Fedora 41+, openSUSE Tumbleweed and later (.deb, .rpm, AppImage), Arch-based systems via the AUR; GNOME, KDE and other desktops, Wayland or X11.
>
> **Price:** free. **Licence:** GPL-3.0. **Source:** github.com/YashwanthKamireddi/Skykeep
>
> **Key facts:**
> - Your library lives on your computer: every app opens instantly and works offline; edits made offline sync when you reconnect.
> - Notes and Reminders changes from an iPhone usually arrive in about a second (Apple's push channel); Calendar and Contacts are checked every few seconds while in use.
> - Notes edited on the phone and the computer at the same moment are merged.
> - On Linux, iCloud Drive and Photos also appear as real folders in the file manager; Skykeep answers in GNOME and KDE search.
> - On Windows, each app has its own Start-menu entry and taskbar button, plus a jump list.
> - Export Everything: notes to Markdown, events to iCalendar, contacts to vCard, files as they are.
> - Privacy: no Skykeep account, no Skykeep servers, no analytics or telemetry. It connects to Apple, the calendars you subscribe to, and GitHub's release servers for its own updates; each connection is locked to its hosts in code. Sign-in uses Apple's SRP protocol, so the password is never sent; the password and session are kept in the system keyring.
> - Signed in-app updates for the Windows installer and AppImage; package-manager installs update through the package manager.
>
> **Built with:** Rust (a five-crate engine) and Tauri 2, with a TypeScript interface. About 457 commits and about 1,000 Rust tests at launch.
>
> **Works with Advanced Data Protection** when "Access iCloud Data on the Web" is on. Managed Apple Accounts aren't supported yet.
>
> **Name:** *Sky* for the cloud; a *keep* is a castle's safest tower. Your cloud, kept safe on your own computer.
>
> **Contact:** <launch email or GitHub Discussions>. Security reports: GitHub private vulnerability reporting.
>
> *Skykeep is an independent project and has not been authorized, sponsored or otherwise approved by Apple Inc. Apple, iCloud and iPhone are trademarks of Apple Inc., registered in the U.S. and other countries and regions.*

### 4.3 Maker quotes

Press will quote you, so **write two or three short quotes yourself, in your own words**, and
only say things you mean. Suggested themes (not words to copy):
- Why: the browser tab, and loving both the iPhone and Linux.
- What you're proudest of: the first time a note from the phone landed on the desktop.
- On trust: "Read SECURITY.md; if a line in it stops being true, that's a bug."

**Social proof:** add real user quotes and creator mentions to the press kit and README **only
after they exist**, with permission and a link to the source. Never invent, paraphrase into
something stronger, or imply a quote from someone who only starred the repo.

---

## 5. GitHub growth

### 5.1 README conversion checklist

What turns a visitor into a star, in the first screen: **what it is, proof it works, how to get
it.** Current README status:

- [x] Logo, name, one line ("Your iCloud, at home on Linux and Windows.").
- [x] Hero animation in the first screen; six-app grid with light/dark.
- [x] Honest FAQ (password, ADP, Managed accounts, offline, speed).
- [x] Privacy section with the real host list and a link to SECURITY.md.
- [ ] **Download buttons in the first screen** (above the six-app grid): "Download for Windows"
  and "Download for Linux" linking to the latest release. Visitors who can't find the download
  in 5 seconds leave.
- [ ] **A real-phone clip near the top** (GIF or a short video), *in addition to* the hero: the
  phone-to-desktop moment is the proof that converts, and the hero shows the desktop only.
- [ ] **A one-line trust row** under the tagline: "Free and open source · No account · No
  analytics · Works offline". All true.
- [ ] Uncomment the dynamic badges once public (release, build, AUR when live, Flathub when
  live). Leave the stars badge out until it isn't tiny.
- [ ] A short "How it works" link to the blog post (developers star the engineering).
- [ ] winget line in Install once merged.
- [ ] Keep it calm: no wall of badges, no emoji headings, no "blazingly fast".

### 5.2 About box, topics, social preview

**Description (replaces the stale one in `github-setup.md` §2):**
```text
Your iCloud, at home on Linux and Windows: Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos as fast desktop apps. Local-first, so everything opens instantly and works offline. No account, no analytics. Free and open source, built with Rust + Tauri. Not affiliated with Apple.
```

**Topics (20 max):** `icloud`, `icloud-drive`, `icloud-photos`, `icloud-notes`, `apple-notes`,
`apple-reminders`, `apple-calendar`, `linux`, `linux-desktop`, `linux-app`, `windows`,
`desktop-app`, `local-first`, `offline-first`, `privacy`, `rust`, `tauri`, `tauri-app`,
`cloudkit`, `crdt`.

**Social preview:** `docs/media/social-preview.png` is ready (1280×640, 296 KB, under GitHub's
1 MB limit). Upload it right after going public.

**Website field:** your blog post or a project page once it exists; never a personal homepage.

### 5.3 Discussions

Announcements, Q&A, Ideas, Show and tell (screenshots of Skykeep on people's desktops become
real, shareable social proof). Pin a welcome post.

### 5.4 Pinned issues (up to three)

1. **"Skykeep 0.1 — what works, what's next"**: the roadmap in plain words (Drive/Photos
   folders on Windows; editing tables; attachments; accessibility pass; Flathub), and links to
   good-first-issues. **No beta/preview labels.**
2. **"Install reports: tell us your distro, desktop and Windows version"**: one place for
   "works on X" comments; it doubles as honest social proof.
3. During launch week only: **"Known issue: <x>"** if something real comes up (§6.3).

### 5.5 Good-first-issues (file 12–15 before launch, each an afternoon)

From the open lists in `ROADMAP.md` and `OPEN-ITEMS-2026-10-22.md`:
- Packaging recipes: **Nix flake**, **Fedora COPR**, **Scoop manifest** (a personal bucket until
  Extras' 100★ bar), **Chocolatey** package, **winget** manifest updates.
- Wake the offline outbox when the network returns (NetworkManager `StateChanged` / the
  NetworkMonitor portal), labelled S in OPEN-ITEMS.
- Accessibility labels on toolbar buttons; an Orca or NVDA walkthrough report (`help wanted`).
- Keyboard shortcuts missing per app (pick specific ones).
- vCard and CSV import edge cases (attach synthetic sample files).
- A man page or `--help` text for the command-line flags (`--app`, `--new`).
- Screenshots for the Flathub metainfo (`<screenshots>` is required and missing).
- Docs: CONTRIBUTING code map per crate.

**help wanted** for expertise: Windows Cloud Files API (Drive as a folder on Windows), Flatpak
portals, a code-signing route (SignPath Foundation).

**Never** file translation issues; translations aren't wanted for now.

### 5.6 Lists and directories

| Where | Link | Rules (read 2026-10-24) | When |
|---|---|---|---|
| winget | [docs](https://learn.microsoft.com/en-us/windows/package-manager/package/repository) | Silent install, installer from the publisher (GitHub Releases is fine), clean uninstall, AV scan; one version per PR | Day 2 |
| Chocolatey | [moderation](https://docs.chocolatey.org/en-us/community-repository/moderation/) | Human-reviewed; ProjectUrl, LicenseUrl, checksums; VERIFICATION.txt for embedded binaries | Day 3 |
| AlternativeTo | [FAQ](https://alternativeto.net/faq) | "Suggest new application"; many declined; no tracking tags | Day 10 |
| 0PandaDEV/awesome-windows | [CONTRIBUTING](https://github.com/0PandaDEV/awesome-windows/blob/main/CONTRIBUTING.md) | One item per PR, title case, alphabetical, or the "Add Application" issue form; rejects "vibecoded slop" | Day 10 |
| cuongtk8/awesome-linux-apps | [repo](https://github.com/cuongtk8/awesome-linux-apps) | `[Name](link) - Short description. 👏` (👏 = open source), one per PR, alphabetical | Day 10 |
| rust-unofficial/awesome-rust | [CONTRIBUTING](https://github.com/rust-unofficial/awesome-rust/blob/main/CONTRIBUTING.md) | **Needs >50 stars** (or >2,000 downloads); Applications section; alphabetical; CI badge | Day 14 if ≥50★ |
| Scoop Extras | [request form](https://github.com/ScoopInstaller/Extras/blob/master/.github/ISSUE_TEMPLATE/package-request.yml) | **≥100 stars and/or 50 forks**; open an issue first; portable install, `checkver`/`autoupdate` | When ≥100★ |
| OpenAlternative | [submit](https://openalternative.co/submit) | Feeds piotrkulpinski/open-source-alternatives; free queue reported at ~5 months (unverified) | Day 10 (free queue) |
| Lissy93/awesome-privacy | [CONTRIBUTING](https://github.com/Lissy93/awesome-privacy/blob/main/.github/CONTRIBUTING.md) | **First stable release must be >4 months old**; human-written PR text; fit is arguable | Not before ~Mar 2027 |
| Flathub | [requirements](https://docs.flathub.org/docs/for-app-authors/requirements) | Builds, portals, screenshots in metainfo, verified via GitHub | Submit T-21; announce when live |

**Not open:** luong-komorebi/Awesome-Linux-Software (archived May 2026 over self-promotion),
tauri-apps/awesome-tauri (no longer accepts applications), Awesome-Windows/Awesome (404), both
awesome-local-first lists (closed or stale), opensourcealternative.to (self-hosted only),
Privacy Guides (needs a threat model; poor fit), Lobsters (invite-only; new accounts can't
self-promote for 70 days).

### 5.7 When to ask for stars, honestly

- **In the README footer, once:** "If Skykeep saves you a trip to the browser, a star helps
  other iPhone owners on Linux and Windows find it." True and specific.
- **In release notes**, the same single line.
- **Never** in HN, Reddit or PH posts (on PH and HN that's vote solicitation), never in DMs,
  never as a condition for support, and never as a pop-up in the app. A star prompt inside the
  app would also contradict the calm brand.
- Stars are a by-product of the real work: fast replies to issues, visible fixes and good
  release notes.

---

## 6. Risks and responses

### 6.1 Likely objections and calm, true answers

Same facts everywhere. Short. Link the source. Never argue twice with the same person.

**"Isn't this against Apple's terms?"**
> Skykeep signs in the way icloud.com does, with Apple's own two-factor, and syncs your own data when you use it. It doesn't scrape, bulk-download for others, or pretend to be an Apple device. Unofficial iCloud clients like icloudpd and rclone's iCloud backend have existed for years. I can't promise how Apple reads its terms. It's your account, and it's open source, so you can see exactly what it does.

(Never say "Apple allows this" or "it's legal". Neither is yours to claim.)

**"Why should I give a stranger's app my Apple password?"**
> You shouldn't on trust alone, which is why it's open source. Sign-in uses Apple's SRP protocol, so the password is never sent, not even to Apple. It's kept in your system keyring so Skykeep can sign back in by itself. SECURITY.md is explicit that any program running as you can read keyring entries. There's no Skykeep server for it to go to; the code that limits each connection is in `apple_only()` and its tests.

**"Why not an app-specific password?"**
> Apple's app-specific passwords only reach mail, contacts and calendars. Notes, Reminders, Drive and Photos need the full sign-in with two-factor, the same one icloud.com uses. rclone's iCloud backend has the same requirement.

**"Why not just use icloud.com?"**
> If it works for you, great. Skykeep is for when you want your library on your disk: instant, offline, desktop alerts, Drive as a folder, search from your launcher, and an export whenever you want.

**"It's Tauri, so it's not native."**
> Fair: the windows are Tauri webviews. The engine (sync, storage, CloudKit decoding, the network locks) is Rust. "Native" in the README means real apps on your machine, not a website in a window.

**"Apple will break it."**
> Possibly. When Apple changes something, your library stays readable offline, Skykeep tells you instead of failing silently, and the fix ships as a signed update (AppImage and Windows) or through your package manager.

**"It phones home to GitHub."**
> Only the AppImage and Windows installer copies, once a day, for `latest.json`, and you can turn it off in Settings ▸ Software Update. It sends nothing about you, though GitHub sees your IP like any website. deb, rpm, AUR and Flatpak copies never contact GitHub.

**"Windows says it's unsafe."**
> The installer isn't code-signed with a certificate yet, so SmartScreen asks you to confirm. The checksums are on the release page, the builds come from the public CI, and in-app updates are signature-checked. Signing is on the roadmap (SignPath Foundation for open source).

**"Is my data encrypted on disk?"**
> Cookies are encrypted with a key in your keyring. The library itself isn't encrypted by Skykeep; use full-disk encryption, as you would on a Mac.

**"Advanced Data Protection?"**
> Works with "Access iCloud Data on the Web" turned on. With it off, Apple doesn't let any app outside your devices read the data, Skykeep included, and Skykeep says so instead of working around it.

**"Was this built with AI?"** If asked, answer truthfully and briefly, in your own words.
Don't lead with it, and don't deny it. What matters to readers is that it works, is tested,
and that you stand behind it.

**"Another iCloud-on-Linux project already exists."**
> Yes, and they're good work: icloud-for-omarchy does native Notes on Arch, rclone does Drive, icloudpd does Photos. Skykeep puts the six apps together with one sign-in and an offline library, on Linux and Windows.

### 6.2 Tactics that would backfire (don't)

- **Asking anyone to upvote** on HN, Reddit or PH, including friends, a WhatsApp group or your
  class. HN and PH detect voting rings and penalise or ban the post, account and domain.
- Sharing the HN link with "go comment". Same rule.
- **Second accounts** to comment, upvote or post "I tried it, it's great". That's
  astroturfing; mods compare IPs and account age.
- **Bought stars, star exchanges or bot stars.** An ICSE 2026 study found about 6M fake stars,
  and the boost fades within two months and then becomes a liability.
- **LLM-written posts on HN** (explicitly asked against since March 2026), r/opensource,
  Privacy Guides and awesome-privacy (human-written text required).
- Cross-posting the same text to many subs on one day.
- False urgency ("launch-day only"), fake scarcity, or countdown timers. Nothing about Skykeep
  is scarce: it's free.
- Any superlative you can't prove ("first", "only", "fastest"), any Apple imagery, any
  "official".
- Deleting and reposting a flopped HN post.
- Arguing with critics. Answer once, with facts, then thank them.

### 6.3 Crisis plan for launch day

**Before Day 1:** a `v0.1.1` hotfix rehearsal done (gate 3); a pinned-issue template ready; the
README's top line editable in one commit.

| Situation | First 30 minutes | Then |
|---|---|---|
| **Sign-in broken for many** (Apple changed something, or a bug) | Pin "Known issue: sign-in" with what's affected and what's safe ("your library stays readable offline; nothing is deleted"). One reply in each thread linking it. | Fix; ship v0.1.1 through the release pipeline; the updater reaches AppImage/Windows users; post the fix in the same threads. |
| **Data-loss report** (a note or reminder lost or duplicated) | Treat as P0. Ask for steps, never for passwords or token logs. If it's real, say so publicly and advise the safe action (e.g. avoid that edit path). | Fix with a test that fails without it; release; write a short, honest postmortem in the issue. |
| **Installer fails on a distro or on Windows** | Confirm the reproduction; add a line to the pinned issue with the workaround (e.g. AppImage `--appimage-extract-and-run`). | Fix the package in v0.1.1. |
| **The HN thread turns hostile** (ToS, password, "AI slop") | Reply once with facts and links (§6.1). Don't defend twice. Upvote nothing. | Let the work answer: fix what's fair, credit the critic in release notes. |
| **A trademark or legal email** | Don't reply in public. Read it carefully, change any wording or imagery it reasonably asks for, and get advice if it's more than that. | Note it in a private file, not in the repo. |
| **You're overwhelmed** | Use the pinned issues as single places to collect reports. Reply "thanks, tracking in #N" and move on. | Turn off notifications for starred-only events; sleep. A 24-hour response time is fine after Day 2. |

There are **no Skykeep servers**, so launch traffic can't take the app down. The only
dependency is GitHub Releases. Say that if anyone asks about scaling.

---

## 7. Metrics and targets

**Sources only:** public counters. No telemetry in the app, ever.
- GitHub stars, forks, watchers, traffic (Insights → Traffic: views, unique visitors, referrers).
- Release asset download counts (GitHub API). **`latest.json` downloads** count the daily update
  checks from AppImage and Windows copies: a rough, anonymous proxy for active installs, counted
  by GitHub, not by Skykeep. Describe it that way if you ever cite it.
- AUR votes and popularity; Flathub install stats once live; winget has no public counts.
- HN points and rank (manual); Reddit upvotes; PH rank.
- Issues: median first response; ratio of "it works" install reports to "it failed".
- Coverage log: articles, videos, podcasts, with links.

**Targets.** These are planning numbers, not forecasts, in three bands. The category's HN
history (most Show HNs at 1–10 points) makes "floor" the honest base case. Press and Reddit
carry the launch.

| Day | Stars (floor / good / breakout) | Downloads (all assets, excl. `latest.json`) | Other |
|---|---|---|---|
| 1 | 75 / 300 / 1,500 | 150 / 600 / 3,000 | ≥1 article live; r/linux post ≥100 upvotes; every comment answered |
| 2 | 125 / 450 / 2,200 | 250 / 900 / 4,500 | winget PR open |
| 3 | 175 / 600 / 2,800 | 350 / 1,200 / 5,500 | Blog post live; 2 articles |
| 5 | 225 / 750 / 3,300 | 450 / 1,500 / 6,500 | v0.1.1 shipped if needed; median issue response <12 h |
| 7 | 300 / 900 / 3,800 | 600 / 1,800 / 7,500 | 3 pieces of coverage; ≥1 external PR |
| 10 | 375 / 1,100 / 4,300 | 750 / 2,300 / 9,000 | Windows wave sent; winget merged |
| 14 | 450 / 1,300 / 5,000 | 900 / 2,800 / 10,000 | ≥50★ unlocks awesome-rust; ≥100★ unlocks Scoop; AUR ≥20 votes; 3 external contributors |
| 30 | 700 / 1,800 / 6,500 | 1,500 / 4,000 / 14,000 | 6 coverage items; 0 open data-loss bugs |

**Daily leading indicators (watch these over stars):**
1. Issues mentioning **sign-in** or **sync wrong**. Trust breaks faster than stars grow.
2. Install success ratio (pinned install-report issue): aim above 5:1.
3. Median first response to issues: under 12 h in week 1, under 24 h after.
4. Referrers in GitHub Traffic: tells you which channel actually worked. Double down there.

**Review on Day 7** and reset the bands to what really happened.

---

## 8. One-page checklist

- [ ] Gates 1–8 (§0) true
- [ ] Film shot on a demo account with a real phone (§4.1); 16:9, 9:16, 1:1 cut
- [ ] Press kit in `press/`; fact sheet numbers re-run
- [ ] Stale lines in `github-setup.md` replaced (§0)
- [ ] README: download buttons, phone clip, trust row (§5.1)
- [ ] About box, topics, social preview (§5.2); Discussions; pinned issues; 12–15 good-first-issues
- [ ] T-7 pitches sent (Tier 1: rows 1–10)
- [ ] HN text written **by you** (§2.3); blog post written by you (§2.9)
- [ ] Day 1 blocked out; hotfix path rehearsed
