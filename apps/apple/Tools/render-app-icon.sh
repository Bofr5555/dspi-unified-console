#!/bin/bash
# Render the Swift app's AppIcon set from the desktop app's icon (job REL013M2).
#
#   ci/run.sh apps/apple/Tools/render-app-icon.sh
#
# The artwork is apps/desktop/qt/dspi-console.svg, the icon the Qt app and its
# dspi-console.icns use; this only rasterises it (ImageMagick with librsvg, in
# the build container), once per pixel size the asset catalog names:
#   macOS  16, 32, 128, 256 and 512 pt at 1x and 2x
#   iOS    one 1024 px universal image (the macOS and iOS targets share the set)
# Run it again after changing the SVG, and commit the PNGs.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../../.." && pwd)
svg=$repo/apps/desktop/qt/dspi-console.svg
out="$repo/apps/apple/DSPi Console/Assets.xcassets/AppIcon.appiconset"

for px in 16 32 64 128 256 512 1024; do
    # The SVG is 256 px square and rsvg takes 96 dpi as 1:1, so this draws it at $px.
    density=$(awk "BEGIN { print 96 * $px / 256 }")
    magick -background none -density "$density" "$svg" \
        -strip -define png:exclude-chunks=date,time \
        "PNG32:$out/icon_${px}.png"
    size=$(magick identify -format '%wx%h' "$out/icon_${px}.png")
    [ "$size" = "${px}x${px}" ] || { echo "icon_${px}.png is $size" >&2; exit 1; }
done
echo "render-app-icon: wrote 7 sizes to $out"
