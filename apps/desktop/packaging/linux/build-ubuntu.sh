#!/usr/bin/env bash
# Runs inside the Ubuntu 24.04 build container (see build-release.sh): the
# AppImage and the release archive, on the official Qt 6.8 in /opt/qt.
set -euo pipefail
export APPIMAGE_EXTRACT_AND_RUN=1
tools=/dist/tools
name="DSPi-uConsole-$VERSION"
here=/src/apps/desktop/packaging/linux

# Build from a copy, leaving the host's build directories alone
rm -rf /work && mkdir /work
tar -C /src --exclude=./target --exclude=./apps/desktop/qt/build --exclude=./.git --exclude=./dist -cf - . | tar -C /work -xf -
cd /work
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cd /work/apps/desktop
cmake -S qt -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6
cmake --build build -j"${JOBS:-4}"
"$here/check-binary.sh" build/DSPiConsole 2.39

# The app, its Qt libraries, QML modules and plugins (X11 and Wayland)
export QMAKE=$QTDIR/bin/qmake
export QML_SOURCES_PATHS=/work/apps/desktop/qt/qml
export EXTRA_PLATFORM_PLUGINS="libqwayland-generic.so;libqwayland-egl.so"
export EXTRA_QT_MODULES="waylandcompositor;svg"
cp qt/dspi-console.svg build/dspi-uconsole.svg
cp build/DSPiConsole build/dspi-uconsole
# In place before linuxdeploy writes AppRun, which then sources the hooks; named
# to run after any hook linuxdeploy-plugin-qt adds
mkdir -p AppDir/apprun-hooks
cp packaging/linux/apprun-hook.sh AppDir/apprun-hooks/zz-dspi-uconsole.sh
"$tools/linuxdeploy-x86_64.AppImage" --appdir AppDir -e build/dspi-uconsole \
    -d packaging/linux/dspi-uconsole.desktop -i build/dspi-uconsole.svg --plugin qt

# KDE window effects (blur, shadow) load these at run time; KWindowSystem is
# built in the image (/opt/kf6)
mkdir -p AppDir/usr/plugins/kf6
cp -r /opt/kf6/lib/plugins/kf6/kwindowsystem AppDir/usr/plugins/kf6/
# Native file dialogs through the desktop portal
mkdir -p AppDir/usr/plugins/platformthemes
cp "$QTDIR/plugins/platformthemes/libqxdgdesktopportal.so" AppDir/usr/plugins/platformthemes/
"$tools/linuxdeploy-x86_64.AppImage" --appdir AppDir --deploy-deps-only AppDir/usr/plugins/kf6/kwindowsystem \
    --deploy-deps-only AppDir/usr/plugins/platformthemes

# -- WP QD42 -- The licence texts, the notice and the list of bundled libraries
APPIMAGE_RUNTIME="$tools/runtime-x86_64" "$here/../licences/stage.sh" appimage AppDir AppDir/usr/share/doc/dspi-uconsole "${VERSION#v}"

# The runtime build-release.sh fetched, so appimagetool downloads nothing
OUTPUT="$name-x86_64.AppImage" LDAI_RUNTIME_FILE="$tools/runtime-x86_64" "$tools/linuxdeploy-x86_64.AppImage" --appdir AppDir --output appimage

# Release archive: the AppImage, the udev rule and the installer
rm -rf "$name" && mkdir "$name"
cp "$name-x86_64.AppImage" packaging/linux/70-dspi.rules packaging/linux/install.sh "$name/"
cp qt/dspi-console.svg "$name/dspi-uconsole.svg"
cp packaging/linux/README.txt "$name/"
cp -R AppDir/usr/share/doc/dspi-uconsole/NOTICE.txt AppDir/usr/share/doc/dspi-uconsole/LICENSES "$name/"   # -- WP QD42 --
tar -czf "$name-linux-x86_64.tar.gz" "$name"
cp "$name-x86_64.AppImage" "$name-linux-x86_64.tar.gz" /dist/
