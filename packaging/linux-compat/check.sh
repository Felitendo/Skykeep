#!/bin/bash
# Does Skykeep install and run on this distribution? Run as root in a clean container of it:
#
#   check.sh <package> <distro_check> <test.heic>
#
#   <package>       a .deb, .rpm, .pkg.tar.zst or .AppImage — installed with the system's own
#                   package manager (the .AppImage is only run), with recommends/weak
#                   dependencies off, so only what the package itself asks for is there;
#   <distro_check>  the probe's distro_check example (probe/examples/distro_check.rs);
#   <test.heic>     a HEIC made for the test — never anybody's photo.
#
# Checked, each on its own line, PASS or FAIL:
#   ldd       the program finds every library it links;
#   gst       the GStreamer elements the player and poster reader load at run time are there
#             (SKIP for an AppImage on a system whose gst-inspect is newer than its GStreamer);
#   heic      a HEIC decodes through the system's libheif, the app's own way;
#   keyring   a made-up session goes into a Secret Service (gnome-keyring) and back out
#             through the app's sign-in storage;
#   launch    started headless under Xvfb on its own session bus, the page draws
#             (window_ready) and the program is still running 8 s later — with --background,
#             as the main launcher starts it, and as one app's launcher does (--app notes);
#   relaunch  with no tray to click, launching again hands over to the running Skykeep and
#             its window shows;
#   desktop   the launchers and the metainfo validate, the icon is in hicolor under the name
#             the launchers ask for (packages only).
# Used by .github/workflows/linux-compat.yml, and runs the same by hand under docker.
set -u
pkg=$1 probe=$2 heic=$3
. /etc/os-release
like=" ${ID} ${ID_LIKE:-} "
fails=0
pass() { echo "PASS $1${2:+ — $2}"; }
fail() { echo "FAIL $1${2:+ — $2}"; fails=$((fails + 1)); }

# --- The test's own tools (never the app's dependencies: those come from the package) -------
case "$like" in
  *" debian "* | *" ubuntu "*)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    tools="xvfb xauth dbus gnome-keyring gstreamer1.0-tools desktop-file-utils appstream"
    # An AppImage carries its libraries but not the ones every desktop has (linuxdeploy's
    # excludelist): GL, X11, fonts and text shaping, ALSA, DRM. Without them in this bare
    # container gst-libav (libavcodec links them) does not load and avdec_* looks missing.
    [[ $pkg == *.AppImage ]] && tools+=" libgl1 libegl1 libgles2 libfontconfig1 fonts-dejavu-core \
      libx11-6 libx11-xcb1 libxcb1 libxcb-dri3-0 libdrm2 libharfbuzz0b libfribidi0 libfreetype6 \
      libasound2t64"
    # shellcheck disable=SC2086
    apt-get install -y -qq --no-install-recommends $tools >/dev/null
    [[ $pkg == *.deb ]] && { apt-get install -y -q --no-install-recommends "$pkg" || exit 1; }
    ;;
  *" fedora "*)
    tools="xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon gnome-keyring gstreamer1 \
           desktop-file-utils appstream which"
    # As above; libGLESv2 (libglvnd-gles) too, which WebKit opens and every Fedora desktop has.
    [[ $pkg == *.AppImage ]] && tools+=" mesa-libGL mesa-libEGL libglvnd-gles fontconfig \
      dejavu-sans-fonts libX11 libX11-xcb libxcb libdrm harfbuzz fribidi freetype alsa-lib"
    # shellcheck disable=SC2086
    dnf -y -q install --setopt=install_weak_deps=False $tools
    [[ $pkg == *.rpm ]] && { dnf -y install --setopt=install_weak_deps=False "$pkg" || exit 1; }
    ;;
  *" suse "* | *" opensuse "*)
    # dbus-run-session is in dbus-1-daemon, not dbus-1.
    tools="xvfb-run dbus-1 dbus-1-daemon gnome-keyring gstreamer-utils desktop-file-utils \
      AppStream which"
    [[ $pkg == *.AppImage ]] && tools+=" Mesa-libGL1 Mesa-libEGL1 fontconfig dejavu-fonts"
    # shellcheck disable=SC2086
    zypper -q --non-interactive install --no-recommends $tools
    [[ $pkg == *.rpm ]] && {
      zypper --non-interactive install --no-recommends --allow-unsigned-rpm "$pkg" || exit 1
    }
    ;;
  *" arch "*)
    pacman -Syu --noconfirm --needed -q xorg-server-xvfb xorg-xauth dbus gnome-keyring \
      gstreamer desktop-file-utils appstream which >/dev/null
    [[ $pkg == *.pkg.tar.* ]] && { pacman -U --noconfirm "$pkg" || exit 1; }
    ;;
  *) echo "no installer for $ID"; exit 2 ;;
