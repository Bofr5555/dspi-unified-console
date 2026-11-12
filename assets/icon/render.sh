#!/bin/bash
# The icon of every client, from one source (job ICON1):
#
#   assets/icon/render.sh
#
# Runs in its own container (assets/icon/Containerfile, built on first use). The
# drawing is apps/android/store/play/tools/make_graphics.py's, the Play icon's; this
#   1. draws assets/icon/dspi-icon-{square,rounded}-1024.png and .svg
#      (square: full bleed; rounded: the macOS app icon grid, see icons() there);
#   2. makes from them the Qt desktop app's apps/desktop/qt/dspi-console.svg (window,
#      tray, Linux hicolor and the AppImage), packaging/windows/dspi-console.ico
#      and the installer's installer-side.bmp (the Qt app ships no macOS build);
#   3. the Swift app's AppIcon set (apps/apple/Tools/render-app-icon.sh);
#   4. the web console's favicon.svg, favicon.ico and apple-touch-icon.png.
# The Play graphics and the launcher layers are make_graphics.py's other outputs;
# this leaves them alone. Commit what it changes.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
image=localhost/dspi-icon:1
if [ "${DSPI_ICON_CONTAINER:-}" != 1 ]; then
    podman image exists "$image" || podman build -t "$image" -f "$repo/assets/icon/Containerfile" "$repo/assets/icon"
    exec podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" -v "$repo":/src:z -w /src \
        -e DSPI_ICON_CONTAINER=1 -e HOME=/src/assets/icon "$image" assets/icon/render.sh
fi

cd "$repo"
src=assets/icon
python3 apps/android/store/play/tools/make_graphics.py --icons "$src"
work=$(mktemp -d "$src/.render.XXXXXX")
trap 'rm -rf "$work"' EXIT
png() {  # png <master> <px> <out>: a smaller copy, no metadata
    magick "$1" -filter Lanczos -resize "$2x$2" -strip -define png:exclude-chunks=date,time "PNG32:$3"
}

# Qt desktop app
cp "$src/dspi-icon-rounded.svg" apps/desktop/qt/dspi-console.svg
for px in 16 24 32 48 64 128 256 512; do png "$src/dspi-icon-rounded-1024.png" "$px" "$work/r$px.png"; done
icotool -c -o apps/desktop/packaging/windows/dspi-console.ico \
    "$work"/r{16,24,32,48,64,128}.png -r "$work/r256.png"
# the installer's welcome/finish page (164 x 314, 24-bit BMP): the icon's 86 px tile
# on the dark background, where the first artwork had it
magick -size 164x314 xc:'#1e1e20' \( "$src/dspi-icon-rounded-1024.png" -filter Lanczos -resize 107x107 \) \
    -geometry +29+42 -composite -alpha off -type TrueColor -define bmp:format=bmp3 \
    BMP3:apps/desktop/packaging/windows/installer-side.bmp

# Swift app, macOS and iOS
apps/apple/Tools/render-app-icon.sh

# web console
web=apps/web/icons
mkdir -p "$web"
cp "$src/dspi-icon-rounded.svg" "$web/favicon.svg"
icotool -c -o "$web/favicon.ico" "$work"/r{16,32,48}.png
magick "$src/dspi-icon-square-1024.png" -filter Lanczos -resize 180x180 -strip \
    -define png:exclude-chunks=date,time "PNG24:$web/apple-touch-icon.png"
echo "render: done"
