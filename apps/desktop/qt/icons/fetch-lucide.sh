#!/bin/bash
# Copies the icons named in lucide/manifest.txt and in each work package's
# lucide/manifest.d/<wp>.txt from a pinned Lucide release into lucide/, with
# Lucide's licence file. Run it in the CI container:
#
#   ci/run.sh 'apps/desktop/qt/icons/fetch-lucide.sh'
#
# Sources: the release asset on GitHub (lucide-icons/lucide) and the LICENSE
# file at the same tag. Both are checked against the SHA-256 below before
# anything is copied; the asset's digest is the one GitHub's release API
# publishes for it. To move to another release, change the three values and
# check the new licence file by hand.
set -euo pipefail
VERSION=1.67.0
ZIP_SHA256=035253db719965b1eee4877f4fb675a695822181a21d06caae60b9d9c0ec4e66
LICENSE_SHA256=b495047bd93a9b06913511076f504daba17d5bbeb3e0650f3bb53a4220329c57

here=$(cd "$(dirname "$0")" && pwd)
out="$here/lucide"
work=${WORK_DIR:-/work}/lucide-$VERSION
rm -rf "$work"
mkdir -p "$work"
zip="$work/lucide-icons-$VERSION.zip"
curl -fsSL -o "$zip" "https://github.com/lucide-icons/lucide/releases/download/$VERSION/lucide-icons-$VERSION.zip"
curl -fsSL -o "$work/LICENSE" "https://raw.githubusercontent.com/lucide-icons/lucide/$VERSION/LICENSE"
echo "$ZIP_SHA256  $zip" | sha256sum -c --quiet
echo "$LICENSE_SHA256  $work/LICENSE" | sha256sum -c --quiet

names=$({ cat "$out/manifest.txt"; cat "$out"/manifest.d/*.txt 2>/dev/null || true; } | sed -n 's/^\([a-z0-9-][a-z0-9-]*\).*/\1/p' | sort -u)
find "$out" -name '*.svg' -delete
missing=0
for n in $names; do
    if ! unzip -p "$zip" "icons/$n.svg" > "$out/$n.svg" 2>/dev/null || [ ! -s "$out/$n.svg" ]; then
        echo "fetch-lucide: $n is not in Lucide $VERSION" >&2
        rm -f "$out/$n.svg"
        missing=1
    fi
done
cp "$work/LICENSE" "$out/LICENSE"
rm -rf "$work"
[ "$missing" = 0 ] || exit 1
echo "fetch-lucide: $(echo "$names" | wc -w) icons from Lucide $VERSION"
