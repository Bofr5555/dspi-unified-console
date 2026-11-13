#!/usr/bin/env bash
# Installs each Linux release file from build-release.sh in a clean container
# of a distribution it targets and starts the app there on the demo device
# for 15 s. Passes when every package installs with only its required
# dependencies (no recommended or weak ones), the app is still running at the
# end and its log shows no QML module, component or image that failed to load.
# -- WP QD42 -- Each carries its licence texts: the .deb Debian's copyright file,
# the .rpm the GPL and the notice in /usr/share/licenses, the AppImage and the
# archive NOTICE.txt and LICENSES/ (Qt's, ICU's, and every library in usr/lib
# listed in BUNDLED.txt). -- WP LIC1 -- and the additional terms (TERMS.txt, in
# the notice), the Rust crates notice, the AppImage runtime's Alpine parts
# (musl, zlib, zstd, mimalloc), and each BUNDLED.txt row's copyright file.
# -- WP QP20 -- The name people see is "uConsole for DSPi"; every package's
# notice labels it GPL-3.0-only (the .rpm's License field too) and carries
# Lucide's licence; the udev rule is installed; the archive's install.sh
# installs and removes a per-user copy; only the AppImage (and so the archive)
# has the update feed, never the .deb or the .rpm.
#   test-packages.sh [dist dir, default dist/ or $DSPI_DIST]
# Needs podman and the network (each container installs its packages).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
dist=$(cd "${1:-${DSPI_DIST:-$repo/dist}}" && pwd)
# The update feed as the app holds it (QStringLiteral: UTF-16), as a grep -P pattern
feed=$(printf '%s' api.github.com/repos/Bofr5555/dspi-unified-console/releases | sed 's/./&\\x00/g')
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
# uConsole for DSPi / uconsole-for-dspi (and not dspi-console, upstream's package name,
# nor dspi-uconsole, this one's name before -- WP RN1 --)
names_check='
d=/usr/share/applications/uconsole-for-dspi.desktop
grep -qx "Name=uConsole for DSPi" $d && grep -qx "Exec=uconsole-for-dspi" $d && grep -qx "Icon=uconsole-for-dspi" $d \
    && grep -q "<id>uconsole-for-dspi</id>" /usr/share/metainfo/uconsole-for-dspi.metainfo.xml \
    && grep -q "<project_license>GPL-3.0-only</project_license>" /usr/share/metainfo/uconsole-for-dspi.metainfo.xml \
    && test -s /usr/share/icons/hicolor/scalable/apps/uconsole-for-dspi.svg \
    && grep -q "ATTR{idVendor}==\"2e8b\", TAG+=\"uaccess\"" /usr/lib/udev/rules.d/70-uconsole-for-dspi.rules \
    && test -x /usr/bin/uconsole-for-dspi && ! ls -d /usr/bin/dspi-console /usr/share/applications/dspi-console.desktop /usr/share/doc/dspi-console /usr/bin/dspi-uconsole 2>/dev/null \
    || { echo "names: the installed files do not carry the name uconsole-for-dspi"; exit 1; }
! grep -aqP '"$feed"' /usr/bin/uconsole-for-dspi || { echo "feed: a distribution package has the update feed"; exit 1; }
echo "names: uconsole-for-dspi.desktop (Name=uConsole for DSPi), metainfo, icon, udev rule, /usr/bin/uconsole-for-dspi; no update feed"'

# -- WP QP20 -- In the container: the notice $1 says GPL-3.0-only, names the
# additional terms, Lucide and the Rust crates; $2 is the folder of LICENSES/
notice_check='
flat=$(tr "\n" " " <"$1" | tr -s " ")
for s in "Licence: GPL-3.0-only" "uConsole for DSPi" "any later version" "section 7 (LICENSES/TERMS.txt" \
         "No trademark rights (7e)" "LICENSES/Lucide/LICENSE" "LICENSES/rust-crates.txt" "-desktop-source.tar.gz"; do
    case $flat in *"$s"*) ;; *) echo "notice: $1 does not say \"$s\""; exit 1 ;; esac
