#!/bin/bash
# Lay out a runnable Windows folder from a MinGW cross build, on Linux: the
# exe, dspi_core.dll, the Qt and MinGW runtime DLLs it loads, the Qt plugins
# and the QML modules the app imports. What windeployqt does on Windows.
#
#   deploy-mingw.sh <build dir> <output dir> [qt major, default 6]
#
# Needs x86_64-w64-mingw32-objdump, the mingw Qt (Fedora: mingw64-qt6-*),
# and the host qmlimportscanner of the same Qt major (Fedora:
# qt6-qtdeclarative-devel). Override MINGW_ROOT / QMLIMPORTSCANNER if your
# distribution puts them elsewhere.
set -euo pipefail
build=$1; out=$2; major=${3:-6}
here=$(cd "$(dirname "$0")" && pwd)
src=$(cd "$here/../../qt" && pwd)
M=${MINGW_ROOT:-/usr/x86_64-w64-mingw32/sys-root/mingw}
OBJDUMP=${OBJDUMP:-x86_64-w64-mingw32-objdump}
STRIP=${STRIP:-x86_64-w64-mingw32-strip}
if [ "$major" = 6 ]; then
    plugins=$M/lib/qt6/plugins; qml=$M/lib/qt6/qml
    scanner=${QMLIMPORTSCANNER:-/usr/lib64/qt6/libexec/qmlimportscanner}
    plugin_list="platforms/qwindows.dll imageformats/qsvg.dll imageformats/qico.dll iconengines/qsvgicon.dll
                 styles/qmodernwindowsstyle.dll tls/qschannelbackend.dll tls/qcertonlybackend.dll"
else
    plugins=$M/lib/qt5/plugins; qml=$M/lib/qt5/qml
    scanner=${QMLIMPORTSCANNER:-/usr/lib64/qt5/bin/qmlimportscanner}
    plugin_list="platforms/qwindows.dll imageformats/qsvg.dll imageformats/qico.dll iconengines/qsvgicon.dll
                 styles/qwindowsvistastyle.dll"
fi

rm -rf "$out"; mkdir -p "$out"
cp "$build/DSPiConsole.exe" "$build/dspi_core.dll" "$out/"

# Plugins, in the folders Qt looks for them next to the exe
for p in $plugin_list; do
    [ -f "$plugins/$p" ] || { echo "skip missing plugin $p" >&2; continue; }
    mkdir -p "$out/$(dirname "$p")"; cp "$plugins/$p" "$out/$p"
done

# QML modules the app's QML imports (and what they import), whole folders
"$scanner" -rootPath "$src/qml" -importPath "$qml" -qrcFiles "$src/resources.qrc" 2>/dev/null \
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
# Qt 6 Fusion and the labs modules load their implementation modules at run time
for extra in QtQuick/Controls/impl QtQuick/Controls/Fusion/impl QtQuick/Controls/Basic QtQuick/Controls/Basic/impl \
             QtQuick/Templates QtQml/WorkerScript QtQml/Models QtQml/Base; do
    [ -d "$qml/$extra" ] && [ ! -d "$out/qml/$extra" ] || continue
    mkdir -p "$out/qml/$extra"; find "$qml/$extra" -maxdepth 1 -type f ! -name '*.a' -exec cp {} "$out/qml/$extra/" \;
done

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

printf '[Paths]\nPrefix = .\nPlugins = .\nQml2Imports = qml\nQmlImports = qml\n' > "$out/qt.conf"
echo "deployed to $out: $(find "$out" -type f | wc -l) files, $(du -sh "$out" | cut -f1)"
