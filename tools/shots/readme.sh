#!/bin/bash
# -- WP DOCS14 -- The front page's screenshots (README.md, docs/images/): real renders of
# the apps on their demo devices and the demo hub, no hardware. Renders first, then crops,
# scales (at most 1400 px wide) and recompresses them with ImageMagick in the build
# container (ci/run.sh's image; nothing on the host).
#
#   tools/shots/readme.sh <work dir>      renders into <work dir>, writes docs/images/*.png
#   SKIP_RENDER=1 tools/shots/readme.sh <work dir>   only the crops, from an earlier render
#
# The renders:
#   - Qt (-- WP FIN015 -- the Modern layout): the function test's app on a 1600x1000 screen
#     with the tuned home theatre of stories/show-gallery.toml (three simulated DSPi units and a
#     simulated microphone), gallery.py's pages and shots2.py's measurement by the user, in the
#     light theme; its output is <build cache>/work/<branch>/runs/readme/
#   - web: tools/shots/web.sh <work dir>/web (the tour on the core's demo and the demo hub)
#   - Terminal: tools/shots/terminal.sh <work dir>/terminal (uses what web.sh built)
set -euo pipefail
work=${1:?usage: readme.sh <work dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
# -- WP SHOWFIX -- a detached worktree is named by its short commit, not "HEAD"
if [ "$branch" = HEAD ]; then branch=$(git -C "$root" rev-parse --short HEAD); fi
runs=$B/work/${branch//\//-}/runs
mkdir -p "$work"
work=$(cd "$work" && pwd)

if [ "${SKIP_RENDER:-}" != 1 ]; then
    ft=$root/tools/functest
    "$ft/build.sh"
    rm -rf "$runs/readme"
    FUNCTEST_SCREEN=1600x1000 python3 "$ft/functest.py" up --name readme --story "$ft/stories/show-gallery.toml"
    python3 "$ft/gallery.py" readme light only=overview,eq-peq || true
    python3 "$ft/shots2.py" readme light || true
    python3 "$ft/functest.py" down --name readme || true
    # the tour's first pictures of each test are the ones taken here; its later steps predate the
    # Modern web console and fail (FIN015), so a failed tour does not stop the crops
    PW_ARGS=test/shots/tour.spec.ts "$root/tools/shots/web.sh" "$work/web" || true
    "$root/tools/shots/terminal.sh" "$work/terminal"
fi

out=$root/docs/images
mkdir -p "$out"
# name | source (in the container) | crop (WxH+X+Y, or "-" for none)
shots=(
    "qt-main|/runs/readme/shots/light/eq-peq-output-MainWindow-0.png|-"
    "web-eq|/work/web/web-desktop-demo-eq-light.png|1366x900+0+0"
    "qt-overview|/runs/readme/shots/light/overview-MainWindow-0.png|-"
    "web-system|/work/web/web-desktop-hub-system-admin-light.png|1366x900+0+0"
    "qt-measure|/runs/readme/shots2/light/measure-results-MainWindow-0.png|-"
    "terminal|/work/terminal/png/terminal-tui-main-dark.png|-"
)
podman run --rm --network=none --userns=keep-id --user "$(id -u):$(id -g)" \
    -v "$runs":/runs:ro,z -v "$work":/work:ro,z -v "$out":/out:z -e HOME=/out \
    "$COREMERGE" bash -euo pipefail -c '
        for s in "$@"; do
            IFS="|" read -r name src crop <<< "$s"
            args=("$src")
            [ "$crop" = - ] || args+=(-crop "$crop" +repage)
            magick "${args[@]}" -resize "1400x>" -strip -define png:compression-level=9 \
                -define png:exclude-chunks=date,time "/out/$name.png"
            echo "$name.png: $(magick identify -format "%wx%h" "/out/$name.png"), $(( $(stat -c %s "/out/$name.png") / 1024 )) KB"
        done' _ "${shots[@]}"
