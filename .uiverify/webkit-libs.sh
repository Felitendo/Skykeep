#!/usr/bin/env bash
# Give Playwright's WebKit the libraries it expects, without root.
#
# Quince draws in WebKitGTK, so the browser checks have to run in WebKit, not only Chromium:
# the two differ in exactly the places bugs hide (pointer capture, blur on removal, fonts).
# Playwright builds its WebKit on Ubuntu and links a few libraries by Ubuntu's names and
# versions; this fetches those into ~/.local/share/quince-webkit-libs and links them into
# Playwright's own library folder, which its launcher puts first. Safe to run repeatedly.
set -euo pipefail
L="$HOME/.local/share/quince-webkit-libs"
PW="$(ls -d "$HOME"/.cache/ms-playwright/webkit-*/ 2>/dev/null | sort | tail -1)"
[ -n "$PW" ] || { echo "Playwright WebKit is not installed (npx playwright install webkit)"; exit 1; }
mkdir -p "$L"/{pkgs,deb,icu74,icu75,arch}

fetch() { [ -f "$L/pkgs/$(basename "$1")" ] || curl -sfL -o "$L/pkgs/$(basename "$1")" "$1"; }
fetch https://archive.archlinux.org/packages/i/icu/icu-74.2-1-x86_64.pkg.tar.zst
fetch https://archive.archlinux.org/packages/i/icu/icu-75.1-2-x86_64.pkg.tar.zst
fetch https://archive.archlinux.org/packages/l/libxml2/libxml2-2.13.5-2-x86_64.pkg.tar.zst
fetch https://geo.mirror.pkgbuild.com/extra/os/x86_64/libbacktrace-1.0.git1-1-x86_64.pkg.tar.zst
fetch http://archive.ubuntu.com/ubuntu/pool/universe/f/flite/libflite1_2.2-6build3_amd64.deb

tar --zstd -xf "$L/pkgs/icu-74.2-1-x86_64.pkg.tar.zst" -C "$L/icu74"
tar --zstd -xf "$L/pkgs/icu-75.1-2-x86_64.pkg.tar.zst" -C "$L/icu75"
tar --zstd -xf "$L/pkgs/libxml2-2.13.5-2-x86_64.pkg.tar.zst" -C "$L/arch"
tar --zstd -xf "$L/pkgs/libbacktrace-1.0.git1-1-x86_64.pkg.tar.zst" -C "$L/arch"
( cd "$L/deb" && ar x "$L/pkgs/libflite1_2.2-6build3_amd64.deb" && tar -xf data.tar.* )

for port in minibrowser-wpe minibrowser-gtk; do
  dest="$PW/$port/sys/lib"
  mkdir -p "$dest"
  for f in "$L"/icu74/usr/lib/libicu*.so.74 "$L"/icu75/usr/lib/libicu*.so.75 \
           "$L"/arch/usr/lib/libxml2.so.2 "$L"/arch/usr/lib/libbacktrace.so.0 \
           "$L"/deb/usr/lib/x86_64-linux-gnu/libflite*.so.1; do
    [ -e "$f" ] && ln -sf "$f" "$dest/$(basename "$f")"
  done
done
echo "WebKit libraries ready in $L"
