#!/usr/bin/env python3
"""Privacy audit before publishing Quince: does any of his real iCloud data appear in the git
history? Kept in docs/ (never committed).

Reads his library (read-only) and every version of every committed file, and reports matches
in truncated form only. Nothing personal is printed in full or written anywhere.

    python3 docs/privacy-audit.py

Also checks the session's cookie values and the account's dsid, the credentials that matter
most. Run it again after any history rewrite (git filter-repo) to prove it is clean.
"""
import json
import os
import re
import sqlite3
import subprocess
from collections import Counter

# The app's data folder: its identifier since the rename, or the old one on an install the
# rename has not moved yet (src-tauri/src/migrate.rs).
DATA = next(
    (d for d in map(os.path.expanduser, ["~/.local/share/io.github.yashwanthkamireddi.Skykeep",
                                          "~/.local/share/io.github.yashwanthkamireddi.Skykeep"])
     if os.path.exists(os.path.join(d, "library.db"))),
    os.path.expanduser("~/.local/share/io.github.yashwanthkamireddi.Skykeep"),
)
history = subprocess.run(["git", "log", "--all", "-p", "--no-color"], capture_output=True, text=True, errors="ignore").stdout
digits = re.sub(r"\D", "", history)
db = sqlite3.connect(f"file:{DATA}/library.db?mode=ro", uri=True)
found = Counter()
examples = {}


def hit(kind, value):
    found[kind] += 1
    examples.setdefault(kind, set()).add(value[:3] + "…")


for (v,) in db.execute("select value from contact_methods where kind='phone'"):
    d = re.sub(r"\D", "", v)[-10:]
    if len(d) == 10 and d in digits:
        hit("phone number", d)
for (v,) in db.execute("select value from contact_methods where kind in ('email','address')"):
    for part in re.split(r"[,\n]", v or ""):
        p = part.strip()
        if len(p) >= 8 and p in history:
            hit("email/address part", p)
for (f, l) in db.execute("select coalesce(first_name,''), coalesce(last_name,'') from contacts"):
    n = f"{f} {l}".strip()
    if len(n) >= 6 and n in history:
        hit("contact name", n)
GENERIC = {"Reminders", "Calendar", "Home", "Work", "Daily", "Study", "Notes", "New Reminder", "New Folder"}
for label, q in [
    ("reminder title", "select title from reminders"),
    ("note title", "select title from notes"),
    ("event title", "select title from events"),
    ("drive file name", "select name from drive_items where kind='FILE'"),
    ("photo file name", "select filename from photos"),
    ("album name", "select name from photo_albums"),
]:
    for (v,) in db.execute(q):
        v = (v or "").strip()
        if len(v) >= 8 and v not in GENERIC and v in history:
            hit(label, v)
try:
    # The jar is encrypted at rest now; a plaintext copy (taken before the first encrypted run)
    # can be named with COOKIES=... to keep checking that no cookie value reached git.
    cookies = json.dumps(json.load(open(os.environ.get("COOKIES", f"{DATA}/cookies.json"))))
    for v in set(re.findall(r'"([A-Za-z0-9+/=_%.-]{24,})"', cookies)):
        if v in history:
            hit("session cookie value", v)
    for d in set(re.findall(r"dsid[^0-9]{0,6}(\d{6,})", cookies)):
        if d in history:
            hit("account dsid", d)
except (OSError, ValueError):
    print("(cookies not checked: the jar is encrypted; set COOKIES to a plaintext copy)")
print("clean" if not found else "FOUND:")
for kind, n in found.items():
    print(f"  {kind}: {n}  e.g. {sorted(examples[kind])[:5]}")
