#!/usr/bin/env bash
# Build Quince and open it. Nothing else — no dev server, no watch mode.
set -euo pipefail
cd "$(dirname "$0")"

# Rust installs itself here but only adds it to the PATH of shells it knows how to edit,
# which is not every shell. Adding it unconditionally means this works from any of them.
export PATH="$HOME/.cargo/bin:$PATH"

for tool in cargo npm; do
  command -v "$tool" >/dev/null || {
    echo "Can't find $tool. Is it installed?" >&2
    exit 1
  }
done

echo "Building the interface…"
npm run build --silent >/dev/null

echo "Building the app…"
cargo build -p quince-app --quiet

# One window at a time, so a second run replaces the first rather than stacking.
pkill -f 'quince-ap[p]' 2>/dev/null || true
sleep 1

echo
echo "Opening Quince. Close the window, or press Ctrl-C here, to stop it."
echo
exec ./target/debug/quince-app
