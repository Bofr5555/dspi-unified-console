#!/usr/bin/env bash
# Installs each Linux release file from build-release.sh in a clean container
# of a distribution it targets and starts the app there on the demo device
# for 15 s. Passes when every package installs with only its required
# dependencies (no recommended or weak ones), the app is still running at the
# end and its log shows no QML module, component or image that failed to load.
# -- WP QD42 -- Each carries its licence texts: the .deb Debian's copyright file,
# the .rpm the GPL and the notice in /usr/share/licenses, the AppImage and the
# archive NOTICE.txt and LICENSES/ (Qt's, KWindowSystem's, ICU's, and every
# library in usr/lib listed in BUNDLED.txt). -- WP LIC1 -- and the additional
# terms (TERMS.txt, in the notice), the Rust crates notice, the AppImage runtime's
# Alpine parts (musl, zlib, zstd, mimalloc), and each BUNDLED.txt row's copyright file.
#   test-packages.sh [dist dir, default dist/ or $DSPI_DIST]
# Needs podman and the network (each container installs its packages).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
dist=$(cd "${1:-${DSPI_DIST:-$repo/dist}}" && pwd)
fails=0

# Run in the container: start the app, judge its log
run_app='
set +e
export QT_FORCE_STDERR_LOGGING=1 HOME=/root
timeout 15 "$@" --demo > /root/app.log 2>&1; code=$?
bad=$(grep -E "is not installed|failed to load component|is not a type|Could not load the Qt platform plugin|cannot open shared object|Error decoding|Unsupported image format" /root/app.log)
if [ "$code" = 124 ] && [ -z "$bad" ]; then echo "app ran for 15 s, no load errors"
else echo "app exit $code; the log:"; cat /root/app.log; exit 1; fi'

# -- WP REL013R -- In the container: an installed package's files carry its name,
# DSPi uConsole / dspi-uconsole (and not dspi-console, upstream's package name)
names_check='
d=/usr/share/applications/dspi-uconsole.desktop
grep -qx "Name=DSPi uConsole" $d && grep -qx "Exec=dspi-uconsole" $d && grep -qx "Icon=dspi-uconsole" $d \
    && grep -q "<id>dspi-uconsole</id>" /usr/share/metainfo/dspi-uconsole.metainfo.xml \
    && test -s /usr/share/icons/hicolor/scalable/apps/dspi-uconsole.svg && test -s /usr/lib/udev/rules.d/70-dspi-uconsole.rules \
    && test -x /usr/bin/dspi-uconsole && ! ls -d /usr/bin/dspi-console /usr/share/applications/dspi-console.desktop /usr/share/doc/dspi-console 2>/dev/null \
    || { echo "names: the installed files do not carry the name dspi-uconsole"; exit 1; }
echo "names: dspi-uconsole.desktop (Name=DSPi uConsole), metainfo, icon, udev rule, /usr/bin/dspi-uconsole"'

# -- WP QD42 -- In the container: the licence files under $1 (NOTICE.txt, LICENSES/)
lic_check='
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/LGPL-3.0.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt \
         LICENSES/Qt/LGPL-3.0-only.txt LICENSES/KWindowSystem/LGPL-2.1-only.txt LICENSES/ICU/LICENSE \
         LICENSES/AppImage-runtime/LICENSE LICENSES/AppImage-runtime/libfuse-LGPL2.txt LICENSES/AppImage-runtime/squashfuse-LICENSE \
         LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/AppImage-runtime/musl-COPYRIGHT LICENSES/AppImage-runtime/zlib-LICENSE \
         LICENSES/AppImage-runtime/zstd-LICENSE LICENSES/AppImage-runtime/zstd-COPYING LICENSES/AppImage-runtime/mimalloc-LICENSE; do
    [ -s "$1/$f" ] || { echo "licences: no $f in $1"; exit 1; }
