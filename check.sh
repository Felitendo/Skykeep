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

printf '\n'
if [ "$fail" -eq 0 ]; then printf '\033[32mAll checks passed.\033[0m\n'; else printf '\033[31mSome checks failed.\033[0m\n'; fi
exit "$fail"