esac
command -v xvfb-run >/dev/null || { echo "no xvfb-run on $ID"; exit 2; }

# --- Where the program is ---------------------------------------------------------------------
if [[ $pkg == *.AppImage ]]; then
  chmod +x "$pkg"
  # No FUSE in a container: unpacked once and run from there, as --appimage-extract-and-run
  # does, so ldd can look at what it carries.
  cd /tmp && "$pkg" --appimage-extract >/dev/null && cd - >/dev/null || exit 1
  root=/tmp/squashfs-root
  prog="$root/AppRun"
  bin="$root/usr/bin/skykeep"
  libs="$root/usr/lib:$root/usr/lib/x86_64-linux-gnu"
else
  prog=/usr/bin/skykeep bin=/usr/bin/skykeep libs=
fi
[[ -x $bin ]] || { echo "FAIL install — no $bin"; exit 1; }
echo "== $PRETTY_NAME — $(basename "$pkg")"

# --- ldd ------------------------------------------------------------------------------------
missing=$(LD_LIBRARY_PATH=$libs ldd "$bin" | grep "not found")
[[ -z $missing ]] && pass ldd || fail ldd "$missing"

# --- GStreamer elements (the package's, or the AppImage's own) ------------------------------
# playbin3/decodebin/videoconvert/appsink: gst-plugins-base; qtdemux, videoflip: -good;
# avdec_h264 / avdec_h265: gst-libav — an iPhone's .mov/.mp4 and its H.264/HEVC video.
if [[ $pkg == *.AppImage ]]; then
  gstdir=$(find "$root" -type d -name gstreamer-1.0 -path '*lib*' | head -1)
  # As the AppImage's own GStreamer hook sets it up: its plugins, its scanner, a registry of
  # its own — and its libraries (libavcodec for gst-libav) found the way its AppRun finds them.
  export GST_PLUGIN_SYSTEM_PATH_1_0=$gstdir GST_PLUGIN_PATH_1_0= GST_REGISTRY_1_0=/tmp/appimage-registry.bin
  scanner=$(find "$root" -type f -name gst-plugin-scanner | head -1)
  [[ -n $scanner ]] && export GST_PLUGIN_SCANNER_1_0=$scanner
