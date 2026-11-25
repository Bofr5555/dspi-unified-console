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
#   - Qt: ci/check.sh --step qt-offscreen with the sections below (the smoke driver's
#     screenshots on the demo device and `uconsole --demo hub --alink`); its output is
#     <build cache>/work/<branch>/smoke-out
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
smoke=$B/work/${branch//\//-}/smoke-out
mkdir -p "$work"
work=$(cd "$work" && pwd)

if [ "${SKIP_RENDER:-}" != 1 ]; then
    (cd "$root" && DSPI_SMOKE_SECTION=qp0-base,lkui1-system,p5qui-measure,p5qui-tune,atqtsim-units,qp19-gallery \
        ci/check.sh --step qt-offscreen)
    "$root/tools/shots/web.sh" "$work/web"
    "$root/tools/shots/terminal.sh" "$work/terminal"
fi

out=$root/docs/images
mkdir -p "$out"
# name | source (in the container) | crop (WxH+X+Y, or "-" for none)
shots=(
    "qt-main|/smoke/window-main-light.png|-"
    "web-simple|/work/web/web-desktop-demo-simple-light.png|1366x860+0+0"
    "qt-system|/smoke/lkui1/system-light.png|880x570+0+0"
    "web-system|/work/web/web-desktop-hub-system-admin-light.png|1366x860+0+0"
    "qt-measure|/smoke/gallery/measure-light.png|940x600+0+0"
    "terminal|/work/terminal/png/terminal-tui-main-dark.png|-"
)
podman run --rm --network=none --userns=keep-id --user "$(id -u):$(id -g)" \
    -v "$smoke":/smoke:ro,z -v "$work":/work:ro,z -v "$out":/out:z -e HOME=/out \
    "$COREMERGE" bash -euo pipefail -c '
        for s in "$@"; do
            IFS="|" read -r name src crop <<< "$s"
            args=("$src")
            [ "$crop" = - ] || args+=(-crop "$crop" +repage)
            magick "${args[@]}" -resize "1400x>" -strip -define png:compression-level=9 \
                -define png:exclude-chunks=date,time "/out/$name.png"
            echo "$name.png: $(magick identify -format "%wx%h" "/out/$name.png"), $(( $(stat -c %s "/out/$name.png") / 1024 )) KB"
        done' _ "${shots[@]}"
