#!/usr/bin/env bash
# -- WP APPIMG -- The AppImage's release test: the AppImage from build-release.sh
# in clean containers of the systems it is for, none with Qt installed:
#
#   test-appimage.sh [--build] [dist dir] [image...]
#
# --build first makes the AppImage and the archive as the release does
# (build-release.sh --appimage-only, from a clean commit) and tests that
# folder. Default images: Ubuntu 24.04 (the oldest glibc it supports) and
# 25.04, Debian 13, Fedora 44. In each, with only the system libraries the
# AppImage leaves to the system (make-appimage.sh's EXCLUDED):
#   - no libQt6 on the system; `--version` prints "uConsole for DSPi <version>"
#     and `--help` exits 0, both with no display (-- WP QTCLI --)
#   - AppRun sources its hooks, the update feed is built in, the licence files
#     (test-lib.sh lic_check)
#   - ldd over every ELF file in the extracted AppImage, as is (no
#     LD_LIBRARY_PATH): nothing not found
#   - the app runs 15 s on the demo device on X11 (Xvfb) and offscreen
#     (QT_QPA_PLATFORM=offscreen) with no load error in its log
#   - offscreen with appimage-probe.cpp preloaded: Help > About shows
#     "Version <version>", the licence section (GPL-3.0) and the notice files
#     from the AppImage's usr/share/doc/uconsole-for-dspi; every Qt library,
#     plugin and QML module mapped, Qt's plugin and QML paths and the engine's
#     import path are the AppImage's own. The About window is saved as
#     about-<image>.png beside the logs. (The app loads no translation
#     catalogue: it is English only, its .ts is the source strings' record.)
# And on the host, once: the archive's udev rule is the repository's (-- WP UDEV1 --
# xtask/70-dspi.rules, the one rule of every component), and the udev
# instructions (packaging/README.md, the archive's README.txt and install.sh)
# name that rule, where it goes and how to reload udev the same way.
# Logs: test-appimage-<image>.log in the dist dir. Needs podman and the network.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
if [ "${1:-}" = --build ]; then
    shift
    "$here/../build-release.sh" --appimage-only || exit 1
    v=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$here/../../qt/CMakeLists.txt")
    set -- "$B/work/release-$v-appimage" "$@"
fi
dist=$(cd "${1:?usage: test-appimage.sh [--build] [dist dir] [image...]}" && pwd); shift
images=("$@")
[ ${#images[@]} -gt 0 ] || images=(docker.io/library/ubuntu:24.04 docker.io/library/ubuntu:25.04
                                   docker.io/library/debian:13 registry.fedoraproject.org/fedora:44)
appimage=$(ls "$dist"/uConsole-for-DSPi-*-x86_64.AppImage 2>/dev/null)
[ "$(printf '%s\n' "$appimage" | grep -c .)" = 1 ] || { echo "test-appimage: not one AppImage in $dist" >&2; exit 1; }
version=${appimage##*/uConsole-for-DSPi-}; version=${version%-x86_64.AppImage}
. "$here/test-lib.sh"
work=$B/work/appimage-test
rm -rf "$work"; mkdir -p "$work/out"

# ---- On the host: the udev rule and the instructions that name it
udev_check() {
    local x rule=70-dspi.rules
    x=$work/archive; mkdir -p "$x"
    tar -xzf "$dist/uConsole-for-DSPi-$version-linux-x86_64.tar.gz" -C "$x" || { echo "udev: no archive"; return 1; }
    x=$x/uConsole-for-DSPi-$version-linux-x86_64
    cmp -s "$x/$rule" "$repo/xtask/$rule" || { echo "udev: the archive's $rule is not the repository's"; return 1; }
    grep -q 'ATTR{idVendor}=="2e8b", TAG+="uaccess", MODE="0666"' "$x/$rule" || { echo "udev: $rule does not open vendor 2e8b (uaccess, 0666)"; return 1; }
    grep -qF "rule=/etc/udev/rules.d/$rule" "$x/install.sh" && grep -qF 'udevadm control --reload-rules' "$x/install.sh" \
        || { echo "udev: install.sh does not install /etc/udev/rules.d/$rule and reload the rules"; return 1; }
    grep -qF './install.sh --udev' "$x/README.txt" && grep -qF "$rule" "$x/README.txt" \
        || { echo "udev: README.txt does not name ./install.sh --udev and $rule"; return 1; }
    # The packaging README's rule section: the same rule, the same folder, the same reload
    # (-- WP UDEV1 -- the top README.md links there for installing)
    local doc
    doc=$(sed -n '/^## Linux: the udev rule/,/^## /p' "$here/../README.md")
    for s in "install.sh --udev" "$rule" "/etc/udev/rules.d/" "udevadm control --reload-rules"; do
        case $doc in *"$s"*) ;; *) echo "udev: packaging/README.md's udev section does not say \"$s\""; return 1 ;; esac
    done
    echo "udev: $rule in the archive is the repository's; install.sh, README.txt and packaging/README.md name it and /etc/udev/rules.d/"
}
echo "== udev (host)"
if udev_check; then echo "== udev: OK"; else echo "== udev: FAILED"; fails=$((fails + 1)); fi

