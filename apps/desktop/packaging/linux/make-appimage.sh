#!/usr/bin/env bash
# -- WP QP20 -- Makes the Linux release files from build-ubuntu.sh's AppDir, in
# Containerfile.ubuntu's image:
#
#   apps/desktop/packaging/linux/make-appimage.sh <AppDir> <output dir>
#
# writes (the names the app's update checker looks for, packaging/README.md):
#   uConsole-for-DSPi-<version>-x86_64.AppImage
#   uConsole-for-DSPi-<version>-linux-x86_64.tar.gz   the AppImage with install.sh,
#       README.txt, the desktop entry, the icon, the udev rule and the notices
#
# The AppDir gets Qt from /opt/qt as the Qt installer lays it out (usr/lib,
# usr/plugins, usr/qml, found through qt.conf and the libraries' RUNPATHs), the
# QML modules the app imports (qmlimportscanner), the libraries those load
# that a desktop may lack (every other one is the system's: EXCLUDED below),
# and the notices (licences/stage.sh). The AppImage is the pinned type2
# runtime (appimage-runtime.sha256, $APPIMAGE_RUNTIME or downloaded and
# checked) followed by a squashfs of the AppDir. Qt stays a set of shared
# libraries (LGPL-3): a user can replace them in an extracted AppImage.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
appdir=$(cd "${1:?usage: make-appimage.sh <AppDir> <output dir>}" && pwd)
out=${2:?usage: make-appimage.sh <AppDir> <output dir>}
Q=${QTDIR:-/opt/qt}
fail() { echo "make-appimage: $*" >&2; exit 1; }
version=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$here/../../qt/CMakeLists.txt")
[ -n "$version" ] || fail "no version in qt/CMakeLists.txt"
bin=$appdir/usr/bin/uconsole-for-dspi
[ -x "$bin" ] || fail "no usr/bin/uconsole-for-dspi in $appdir (run build-ubuntu.sh)"

# ---- Qt's plugins: the platforms (X11, Wayland), what they load, images, TLS,
# the portal theme for file dialogs, input methods
plugins="platforms/libqxcb.so platforms/libqwayland-generic.so platforms/libqwayland-egl.so
         platforms/libqoffscreen.so xcbglintegrations wayland-shell-integration
         wayland-decoration-client wayland-graphics-integration-client
         platforminputcontexts imageformats/libqsvg.so imageformats/libqico.so
         iconengines/libqsvgicon.so tls/libqopensslbackend.so tls/libqcertonlybackend.so
         platformthemes/libqxdgdesktopportal.so generic/libqtuiotouchplugin.so"
mkdir -p "$appdir/usr/plugins" "$appdir/usr/qml" "$appdir/usr/lib"
for p in $plugins; do
    [ -e "$Q/plugins/$p" ] || { echo "make-appimage: skip missing plugin $p" >&2; continue; }
    mkdir -p "$appdir/usr/plugins/$(dirname "$p")"
    cp -a "$Q/plugins/$p" "$appdir/usr/plugins/$(dirname "$p")/"