fi
media() {
  local absent= e
  # The AppImage's elements are looked at with this system's gst-inspect-1.0 on the AppImage's
  # own libgstreamer. A system GStreamer newer than the bundled one (Fedora 42's 1.26 against
  # Ubuntu 24.04's 1.24) cannot do that ("undefined symbol"), which says nothing about the
  # AppImage; then the elements are not checked here (Ubuntu and Debian check them).
  if [[ $pkg == *.AppImage ]] && ! LD_LIBRARY_PATH=$libs gst-inspect-1.0 --version >/dev/null 2>&1; then
    echo "SKIP gst — this system's gst-inspect-1.0 cannot load the AppImage's GStreamer"
  else
    for e in playbin3 decodebin videoconvert videoflip appsink qtdemux \
             "avdec_h264|openh264dec" avdec_h265; do
      LD_LIBRARY_PATH=$libs gst-inspect-1.0 "${e%%|*}" >/dev/null 2>&1 || { [[ $e == *"|"* ]] &&
        LD_LIBRARY_PATH=$libs gst-inspect-1.0 "${e##*|}" >/dev/null 2>&1; } || absent+=" $e"
    done
    [[ -z $absent ]] && pass gst || $1 gst "missing:$absent"
  fi
  if out=$(LD_LIBRARY_PATH=$libs "$probe" heic "$heic" 2>&1); then pass heic "$out"
  else $1 heic "$out"; fi
}
# Fedora leaves HEVC out of its own repositories — libheif without its libde265 decoder, FFmpeg
# without H.265 — so an iPhone's HEIC photos and HEVC videos need RPM Fusion there (the note
# under the README's Linux install lines). Without it, those two are noted, not failed; then
# RPM Fusion is added exactly as the README says, and with it both must pass.
# openSUSE is the same: no libde265 and an FFmpeg without HEVC in its own repositories; H.264
# comes from the openh264 repository it has enabled out of the box, HEVC from Packman.
known() { echo "KNOWN $1 — ${2} (no HEVC decoder in $NAME's own repositories)"; }
if [[ $like == *" fedora "* && $pkg != *.AppImage ]]; then
  media known
  rel=$(rpm -E %fedora)
  # RPM Fusion's own instructions (rpmfusion.org/Howto/Multimedia): the full FFmpeg in place of
  # ffmpeg-free, and libheif's HEVC decoder. ffmpeg-libs alone did not give gst-libav H.264.
  dnf -y -q install "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$rel.noarch.rpm" &&
    dnf -y -q swap ffmpeg-free ffmpeg --allowerasing &&
    dnf -y -q install --setopt=install_weak_deps=False libheif-freeworld ||
    fail rpmfusion "could not be added"
  echo "-- with RPM Fusion:"
  # GStreamer's registry remembers gst-libav with ffmpeg-free's decoders; read it afresh.
  rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/gstreamer-1.0"
  media fail
elif [[ $like == *" suse "* && $pkg != *.AppImage ]]; then
  media known
  # Packman's FFmpeg and libheif, as Packman's own instructions put them in place.
  zypper -q --non-interactive ar -cfp 90 \
      "https://ftp.gwdg.de/pub/linux/misc/packman/suse/openSUSE_Tumbleweed/" packman &&
    zypper -q --non-interactive --gpg-auto-import-keys refresh packman &&
    zypper -q --non-interactive install --no-recommends --allow-vendor-change --from packman \
      libavcodec62 libheif1 libde265-0 libheif-HEIF ||
    fail packman "could not be added"
  echo "-- with Packman:"
  rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/gstreamer-1.0"
  media fail
else
  media fail
fi
unset GST_PLUGIN_SYSTEM_PATH_1_0 GST_PLUGIN_PATH_1_0

# --- Keyring and launch, in one session bus with a keyring daemon on it ---------------------
export HOME=$(mktemp -d) XDG_RUNTIME_DIR=$(mktemp -d) RUST_BACKTRACE=1 SKYKEEP_TIMING=1
chmod 700 "$XDG_RUNTIME_DIR"
cat >/tmp/session.sh <<'SH'
#!/bin/bash
# Inside dbus-run-session: a gnome-keyring with an unlocked login keyring (a throwaway
# password), then the keyring check and the two launches.
probe=$1 prog=$2 libs=$3
echo -n throwaway | gnome-keyring-daemon --unlock --components=secrets >/dev/null 2>&1
if out=$(LD_LIBRARY_PATH=$libs "$probe" keyring 2>&1); then echo "PASS keyring — $out"
else echo "FAIL keyring — $out"; fi
for how in --background launcher "--app notes"; do
  log=$(mktemp)
  # shellcheck disable=SC2206
  args=(); [[ $how == --* ]] && args=($how)
  window=main; [[ $how == "--app notes" ]] && window=app-notes
  timeout --kill-after=5 8 xvfb-run -a -s "-screen 0 1280x800x24" "$prog" "${args[@]}" >"$log" 2>&1
  rc=$?
  drawn=$(grep -m1 -o "+[0-9.]*ms window_ready $window" "$log")
  if [[ $rc -eq 124 && -n $drawn ]] && ! grep -q 'panicked at' "$log"; then
    echo "PASS launch ($how) — page drawn at ${drawn%% *}, still running at 8 s"
  else
    echo "FAIL launch ($how) — exit $rc, drawn: ${drawn:-never}"; grep -v skykeep-timing "$log" | tail -20
  fi
