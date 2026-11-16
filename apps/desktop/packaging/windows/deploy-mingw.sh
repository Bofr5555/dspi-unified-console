#!/bin/bash
# Lay out a runnable Windows folder from a MinGW cross build, on Linux: the
# exe, dspi_capi.dll, the Qt and MinGW runtime DLLs it loads, the Qt plugins
# and the QML modules the app imports. What windeployqt does on Windows.
#
#   deploy-mingw.sh <build dir> <output dir>
#
# Needs x86_64-w64-mingw32-objdump, the mingw Qt 6 (Fedora: mingw64-qt6-*),
# and the host qmlimportscanner of Qt 6 (Fedora: qt6-qtdeclarative-devel).
# Override MINGW_ROOT / QMLIMPORTSCANNER if your distribution puts them
# elsewhere.
set -euo pipefail
build=$1; out=$2
here=$(cd "$(dirname "$0")" && pwd)
src=$(cd "$here/../../qt" && pwd)
M=${MINGW_ROOT:-/usr/x86_64-w64-mingw32/sys-root/mingw}
OBJDUMP=${OBJDUMP:-x86_64-w64-mingw32-objdump}
STRIP=${STRIP:-x86_64-w64-mingw32-strip}
plugins=$M/lib/qt6/plugins; qml=$M/lib/qt6/qml
scanner=${QMLIMPORTSCANNER:-/usr/lib64/qt6/libexec/qmlimportscanner}
# -- WP QP20 -- tls/ for the update checker's https requests
plugin_list="platforms/qwindows.dll imageformats/qsvg.dll imageformats/qico.dll iconengines/qsvgicon.dll
             styles/qmodernwindowsstyle.dll tls/qschannelbackend.dll tls/qcertonlybackend.dll"

rm -rf "$out"; mkdir -p "$out"
cp "$build/uconsole-for-dspi.exe" "$build/dspi_capi.dll" "$out/"

# Plugins, in the folders Qt looks for them next to the exe
for p in $plugin_list; do
    [ -f "$plugins/$p" ] || { echo "skip missing plugin $p" >&2; continue; }
    mkdir -p "$out/$(dirname "$p")"; cp "$plugins/$p" "$out/$p"
done

# QML modules the app's QML imports (and what they import), whole folders.
# -- WP QP20 -- The app's own module (UConsole) is compiled into the exe; the
# scanner reads its sources for their imports.
"$scanner" -rootPath "$src/qml" -importPath "$qml" 2>/dev/null \
  | python3 -c '
import json, sys
for m in json.load(sys.stdin):
    p = m.get("path")
    if m.get("type") == "module" and p: print(p)' \
  | sort -u | while read -r dir; do
    case "$dir" in "$qml"/*) ;; *) continue ;; esac
    rel=${dir#"$qml"/}
    mkdir -p "$out/qml/$rel"
    # Only the module's own files; nested modules come on their own lines
    find "$dir" -maxdepth 1 -type f ! -name '*.a' ! -name '*d.dll' -exec cp {} "$out/qml/$rel/" \;
done
# The Basic style and the controls load their implementation modules at run time
for extra in $(sed -n 's/^runtime //p' "$here/../qt-modules.txt"); do
    [ -d "$qml/$extra" ] && [ ! -d "$out/qml/$extra" ] || continue
    mkdir -p "$out/qml/$extra"; find "$qml/$extra" -maxdepth 1 -type f ! -name '*.a' -exec cp {} "$out/qml/$extra/" \;
done
[ -d "$out/qml/QtQuick/Controls" ] || { echo "deploy-mingw: no QtQuick.Controls found by $scanner" >&2; exit 1; }

# Every DLL any of those loads that ships with mingw / Qt (system DLLs stay out)
changed=1
while [ $changed = 1 ]; do
    changed=0
    while read -r dll; do
        [ -f "$out/$dll" ] && continue
        if [ -f "$M/bin/$dll" ]; then cp "$M/bin/$dll" "$out/"; changed=1; fi
    done < <(find "$out" -iname '*.dll' -o -iname '*.exe' | xargs -r "$OBJDUMP" -p 2>/dev/null \
             | sed -n 's/^\s*DLL Name: //p' | sort -u)
done

# Distribution DLLs keep their symbols; the bundle doesn't need them
find "$out" \( -iname '*.dll' -o -iname '*.exe' \) -exec "$STRIP" --strip-unneeded {} + 2>/dev/null || true

# -- WP P5QCAP -- The microphone: Qt6Multimedia.dll (its WASAPI audio backend is
# built in; QAudioSource and QMediaDevices need no media plugin). No multimedia/
# plugin is bundled, so neither FFmpeg nor Media Foundation playback is in the
# package, and nothing of FFmpeg's may come in through a dependency.
[ -f "$out/Qt6Multimedia.dll" ] || { echo "deploy-mingw: Qt6Multimedia.dll is missing (the app's microphone)" >&2; exit 1; }
[ ! -e "$out/multimedia" ] || { echo "deploy-mingw: a multimedia/ plugin folder was bundled" >&2; exit 1; }
if find "$out" -iname 'av*.dll' -o -iname 'sw*.dll' -o -iname '*ffmpeg*' | grep -q .; then
    echo "deploy-mingw: FFmpeg files in the bundle:" >&2; find "$out" -iname 'av*.dll' -o -iname 'sw*.dll' -o -iname '*ffmpeg*' >&2; exit 1
fi

printf '[Paths]\nPrefix = .\nPlugins = .\nQml2Imports = qml\nQmlImports = qml\n' > "$out/qt.conf"
echo "deployed to $out: $(find "$out" -type f | wc -l) files, $(du -sh "$out" | cut -f1)"
