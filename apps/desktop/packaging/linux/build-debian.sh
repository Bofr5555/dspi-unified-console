#!/usr/bin/env bash
# Runs inside the Debian build container (see build-release.sh): the
# AppImage, the release archive and the .deb, on Qt 6.
set -euo pipefail
export APPIMAGE_EXTRACT_AND_RUN=1
tools=/dist/tools
name="DSPi-Console-$VERSION"
here=/src/apps/desktop/packaging/linux

# Build from a copy, leaving the host's build directories alone
rm -rf /work && mkdir /work
tar -C /src --exclude=./target --exclude=./apps/desktop/qt/build --exclude=./.git --exclude=./dist -cf - . | tar -C /work -xf -
cd /work
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cd /work/apps/desktop
cmake -S qt -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6
cmake --build build -j"${JOBS:-4}"
"$here/check-binary.sh" build/DSPiConsole 2.41

# The app, its Qt libraries, QML modules and plugins (X11 and Wayland)
export QMAKE=/usr/lib/qt6/bin/qmake
export QML_SOURCES_PATHS=/work/apps/desktop/qt/qml
export EXTRA_PLATFORM_PLUGINS="libqwayland-generic.so;libqwayland-egl.so"
export EXTRA_QT_MODULES="waylandcompositor;svg"
cp qt/dspi-console.svg build/dspi-console.svg
cp build/DSPiConsole build/dspi-console
# In place before linuxdeploy writes AppRun, which then sources the hooks; named
# to run after any hook linuxdeploy-plugin-qt adds
mkdir -p AppDir/apprun-hooks
cp packaging/linux/apprun-hook.sh AppDir/apprun-hooks/zz-dspi-console.sh
"$tools/linuxdeploy-x86_64.AppImage" --appdir AppDir -e build/dspi-console \
    -d packaging/linux/dspi-console.desktop -i build/dspi-console.svg --plugin qt

# KDE window effects (blur, shadow) load these at run time
plugins=/usr/lib/x86_64-linux-gnu/qt6/plugins
mkdir -p AppDir/usr/plugins/kf6
cp -r "$plugins/kf6/kwindowsystem" AppDir/usr/plugins/kf6/
# Native file dialogs through the desktop portal
mkdir -p AppDir/usr/plugins/platformthemes
cp "$plugins/platformthemes/libqxdgdesktopportal.so" AppDir/usr/plugins/platformthemes/
"$tools/linuxdeploy-x86_64.AppImage" --appdir AppDir --deploy-deps-only AppDir/usr/plugins/kf6/kwindowsystem \
    --deploy-deps-only AppDir/usr/plugins/platformthemes

# The runtime build-release.sh fetched, so appimagetool downloads nothing
OUTPUT="$name-x86_64.AppImage" LDAI_RUNTIME_FILE="$tools/runtime-x86_64" "$tools/linuxdeploy-x86_64.AppImage" --appdir AppDir --output appimage

# Release archive: the AppImage, the udev rule and the installer
rm -rf "$name" && mkdir "$name"
cp "$name-x86_64.AppImage" packaging/linux/70-dspi.rules packaging/linux/install.sh qt/dspi-console.svg "$name/"
cp packaging/linux/README.txt "$name/"
tar -czf "$name-linux-x86_64.tar.gz" "$name"
cp "$name-x86_64.AppImage" "$name-linux-x86_64.tar.gz" /dist/

# The .deb, against the system Qt 6 (Debian 13 and newer, Ubuntu 25.04 and newer)
(cd build && cpack -G DEB && cp dspi-console_*.deb /dist/)