done
# (Qt's Wayland folders also hold the compositor's side, which a client never loads)
rm -f "$appdir"/usr/plugins/wayland-graphics-integration-client/*-server.so

# ---- QML modules the app's QML imports (its own module is in the binary)
"$Q/libexec/qmlimportscanner" -rootPath "$here/../../qt/qml" -importPath "$Q/qml" 2>/dev/null \
  | python3 -c '
import json, sys
for m in json.load(sys.stdin):
    p = m.get("path")
    if m.get("type") == "module" and p: print(p)' \
  | sort -u | while read -r dir; do
    case "$dir" in "$Q/qml"/*) ;; *) continue ;; esac
    rel=${dir#"$Q/qml"/}
    mkdir -p "$appdir/usr/qml/$rel"
    find "$dir" -maxdepth 1 -type f ! -name '*.debug' -exec cp {} "$appdir/usr/qml/$rel/" \;
done
for extra in $(sed -n 's/^runtime //p' "$here/../qt-modules.txt"); do
    [ -d "$Q/qml/$extra" ] && [ ! -d "$appdir/usr/qml/$extra" ] || continue
    mkdir -p "$appdir/usr/qml/$extra"; find "$Q/qml/$extra" -maxdepth 1 -type f ! -name '*.debug' -exec cp {} "$appdir/usr/qml/$extra/" \;
done
[ -d "$appdir/usr/qml/QtQuick/Controls" ] || fail "no QtQuick.Controls found by qmlimportscanner"
printf '[Paths]\nPrefix = ..\nLibraries = lib\nPlugins = plugins\nQmlImports = qml\n' >"$appdir/usr/bin/qt.conf"
# The binary finds the bundled Qt beside it, never the build's /opt/qt
patchelf --set-rpath '$ORIGIN/../lib' "$bin"

# ---- Libraries. The ones every desktop has stay the system's (glibc, the C++
# runtime, GL/EGL and the GPU stack, X11 and xcb's core, Wayland's client,
# fontconfig and FreeType, HarfBuzz, ALSA, D-Bus, zlib): bundling them breaks
# graphics drivers and the system's ABI. Everything else comes along.
EXCLUDED='^(linux-vdso|ld-linux-x86-64|libc|libm|libmvec|libdl|libpthread|librt|libresolv|libutil|libanl|libnss_.*|libthread_db|libstdc\+\+|libgcc_s|libGL|libEGL|libGLX|libGLdispatch|libOpenGL|libglapi|libgbm|libdrm|libX11|libX11-xcb|libxcb|libXext|libXau|libXdmcp|libwayland-client|libfontconfig|libfreetype|libharfbuzz|libasound|libdbus-1|libz|libexpat|libuuid|libcom_err|libgpg-error|libICE|libSM|libudev|libsystemd|libgcrypt|liblzma|libzstd|liblz4|libcap|libbz2)\.so'
deps() {  # every library ldd resolves for the ELF files under the AppDir
    find "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/plugins" "$appdir/usr/qml" -type f \
        \( -name '*.so*' -o -perm -u+x \) -print0 \
      | xargs -0 -r file -L | sed -n 's/^\([^:]*\): *ELF.*/\1/p' \
      | while read -r f; do LD_LIBRARY_PATH=$Q/lib:$appdir/usr/lib ldd "$f" 2>/dev/null; done \
      | sed -n 's/^\s*\([^ ]*\) => \(\/[^ ]*\) .*/\1 \2/p' | sort -u
}
missing=$(find "$appdir/usr" -type f -name '*.so*' -print0 | xargs -0 -r -n1 sh -c 'LD_LIBRARY_PATH='"$Q/lib:$appdir/usr/lib"' ldd "$0" 2>/dev/null | grep "not found" | sed "s|^|$0: |"' || true)
[ -z "$missing" ] || fail "libraries not found (install them in Containerfile.ubuntu):
$missing"
changed=1
while [ $changed = 1 ]; do
    changed=0
    while read -r soname path; do
        [ -e "$appdir/usr/lib/$soname" ] && continue
        [[ $soname =~ $EXCLUDED ]] && continue
        cp -L "$path" "$appdir/usr/lib/$soname"; changed=1
        # Ubuntu's libraries look for their own dependencies beside them
        case $path in "$Q"/*) ;; *) patchelf --set-rpath '$ORIGIN' "$appdir/usr/lib/$soname" ;; esac
    done < <(deps)
done
# Plugins copied as symlinks into Qt's tree would point outside the AppDir
find "$appdir/usr" -type l | while read -r l; do
    t=$(readlink -f "$l"); case $t in "$appdir"/*) ;; *) rm "$l"; cp -L "$t" "$l" ;; esac
done
strip --strip-unneeded "$appdir"/usr/lib/*.so* 2>/dev/null || true

# ---- AppRun, its hooks, the AppImage's desktop entry and icon at the top
mkdir -p "$appdir/apprun-hooks"
cp "$here/apprun-hook.sh" "$appdir/apprun-hooks/uconsole-for-dspi-qt.sh"
cat >"$appdir/AppRun" <<'APPRUN'
#!/bin/sh
# Starts uConsole for DSPi from its AppImage: the hooks in apprun-hooks/ set
# up the environment, then the app runs with the AppImage's own Qt.
APPDIR=${APPDIR:-$(dirname "$(readlink -f "$0")")}
export APPDIR
for hook in "$APPDIR"/apprun-hooks/*.sh; do
    [ -f "$hook" ] && . "$hook"
done
exec "$APPDIR/usr/bin/uconsole-for-dspi" "$@"
APPRUN
chmod 755 "$appdir/AppRun"
cp "$appdir/usr/share/applications/uconsole-for-dspi.desktop" "$appdir/uconsole-for-dspi.desktop"
cp "$appdir/usr/share/icons/hicolor/scalable/apps/uconsole-for-dspi.svg" "$appdir/uconsole-for-dspi.svg"
ln -sf uconsole-for-dspi.svg "$appdir/.DirIcon"

# ---- The runtime, pinned
pin=$(grep -v '^#' "$here/appimage-runtime.sha256" | head -n 1)
sum=${pin%% *}; url=${pin##* }
runtime=${APPIMAGE_RUNTIME:-}
if [ -z "$runtime" ]; then
    runtime=${XDG_CACHE_HOME:-$HOME/.cache}/uconsole-for-dspi/runtime-x86_64-${sum:0:12}
    if [ ! -f "$runtime" ]; then
        mkdir -p "$(dirname "$runtime")"
        curl -sSfL --retry 4 --retry-all-errors --max-time 120 -o "$runtime.part" "$url"
        mv "$runtime.part" "$runtime"
    fi
fi
echo "$sum  $runtime" | sha256sum -c --quiet - || fail "$runtime is not the pinned runtime ($sum)"

# ---- The notices (they name the runtime's commit, from the runtime itself)
APPIMAGE_RUNTIME=$runtime QTDIR=$Q "$here/../licences/stage.sh" appimage "$appdir" \
    "$appdir/usr/share/doc/uconsole-for-dspi" "$version"

# ---- The AppImage: the runtime, then the squashfs it mounts
mkdir -p "$out"
out=$(cd "$out" && pwd)
image=$out/uConsole-for-DSPi-$version-x86_64.AppImage
rm -f "$image" "$out/.appdir.squashfs"
mksquashfs "$appdir" "$out/.appdir.squashfs" -root-owned -noappend -comp zstd -quiet
cat "$runtime" "$out/.appdir.squashfs" >"$image"
rm "$out/.appdir.squashfs"
chmod 755 "$image"

# ---- The archive: the AppImage and what installs it
name=uConsole-for-DSPi-$version-linux-x86_64
stage=$out/.$name
rm -rf "$stage"; mkdir -p "$stage/$name"
cp "$image" "$stage/$name/"
cp "$here/install.sh" "$here/README.txt" "$here/70-uconsole-for-dspi.rules" "$here/uconsole-for-dspi.desktop" "$stage/$name/"
cp "$appdir/uconsole-for-dspi.svg" "$stage/$name/"
cp -R "$appdir/usr/share/doc/uconsole-for-dspi/NOTICE.txt" "$appdir/usr/share/doc/uconsole-for-dspi/LICENSES" "$stage/$name/"
chmod 755 "$stage/$name/install.sh"
tar -C "$stage" --owner=0 --group=0 --sort=name -czf "$out/$name.tar.gz" "$name"
rm -rf "$stage"
ls -la "$image" "$out/$name.tar.gz"