# ---- The probe, built against the release's Qt in the release's image
. "$here/../image.sh"
image uconsole-for-dspi-ubuntu "$here/Containerfile.ubuntu" "$here" >/dev/null || exit 1
mods="Core Gui Qml Quick"
podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" --security-opt label=disable \
    -v "$here":/p:ro -v "$work":/w "$tag" bash -c "g++ -std=c++17 -O1 -fPIC -shared /p/appimage-probe.cpp -o /w/appimage-probe.so \
        -I/opt/qt/include $(for m in $mods; do printf -- '-I/opt/qt/include/Qt%s ' "$m"; done) \
        -L/opt/qt/lib $(for m in $mods; do printf -- '-lQt6%s ' "$m"; done) -Wl,-rpath,/root/squashfs-root/usr/lib" \
    || { echo "test-appimage: the probe did not build" >&2; exit 1; }

# ---- In each container
setup='
if command -v apt-get >/dev/null; then
    # A release past its end of life (25.04) is on old-releases.ubuntu.com
    apt-get update -qq 2>/dev/null || { sed -i -E "s#http://(archive|security)\.ubuntu\.com#http://old-releases.ubuntu.com#" \
        /etc/apt/sources.list.d/*.sources /etc/apt/sources.list 2>/dev/null; apt-get update -qq; }
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends xvfb xauth libgl1 libegl1 libopengl0 libfontconfig1 libfreetype6 libharfbuzz0b \
        libx11-6 libx11-xcb1 libxcb1 libsm6 libice6 libwayland-client0 libcom-err2 libgpg-error0 libasound2t64 libdbus-1-3 \
        libpulse0 >/dev/null
else dnf -y -q install xorg-x11-server-Xvfb mesa-libGL mesa-libEGL fontconfig alsa-lib libglvnd-opengl dbus-libs pulseaudio-libs >/dev/null; fi
! ldconfig -p | grep -q libQt6 || { echo "qt: this system has Qt installed"; exit 1; }'

check='
# -- WP QTCLI -- with no display (no DISPLAY, WAYLAND_DISPLAY or
# QT_QPA_PLATFORM): the app reads --version and --help before its GUI
v=$(env -u DISPLAY -u WAYLAND_DISPLAY -u QT_QPA_PLATFORM APPIMAGE_EXTRACT_AND_RUN=1 /root/app.AppImage --version 2>/dev/null); c=$?
[ "$c:$v" = "0:uConsole for DSPi $VERSION" ] || { echo "version: --version exited $c, printed \"$v\""; exit 1; }
echo "version: $v (no display)"
h=$(env -u DISPLAY -u WAYLAND_DISPLAY -u QT_QPA_PLATFORM APPIMAGE_EXTRACT_AND_RUN=1 /root/app.AppImage --help 2>&1); c=$?
case "$c:$h" in 0:*--version*--demo*) echo "help: --help exits 0 (no display)" ;;
    *) echo "help: --help exited $c without a display: $h"; exit 1 ;; esac
cd /root && ./app.AppImage --appimage-extract >/dev/null
grep -q apprun-hooks squashfs-root/AppRun || { echo "AppRun does not source apprun-hooks"; exit 1; }
grep -aqP "$FEED" squashfs-root/usr/bin/uconsole-for-dspi || { echo "feed: the AppImage has no update feed"; exit 1; }
# Every ELF file, as the loader sees it with nothing set
n=0; missing=
while IFS= read -r -d "" f; do
    [ "$(head -c 4 "$f" | od -An -c | tr -d " ")" = "177ELF" ] || continue
    n=$((n + 1)); out=$(ldd "$f" 2>&1 | grep "not found") && missing="$missing$f: $out
"
done < <(find squashfs-root -type f -print0)
[ -z "$missing" ] || { echo "ldd: libraries not found:"; echo "$missing"; exit 1; }
echo "ldd: $n ELF files, every library found"'

probe='
set +e
QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 LD_PRELOAD=/w/appimage-probe.so PROBE_PNG=/w/out/about-$NAME.png \
    timeout 60 /root/squashfs-root/AppRun --demo >/root/probe.log 2>&1; code=$?
p() { sed -n "s/^probe: $1=//p" /root/probe.log | head -n 1; }
fail() { echo "probe: $*; the log:"; cat /root/probe.log; exit 1; }
[ "$code" = 0 ] || fail "exit $code"
root=$(readlink -f /root/squashfs-root)
case $(p version) in "Version $VERSION "*) ;; *) fail "About shows \"$(p version)\", not Version $VERSION" ;; esac
case $(p About.licenceGpl) in *"General Public License"*) ;; *) fail "About has no GPL row" ;; esac
[ -n "$(p About.licenceStatement)" ] || fail "About has no licence statement"
[ "$(readlink -f "$(p noticesDir)")" = "$root/usr/share/doc/uconsole-for-dspi" ] || fail "notices from \"$(p noticesDir)\""
[ "$(p notices)" -ge 5 ] 2>/dev/null || fail "About lists $(p notices) notice files"
for k in plugins qml libraries; do
    case $(readlink -f "$(p qtpath.$k)") in "$root"/usr/*) ;; *) fail "Qt $k path $(p qtpath.$k) is not the AppImage'"'"'s" ;; esac
done
case :$(p importpath): in *":$root/usr/qml:"*) ;; *) fail "the QML import path $(p importpath) lacks the AppImage'"'"'s" ;; esac
libs=$(sed -n "s/^probe: lib=//p" /root/probe.log)
foreign=$(printf "%s\n" "$libs" | grep -v "^$root/" | grep -v "^/w/appimage-probe.so$")
[ -z "$foreign" ] || fail "Qt files from outside the AppImage: $foreign"
for want in usr/lib/libQt6Quick.so usr/plugins/platforms/libqoffscreen.so usr/qml/QtQuick/Controls; do
    printf "%s\n" "$libs" | grep -q "^$root/$want" || fail "nothing loaded from $want"
done
echo "probe: $(p version); $(p About.licenceGpl); $(p notices) notices; $(printf "%s\n" "$libs" | wc -l) Qt files, all the AppImage'"'"'s"'

for img in "${images[@]}"; do
    name=appimage-${img##*/}; name=${name//:/-}
    t "$name" "$img" "
        $setup
        cp /dist/${appimage##*/} /root/app.AppImage && chmod +x /root/app.AppImage
        $check
        lic() { $lic_check
        }; lic squashfs-root/usr/share/doc/uconsole-for-dspi squashfs-root/usr/lib
        run() { $run_app
        }
        Xvfb :5 -nolisten tcp >/dev/null 2>&1 & export DISPLAY=:5; sleep 1
        APPIMAGE_EXTRACT_AND_RUN=1 run /root/app.AppImage
        unset DISPLAY; QT_QPA_PLATFORM=offscreen APPIMAGE_EXTRACT_AND_RUN=1 run /root/app.AppImage
        $probe" -v "$work":/w -e VERSION="$version" -e FEED="$feed" -e NAME="${name#appimage-}"
    [ ! -s "$work/out/about-${name#appimage-}.png" ] || cp "$work/out/about-${name#appimage-}.png" "$dist/"
done
[ "$fails" = 0 ] && echo "test-appimage: PASS" || echo "test-appimage: $fails failed"
[ "$fails" = 0 ]
