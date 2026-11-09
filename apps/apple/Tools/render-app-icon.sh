#!/bin/bash
# Render the Swift app's AppIcon set from the project's icon (jobs REL013M2, ICON1).
#
#   assets/icon/render.sh      (runs this, after drawing the masters)
#   ci/run.sh apps/apple/Tools/render-app-icon.sh     (this alone: ImageMagick)
#
# The masters are assets/icon/dspi-icon-*-1024.png, drawn by
# apps/android/store/play/tools/make_graphics.py. The macOS and iOS targets share
# the set:
#   macOS  16, 32, 128, 256 and 512 pt at 1x and 2x, from the rounded master
#          (the macOS icon grid: rounded square, transparent margin)
#   iOS    one 1024 px universal image, from the square master (full bleed, no
#          alpha: iOS applies its own mask)
# Commit the PNGs.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
rounded=$repo/assets/icon/dspi-icon-rounded-1024.png
square=$repo/assets/icon/dspi-icon-square-1024.png
out="$repo/apps/apple/DSPi Console/Assets.xcassets/AppIcon.appiconset"

check() {  # check <file> <size> <channels>
    local got
    got=$(magick identify -format '%wx%h %[channels]' "$1")
    case $got in "$2 $3 "*) ;; *) echo "$1 is $got, not $2 $3" >&2; exit 1 ;; esac
}
for px in 16 32 64 128 256 512 1024; do
    magick "$rounded" -filter Lanczos -resize "${px}x${px}" \
        -strip -define png:exclude-chunks=date,time "PNG32:$out/icon_${px}.png"
    check "$out/icon_${px}.png" "${px}x${px}" srgba
done
magick "$square" -alpha off -strip -define png:exclude-chunks=date,time "PNG24:$out/icon_ios_1024.png"
check "$out/icon_ios_1024.png" 1024x1024 srgb
echo "render-app-icon: wrote 8 images to $out"