done
grep -q "^ISC License" "$2/LICENSES/Lucide/LICENSE" && grep -q "No trademark rights (7e)" "$2/LICENSES/TERMS.txt" \
    && grep -q "^Third-party Rust crates linked into" "$2/LICENSES/rust-crates.txt" \
    && test -s "$2/LICENSES/firmware-THIRD-PARTY-NOTICES.txt" && test -s "$2/LICENSES/AutoEq-MIT.txt" \
    || { echo "notice: $2/LICENSES lacks Lucide'"'"'s licence, the terms, the Rust crates or the firmware'"'"'s notices"; exit 1; }
echo "notice: GPL-3.0-only, the terms, Lucide, the Rust crates"'

# -- WP QD42 -- In the container: the licence files under $1 (NOTICE.txt, LICENSES/)
lic_check='
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/LGPL-3.0.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt \
         LICENSES/Qt/LGPL-3.0-only.txt LICENSES/ICU/LICENSE LICENSES/Lucide/LICENSE \
         LICENSES/AppImage-runtime/LICENSE LICENSES/AppImage-runtime/libfuse-LGPL2.txt LICENSES/AppImage-runtime/squashfuse-LICENSE \
         LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/AppImage-runtime/musl-COPYRIGHT LICENSES/AppImage-runtime/zlib-LICENSE \
         LICENSES/AppImage-runtime/zstd-LICENSE LICENSES/AppImage-runtime/zstd-COPYING LICENSES/AppImage-runtime/mimalloc-LICENSE; do
    [ -s "$1/$f" ] || { echo "licences: no $f in $1"; exit 1; }
done
grep -q "GNU LESSER GENERAL PUBLIC LICENSE" "$1/LICENSES/LGPL-3.0.txt" || { echo "licences: LGPL-3.0.txt is not the LGPL"; exit 1; }
notice() { '"$notice_check"'
}; notice "$1/NOTICE.txt" "$1"
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
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends /dist/uconsole-for-dspi_*_amd64.deb >/dev/null
    doc=/usr/share/doc/uconsole-for-dspi
    grep -q /usr/share/common-licenses/GPL-3 \$doc/copyright && test -s /usr/share/common-licenses/GPL-3 \
        && cmp -s \$doc/copyright \$doc/NOTICE.txt \
        || { echo 'licences: no copyright file pointing at the GPL'; exit 1; }
    notice() { $notice_check
    }; notice \$doc/copyright \$doc
    names() { $names_check
    }; names
    run() { $run_app
    }; QT_QPA_PLATFORM=offscreen run uconsole-for-dspi"
