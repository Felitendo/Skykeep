#!/usr/bin/env python3
"""The Flatpak's offline sources hold every package the lockfiles name, checksum for checksum.

The Flatpak is built with no network: Cargo and npm read only what cargo-sources.json and
node-sources.json put in their caches. A dependency added to Cargo.lock or package-lock.json
without the sources regenerated (see README.md here) builds everywhere else and fails only in
the Flatpak, at release: v0.1.18's tag was spent that way (uniffi, for the Android bridge).
Run by the gate and CI on every change, so it fails where the lockfile changed instead.

Usage: packaging/flatpak/sources-current.py  (from the repository root; exits 1 on a gap)
"""

import base64
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def cargo_gaps() -> list[str]:
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    have = {}
    for entry in json.loads((HERE / "cargo-sources.json").read_text()):
        if entry.get("type") == "archive" and entry.get("dest", "").startswith("cargo/vendor/"):
            have[entry["dest"].removeprefix("cargo/vendor/")] = entry.get("sha256")
    gaps = []
    for package in lock["package"]:
        source = package.get("source")
        if source is None:
            continue  # the workspace's own crates
        name = f"{package['name']}-{package['version']}"
        if source != CRATES_IO:
            gaps.append(f"Cargo.lock: {name} comes from {source}, which this check does not know")
        elif name not in have:
            gaps.append(f"Cargo.lock: {name} is not in cargo-sources.json")
        elif have[name] != package.get("checksum"):
            gaps.append(f"Cargo.lock: {name}'s checksum differs from cargo-sources.json's")
    return gaps


def npm_gaps() -> list[str]:
    lock = json.loads((ROOT / "package-lock.json").read_text())
    have = {
        entry["url"]: entry.get("sha512")
        for entry in json.loads((HERE / "node-sources.json").read_text())
        if entry.get("type") == "file" and "url" in entry
    }
    gaps = []
    for path, package in lock.get("packages", {}).items():
        url = package.get("resolved")
        if not path or not url:
            continue  # the project itself, or a linked workspace package
        integrity = package.get("integrity", "")
        if url not in have:
            gaps.append(f"package-lock.json: {url} is not in node-sources.json")
        elif integrity.startswith("sha512-"):
            if base64.b64decode(integrity.removeprefix("sha512-")).hex() != have[url]:
                gaps.append(f"package-lock.json: {url}'s checksum differs from node-sources.json's")
    return gaps


def main() -> int:
    gaps = cargo_gaps() + npm_gaps()
    for gap in gaps:
        print(gap, file=sys.stderr)
    if gaps:
        print(
            "The Flatpak builds offline from packaging/flatpak/*-sources.json: regenerate them "
            "as packaging/flatpak/README.md says.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
