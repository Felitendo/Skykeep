#!/bin/bash
# store-install.sh <snap install arguments…>: a snap installed from the Snap Store on a CI runner,
# made to finish or fail instead of hanging. STORE_MINUTES gives each try its limit (5 by
# default): sized to the snap, as the GNOME SDK is far larger than snapcraft, and on a slow day
# a limit made for a small snap cut every try of the large one off mid-download (10 Nov).
#
# 10 Nov 2026: the Snap Store stopped answering runners mid-install five times in a day (lxd,
# build snaps, snapcraft itself). A hung install never fails by itself, and when a time limit ends
# it, its change stays "in progress" in snapd, so every later install is refused at once ("has
# 'install-snap' change in progress") and a retry never really retries. So each try has its own
# limit, and a try that runs out aborts every unfinished snapd change and waits for it to clear
# before the next try.
set -u
limit=$(( ${STORE_MINUTES:-5} * 60 ))
for try in 1 2 3; do
  if sudo timeout "$limit" snap install "$@"; then
    exit 0
  fi
  echo "::warning::snap install $* did not finish (try $try of 3): clearing it and trying again"
  for id in $(snap changes 2>/dev/null | awk 'NR > 1 && ($2 == "Do" || $2 == "Doing") { print $1 }'); do
    sudo snap abort "$id" || true
  done
  for _ in $(seq 60); do
    snap changes 2>/dev/null | awk 'NR > 1 { print $2 }' | grep -qE '^(Do|Doing|Undo|Undoing)$' || break
    sleep 2
  done
  sleep $((try * 15))
done
echo "::error::snap install $* failed three times: the Snap Store isn't answering this runner"
exit 1
