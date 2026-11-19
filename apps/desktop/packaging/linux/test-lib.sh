# -- WP APPIMG -- Sourced by test-packages.sh and test-appimage.sh (they set
# $dist and $here): the checks both run in clean containers, and t(), which
# runs one of them and counts it in $fails.
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

# -- WP APPIMG -- The container sees no USB device (an empty folder over
# /sys/bus/usb/devices, as ci/run.sh does); [podman args] add mounts.
mkdir -p "${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/release-build/empty"
t() {  # <name> <image> <script> [podman args...]
    echo "== $1"
    if podman run --rm --security-opt label=disable -v "$dist":/dist:ro \
        -v "${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/release-build/empty":/sys/bus/usb/devices:ro "${@:4}" \
        "$2" bash -c "set -e; $3" >"$dist/test-$1.log" 2>&1; then
        tail -n 1 "$dist/test-$1.log"; echo "== $1: OK"
    else
        tail -n 30 "$dist/test-$1.log"; echo "== $1: FAILED (log $dist/test-$1.log)"; fails=$((fails + 1))
    fi
}
