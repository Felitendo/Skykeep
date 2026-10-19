#!/usr/bin/env bash
# Everything that has to be true before a change is worth keeping.
# One command, so "is it broken?" is answerable rather than a matter of trust.
set -uo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"

fail=0
step() {
  printf '\n\033[1m── %s ──\033[0m\n' "$1"; shift
  if "$@"; then printf '\033[32mok\033[0m\n'; else printf '\033[31mFAILED\033[0m\n'; fail=1; fi
}

# The app must serve its own bundle. Pointing it at a dev server made it depend on `npm run
# dev` being up and healthy, which produced both a connection error and a blank window.
step "app is standalone" bash -c '! grep -q devUrl src-tauri/tauri.conf.json'

step "rust format"      cargo fmt --all -- --check
step "rust lint"        cargo clippy --workspace --all-targets -- -D warnings
step "rust tests"       cargo test --workspace --quiet
step "typescript"       npx tsc --noEmit
step "calendar layout"  node --test .uiverify/calendar-model.test.mjs
step "notes layout"     node --test .uiverify/notes-model.test.mjs
step "notes editor model" node --test .uiverify/notes-rich.test.mjs
step "frontend build"   npm run build

# The browser checks need the built bundle served.
npx vite preview --port 4173 --strictPort >/tmp/quince-preview.log 2>&1 &
preview=$!
trap 'kill $preview 2>/dev/null' EXIT
for _ in $(seq 20); do curl -sf -o /dev/null http://localhost:4173/ && break; sleep 0.5; done

step "ui renders"       node .uiverify/shots.mjs
step "nav behaviour"    node .uiverify/assert.mjs
step "accessibility"    node .uiverify/a11y.mjs
step "keyboard only"    node .uiverify/keyboard.mjs
step "somewhere else"   node .uiverify/elsewhere.mjs
step "writing a note"   node .uiverify/notes-compose.mjs
step "deleting a note"  node .uiverify/notes-trash.mjs
step "pinning a note"   node .uiverify/notes-pin.mjs
step "a long note"      node .uiverify/notes-long.mjs
step "uploading"        node .uiverify/drive-upload.mjs
step "calendar grids"   node .uiverify/calendar.mjs
step "every size"       node .uiverify/sizes.mjs
step "narrow windows"   node .uiverify/compact.mjs
step "a screen failing"  node .uiverify/boundary.mjs
step "coming back"       node .uiverify/resume.mjs
step "birthdays, holidays" node .uiverify/occasions.mjs
step "drive folders"    node .uiverify/drive-nav.mjs
step "birthdays in contacts" node .uiverify/contact-birthday.mjs
step "contact labels"    node .uiverify/contacts-labels.mjs
step "empty today"     node .uiverify/reminders-empty.mjs
step "calendar keys"   node .uiverify/calendar-keys.mjs
step "reminders instant" node .uiverify/reminders-instant.mjs
step "calendar instant"  node .uiverify/calendar-instant.mjs
step "notes instant"     node .uiverify/notes-instant.mjs
step "drive sort"        node .uiverify/drive-sort.mjs
step "drive trash"       node .uiverify/drive-trash.mjs
step "drive info"        node .uiverify/drive-info.mjs
step "drive delete"      node .uiverify/drive-delete.mjs
step "notes folders"     node .uiverify/notes-folders.mjs
step "list info"         node .uiverify/reminders-list-info.mjs
step "reminder date"     node .uiverify/reminders-date.mjs
step "reminder keys"     node .uiverify/reminders-keys.mjs
step "contacts order"    node .uiverify/contacts-order.mjs
step "contacts lists"    node .uiverify/contacts-lists.mjs
step "drive keys"        node .uiverify/drive-keys.mjs
step "drive drag"        node .uiverify/drive-drag.mjs
step "drive recents"     node .uiverify/drive-recents.mjs
step "drive gone folder" node .uiverify/drive-gone.mjs
step "settings"          node .uiverify/settings.mjs
step "calendar day steps" node .uiverify/calendar-daystep.mjs
step "sidebar fold"      node .uiverify/sidebar-fold.mjs
step "pending writes"    node .uiverify/pending-writes.mjs
step "row hover"         node .uiverify/row-hover.mjs
step "reminders switch"  node .uiverify/reminders-switch.mjs
step "upload cancel"     node .uiverify/drive-upload-cancel.mjs
step "notes tables"      node .uiverify/notes-table.mjs
step "reminder menu"     node .uiverify/reminders-context.mjs
step "notes media"       node .uiverify/notes-media.mjs
step "webkit: tables"    env ENGINE=webkit node .uiverify/notes-table.mjs
step "webkit: labels"    env ENGINE=webkit node .uiverify/contacts-labels.mjs
step "webkit: row hover" env ENGINE=webkit node .uiverify/row-hover.mjs
step "webkit: fold"      env ENGINE=webkit node .uiverify/sidebar-fold.mjs
step "reminder colours"  node .uiverify/calendar-reminder-colour.mjs
step "calendar dblclick"  node .uiverify/calendar-dblclick.mjs
step "menus"             node .uiverify/menus.mjs
step "webkit: menus"     env ENGINE=webkit node .uiverify/menus.mjs
step "contacts vcard"    node .uiverify/contacts-vcard.mjs
step "contacts select"   node .uiverify/contacts-select.mjs
step "calendar alerts"   node .uiverify/calendar-alerts.mjs
step "calendar repeat"   node .uiverify/calendar-repeat.mjs
step "reminder add/done"  node .uiverify/reminders-add-done.mjs
step "notes formatted"   node .uiverify/notes-formatted.mjs
step "notes merge"       node .uiverify/notes-merge.mjs

# The same screens in WebKit, the engine Quince draws in. Chromium above catches most things;
# this catches what only WebKit does — pointer capture, blur on removal, its own layout.
export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1
step "webkit: set up"   .uiverify/webkit-libs.sh
step "webkit: nav"      env ENGINE=webkit node .uiverify/assert.mjs
step "webkit: keyboard" env ENGINE=webkit node .uiverify/keyboard.mjs
step "webkit: notes"    bash -c 'ENGINE=webkit node .uiverify/notes-compose.mjs && ENGINE=webkit node .uiverify/notes-trash.mjs && ENGINE=webkit node .uiverify/notes-pin.mjs && ENGINE=webkit node .uiverify/notes-long.mjs'
step "webkit: drive"    env ENGINE=webkit node .uiverify/drive-upload.mjs
step "webkit: calendar" env ENGINE=webkit node .uiverify/calendar.mjs
step "webkit: sizes"    env ENGINE=webkit node .uiverify/sizes.mjs
step "webkit: narrow"   env ENGINE=webkit node .uiverify/compact.mjs
step "webkit: failing"  env ENGINE=webkit node .uiverify/boundary.mjs
step "webkit: back"     env ENGINE=webkit node .uiverify/resume.mjs
step "webkit: occasions" env ENGINE=webkit node .uiverify/occasions.mjs
step "webkit: drive nav" env ENGINE=webkit node .uiverify/drive-nav.mjs

printf '\n'
if [ "$fail" -eq 0 ]; then printf '\033[32mAll checks passed.\033[0m\n'; else printf '\033[31mSome checks failed.\033[0m\n'; fi
exit "$fail"
