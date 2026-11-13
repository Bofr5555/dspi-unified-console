#!/bin/bash
# A contact sheet: labelled thumbnails of PNGs in a grid, with ImageMagick in the build
# container (ci/run.sh's image; nothing on the host).
#
#   tools/shots/sheet.sh <shots dir> <list file: one PNG name per line> <out.png> <title> [columns]
#
# Each thumbnail is the picture's top, 300 px wide and at most 420 px tall (a whole web page
# is long); its label is the file name without the number and .png.
set -euo pipefail
dir=$(cd "$1" && pwd); list=$(realpath "$2"); out=$(realpath -m "$3"); title=$4; cols=${5:-6}
root=$(cd "$(dirname "$0")/../.." && pwd)
. "$root/ci/container/images.conf"
mkdir -p "$(dirname "$out")"
podman run --rm --network=none --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$dir":/shots:ro,z -v "$list":/list.txt:ro,z -v "$(dirname "$out")":/out:z \
  -e HOME=/out -e TITLE="$title" -e COLS="$cols" -e OUT="$(basename "$out")" \
  "$COREMERGE" bash -euo pipefail -c '
    t=$(mktemp -d /out/.thumbs.XXXXXX)
    args=()
    while read -r f; do
      [ -n "$f" ] || continue
      label=${f#*-}; label=${label%.png}
      magick "/shots/$f" -resize 300x -gravity north -crop 300x420+0+0 +repage "$t/$f"
      args+=(-label "$label" "$t/$f")
    done < /list.txt
    magick montage "${args[@]}" -tile "${COLS}x" -geometry "300x420>+8+8" \
      -background "#202124" -fill "#e8e8e8" -font Helvetica -pointsize 13 \
      -title "$TITLE" "/out/$OUT"
    rm -rf "$t"'
echo "sheet: $out"