# Each Fedora release its own .rpm (Containerfile.fedora)
for fc in 43 44; do
    t "rpm-fedora-$fc" "registry.fedoraproject.org/fedora:$fc" "
        dnf -y -q install --setopt=install_weak_deps=False /dist/uconsole-for-dspi-*.fc$fc.x86_64.rpm >/dev/null
        lic=/usr/share/licenses/uconsole-for-dspi
        grep -q 'GNU GENERAL PUBLIC LICENSE' \$lic/LICENSES/GPL-3.0.txt && [ \"\$(rpm -q --qf '%{LICENSE}' uconsole-for-dspi)\" = GPL-3.0-only ] \
            || { echo 'licences: no GPL text in \$lic, or the License field is not GPL-3.0-only'; exit 1; }
        notice() { $notice_check
        }; notice \$lic/NOTICE.txt \$lic
        names() { $names_check
        }; names
        run() { $run_app
        }; QT_QPA_PLATFORM=offscreen run uconsole-for-dspi"
done
# The AppImage on a system with no Qt at all, on X11 (Xvfb): it brings its own.
# Like every AppImage it uses the host's ALSA, GL, fontconfig and a few more
# (make-appimage.sh's EXCLUDED), installed here without recommends
# (not ubuntu:22.04: its glibc 2.35 is older than the release's 2.39)
for img in docker.io/library/ubuntu:24.04 docker.io/library/debian:13 registry.fedoraproject.org/fedora:44; do
    name=appimage-${img##*/}; name=${name//:/-}
    t "$name" "$img" "
        if command -v apt-get >/dev/null; then
            apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends xvfb xauth libgl1 libegl1 libopengl0 libfontconfig1 libfreetype6 libharfbuzz0b \
                libx11-6 libx11-xcb1 libxcb1 libsm6 libice6 libwayland-client0 libcom-err2 libgpg-error0 libasound2t64 libdbus-1-3 >/dev/null
        else dnf -y -q install xorg-x11-server-Xvfb mesa-libGL mesa-libEGL fontconfig alsa-lib libglvnd-opengl dbus-libs >/dev/null; fi
        cp /dist/uConsole-for-DSPi-*-x86_64.AppImage /root/app.AppImage && chmod +x /root/app.AppImage
        cd /root && ./app.AppImage --appimage-extract >/dev/null
        grep -q apprun-hooks squashfs-root/AppRun || { echo 'AppRun does not source apprun-hooks'; exit 1; }
        grep -aqP '$feed' squashfs-root/usr/bin/uconsole-for-dspi || { echo 'feed: the AppImage has no update feed'; exit 1; }
        lic() { $lic_check
        }; lic squashfs-root/usr/share/doc/uconsole-for-dspi squashfs-root/usr/lib
        Xvfb :5 -nolisten tcp >/dev/null 2>&1 & export DISPLAY=:5; sleep 1
        run() { $run_app
        }; APPIMAGE_EXTRACT_AND_RUN=1 run /root/app.AppImage"
done
# -- WP QD42 -- The archive carries the AppImage's licence files beside it;
# -- WP QP20 -- and its install.sh installs a per-user copy and removes it
t archive docker.io/library/debian:13 "
    mkdir /root/x && tar -xzf /dist/uConsole-for-DSPi-*-linux-x86_64.tar.gz -C /root/x && cd /root/x/uConsole-for-DSPi-*-linux-x86_64
    for f in NOTICE.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/Lucide/LICENSE \
             install.sh README.txt 70-uconsole-for-dspi.rules uconsole-for-dspi.desktop uconsole-for-dspi.svg; do
        test -s \$f || { echo \"archive: no \$f\"; exit 1; }
    done
    ls uConsole-for-DSPi-*-x86_64.AppImage >/dev/null
    export HOME='/root/home dir' PATH=\$PATH
    ./install.sh >/dev/null
    e=\"\$HOME/.local/share/applications/uconsole-for-dspi.desktop\"
    test -x \"\$HOME/.local/bin/uconsole-for-dspi\" && grep -qx 'Name=uConsole for DSPi' \"\$e\" \
        && grep -qx 'Exec=\"/root/home dir/.local/bin/uconsole-for-dspi\"' \"\$e\" \
        && test -s \"\$HOME/.local/share/icons/hicolor/scalable/apps/uconsole-for-dspi.svg\" \
        || { echo 'archive: install.sh did not install the app, its entry (Exec quoted) and its icon'; cat \"\$e\"; exit 1; }
    ./install.sh --uninstall >/dev/null
    ! test -e \"\$HOME/.local/bin/uconsole-for-dspi\" && ! test -e \"\$e\" || { echo 'archive: install.sh --uninstall left files'; exit 1; }
    echo 'archive: NOTICE.txt, LICENSES/, install.sh, the rule; install.sh installs and removes a per-user copy'"
[ "$fails" = 0 ] && echo "test-packages: PASS" || echo "test-packages: $fails failed"
[ "$fails" = 0 ]