done
# Reachable with no tray to click (stock GNOME, and this container): launched again, as from
# the launcher, the second process hands over to the running one and leaves; that one shows
# its window.
log=$(mktemp)
timeout --kill-after=5 16 xvfb-run -a -s "-screen 0 1280x800x24" bash -c \
  '"$0" --background & sleep 6; timeout 6 "$0"; echo "second launch exited $?"; sleep 2' \
  "$prog" >"$log" 2>&1
if grep -q 'second launch exited 0' "$log" && grep -q 'handed a launch' "$log" &&
   grep -q 'main shown' "$log"; then
  echo "PASS relaunch — a second launch reopened the running one's window"
else
  echo "FAIL relaunch"; grep -v skykeep-timing "$log" | tail -20
fi
SH
chmod +x /tmp/session.sh
results=$(dbus-run-session -- /tmp/session.sh "$probe" "$prog" "$libs")
echo "$results"
fails=$((fails + $(grep -c '^FAIL' <<<"$results")))

# --- Desktop integration (packages) ---------------------------------------------------------
if [[ $pkg != *.AppImage ]]; then
  apps=/usr/share/applications
  problems=
  for f in "$apps"/*Skykeep*.desktop "$apps"/skykeep*.desktop; do
    [[ -e $f ]] || continue
    m=$(desktop-file-validate "$f" 2>&1 | grep -v 'hint:')
    [[ -z $m ]] || problems+=" $(basename "$f"): $m"
    icon=$(sed -n 's/^Icon=//p' "$f" | head -1)
    sizes=$(find /usr/share/icons/hicolor -name "$icon.png" -o -name "$icon.svg" | wc -l)
    [[ $sizes -gt 0 ]] || problems+=" $(basename "$f"): no hicolor icon '$icon'"
    # The icon theme spec's one requirement of an app: a 48x48 in hicolor (or a scalable one).
    [[ -e /usr/share/icons/hicolor/48x48/apps/$icon.png ||
       -e /usr/share/icons/hicolor/scalable/apps/$icon.svg ]] ||
      problems+=" $(basename "$f"): no 48x48 hicolor icon '$icon'"
  done
  m=$(appstreamcli validate --no-net --explain /usr/share/metainfo/*Skykeep*.xml 2>&1) ||
    problems+=" metainfo: $(grep -E '^[EW]:' <<<"$m" | tr '\n' ' ')"
  launchable=$(sed -n 's:.*<launchable type="desktop-id">\(.*\)</launchable>.*:\1:p' /usr/share/metainfo/*Skykeep*.xml)
  [[ -e $apps/$launchable ]] || problems+=" metainfo launchable $launchable not installed"
  # GNOME Shell drops a search provider whose DesktopId is not an installed launcher.
  for ini in /usr/share/gnome-shell/search-providers/*Skykeep*.ini; do
    id=$(sed -n 's/^DesktopId=//p' "$ini")
    [[ -e $apps/$id ]] || problems+=" search provider DesktopId $id not installed"
  done
  icons=$(find /usr/share/icons/hicolor -path '*apps*' \( -iname '*skykeep*' \) | sed 's:/usr/share/icons/hicolor/::' | tr '\n' ' ')
  [[ -z $problems ]] && pass desktop "icons: $icons" || fail desktop "$problems"
fi

echo "== $fails failed"
exit $((fails > 0))