done
grep -q "GNU LESSER GENERAL PUBLIC LICENSE" "$1/LICENSES/LGPL-3.0.txt" || { echo "licences: LGPL-3.0.txt is not the LGPL"; exit 1; }
grep -q "any later version" "$1/NOTICE.txt" || { echo "licences: NOTICE.txt does not say GPL-3.0-or-later"; exit 1; }
# -- WP LIC1 -- the additional terms, verbatim, and the Rust crates
grep -q "section 7 (LICENSES/TERMS.txt" "$1/NOTICE.txt" && grep -q "No trademark rights (7e)" "$1/NOTICE.txt" \
    && grep -q "No trademark rights (7e)" "$1/LICENSES/TERMS.txt" || { echo "licences: NOTICE.txt or TERMS.txt lacks the additional terms"; exit 1; }
grep -q "^Third-party Rust crates linked into" "$1/LICENSES/rust-crates.txt" && grep -q "LICENSES/rust-crates.txt" "$1/NOTICE.txt" \
    || { echo "licences: no Rust crates notice"; exit 1; }
grep -q "^AppImage runtime	.*	.*musl-[0-9.]*\.tar\.gz.*	.*musl" "$1/LICENSES/BUNDLED.txt" || { echo "licences: BUNDLED.txt does not name musl'"'"'s source and licence"; exit 1; }
# -- WP LIC1 -- (review c80 N2) every row that names a copyright file has it
while IFS="	" read -r pkg _ _ _ lic _; do
    case $pkg in "#"*) continue ;; esac
    case $lic in LICENSES/bundled/*/copyright) [ -s "$1/$lic" ] || { echo "licences: $pkg: no $lic"; exit 1; } ;; esac
done <"$1/LICENSES/BUNDLED.txt"
grep -q "^AppImage runtime	.*	type2-runtime-[0-9a-f]*\.tar\.gz.* fuse-[0-9]" "$1/LICENSES/BUNDLED.txt" || { echo "licences: BUNDLED.txt does not name the runtime'"'"'s sources"; exit 1; }
grep -q "^Qt	.*	qtbase-everywhere-src-" "$1/LICENSES/BUNDLED.txt" || { echo "licences: BUNDLED.txt does not name Qt'"'"'s source archives"; exit 1; }
for l in "$2"/*.so*; do
    cut -f6 "$1/LICENSES/BUNDLED.txt" | tr " " "\n" | grep -qx "usr/lib/${l##*/}" || { echo "licences: ${l##*/} is not in BUNDLED.txt"; exit 1; }
done
echo "licences: NOTICE.txt and LICENSES/ in $1, $(ls "$2" | wc -l) libraries listed"'

t() {  # <name> <image> <script>
    echo "== $1"
    if podman run --rm -v "$dist":/dist:ro,z "$2" bash -c "set -e; $3" >"$dist/test-$1.log" 2>&1; then
        tail -n 1 "$dist/test-$1.log"; echo "== $1: OK"
    else
        tail -n 30 "$dist/test-$1.log"; echo "== $1: FAILED (log $dist/test-$1.log)"; fails=$((fails + 1))
    fi
}

t deb-debian-13 docker.io/library/debian:13 "
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends /dist/dspi-uconsole_*_amd64.deb >/dev/null
    grep -q /usr/share/common-licenses/GPL-3 /usr/share/doc/dspi-uconsole/copyright && test -s /usr/share/common-licenses/GPL-3 \
        && grep -q 'any later version' /usr/share/doc/dspi-uconsole/copyright \
        || { echo 'licences: no copyright file pointing at the GPL'; exit 1; }
    grep -q 'No trademark rights (7e)' /usr/share/doc/dspi-uconsole/copyright && grep -q /usr/share/doc/dspi-uconsole/rust-crates.txt /usr/share/doc/dspi-uconsole/copyright \
        && grep -q '^Third-party Rust crates linked into' /usr/share/doc/dspi-uconsole/rust-crates.txt \
        || { echo 'licences: copyright lacks the additional terms or the Rust crates notice'; exit 1; }   # -- WP LIC1 --
    names() { $names_check
    }; names
    run() { $run_app
    }; QT_QPA_PLATFORM=offscreen run dspi-uconsole"
t rpm-fedora-42 registry.fedoraproject.org/fedora:42 "
    dnf -y -q install --setopt=install_weak_deps=False /dist/dspi-uconsole-*.x86_64.rpm >/dev/null
    grep -q 'GNU GENERAL PUBLIC LICENSE' /usr/share/licenses/dspi-uconsole/GPL-3.0.txt && grep -q 'any later version' /usr/share/licenses/dspi-uconsole/NOTICE.txt \
        && [ \"\$(rpm -q --qf '%{LICENSE}' dspi-uconsole)\" = GPL-3.0-or-later ] \
        || { echo 'licences: no GPL text or notice in /usr/share/licenses/dspi-uconsole'; exit 1; }
    grep -q 'No trademark rights (7e)' /usr/share/licenses/dspi-uconsole/NOTICE.txt && grep -q 'No trademark rights (7e)' /usr/share/licenses/dspi-uconsole/TERMS.txt \
        && grep -q '^Third-party Rust crates linked into' /usr/share/licenses/dspi-uconsole/rust-crates.txt \
        || { echo 'licences: no additional terms or Rust crates notice in /usr/share/licenses/dspi-uconsole'; exit 1; }   # -- WP LIC1 --
    names() { $names_check
    }; names
    run() { $run_app
    }; QT_QPA_PLATFORM=offscreen run dspi-uconsole"
# The AppImage on a system with no Qt at all, on X11 (Xvfb): it brings its own.
# Like every AppImage it uses the host's ALSA, GL, fontconfig and a few more
# (linuxdeploy's excludelist), installed here without recommends
# (not ubuntu:22.04: libraries bundled from 24.04 need glibc 2.38, 22.04 has 2.35)
for img in docker.io/library/ubuntu:24.04 docker.io/library/debian:13 registry.fedoraproject.org/fedora:44; do
    name=appimage-${img##*/}; name=${name//:/-}
    t "$name" "$img" "
        if command -v apt-get >/dev/null; then
            apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends xvfb xauth libgl1 libegl1 libopengl0 libfontconfig1 libfreetype6 libharfbuzz0b \
                libx11-6 libx11-xcb1 libxcb1 libsm6 libice6 libwayland-client0 libcom-err2 libgpg-error0 libasound2t64 >/dev/null
        else dnf -y -q install xorg-x11-server-Xvfb mesa-libGL mesa-libEGL fontconfig alsa-lib libglvnd-opengl >/dev/null; fi
        cp /dist/DSPi-uConsole-*-x86_64.AppImage /root/app.AppImage && chmod +x /root/app.AppImage
        cd /root && ./app.AppImage --appimage-extract >/dev/null
        grep -q apprun-hooks squashfs-root/AppRun || { echo 'AppRun does not source apprun-hooks'; exit 1; }
        lic() { $lic_check
        }; lic squashfs-root/usr/share/doc/dspi-uconsole squashfs-root/usr/lib
        Xvfb :5 -nolisten tcp >/dev/null 2>&1 & export DISPLAY=:5; sleep 1
        run() { $run_app
        }; APPIMAGE_EXTRACT_AND_RUN=1 run /root/app.AppImage"
done
# -- WP QD42 -- The archive carries the AppImage's licence files beside it
echo "== archive"
if listing=$(tar -tzf "$dist"/DSPi-uConsole-*-linux-x86_64.tar.gz) && grep -q '^[^/]*/NOTICE.txt$' <<<"$listing" \
    && grep -q '^[^/]*/LICENSES/BUNDLED.txt$' <<<"$listing" && grep -q '^[^/]*/LICENSES/Qt/THIRD-PARTY.txt$' <<<"$listing" \
    && grep -q '^[^/]*/LICENSES/TERMS.txt$' <<<"$listing" && grep -q '^[^/]*/LICENSES/rust-crates.txt$' <<<"$listing"; then
    echo "== archive: OK (NOTICE.txt, LICENSES/)"
else
    echo "== archive: FAILED (no NOTICE.txt or LICENSES/ at its top)"; fails=$((fails + 1))
fi
[ "$fails" = 0 ] && echo "test-packages: PASS" || echo "test-packages: $fails failed"
[ "$fails" = 0 ]
