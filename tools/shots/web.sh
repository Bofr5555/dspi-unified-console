#!/bin/bash
# The web console's screenshot tour (apps/web/test/shots/tour.spec.ts) on the core's demo
# device and on `uconsole --demo hub --alink`: dark and light, 1366 px and 375 px.
#
#   tools/shots/web.sh <out dir>          PNGs into <out dir>
#   SHOTS_ONLY=<regex> PW_ARGS=<spec…>     only the pictures whose name matches (the specs that read it)
#
# The builds are apps/web/tools/ci.sh's (`--no-e2e`: the core, the fake hub, `uconsole`, the
# page); then the tour alone, in the same Playwright image and mounts. The steps that make a
# unit lose its link use upstream's @dspi/link client, at the commit the conformance runner
# pins (crates/dspi-hub/tests/conformance/run-clients.sh), taken from a clone with
# `git archive` and checked against its tree hash. No device and no audio: demo devices only.
set -euo pipefail
out=${1:?usage: web.sh <out dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
# -- WP SHOWFIX -- a detached worktree is named by its short commit, not "HEAD"
if [ "$branch" = HEAD ]; then branch=$(git -C "$root" rev-parse --short HEAD); fi
work=$B/work/web/${branch//\//-}
mkdir -p "$out"
out=$(cd "$out" && pwd)

[ "${SKIP_BUILD:-}" = 1 ] || "$root/apps/web/tools/ci.sh" --no-e2e

DSPI_LINK_REPO=${DSPI_LINK_REPO:-$B/work/consoles/DSPi-Link}
DSPI_LINK_COMMIT=86f10e38dd760d01d4f589e4b606a5250b2d523d
DSPI_LINK_TREE=ac3f823b0d8d638ea133807da431fd9f4e2d8ebd
up=$work/upstream
link=()
if [ -d "$DSPI_LINK_REPO" ]; then
  got=$(git -C "$DSPI_LINK_REPO" rev-parse "$DSPI_LINK_COMMIT:js/packages/dspi-link")
  [ "$got" = "$DSPI_LINK_TREE" ] || { echo "web.sh: upstream tree $got, expected $DSPI_LINK_TREE" >&2; exit 1; }
  rm -rf "$up"; mkdir -p "$up"
  git -C "$DSPI_LINK_REPO" archive "$DSPI_LINK_COMMIT" js/packages/dspi-link/src | tar -x -C "$up"
  link=(-v "$up":/upstream:ro,z -e DSPI_LINK_SRC=/upstream/js/packages/dspi-link/src)
else
  echo "web.sh: no clone at $DSPI_LINK_REPO: the link loss steps are skipped" >&2
fi

cmd=$("$root/ci/command-name.sh")
podman run --rm --init --userns=keep-id --user "$(id -u):$(id -g)" --ipc=private --shm-size=1g \
  -v "$root":/src:ro,z \
  -v "$work/node_modules":/src/apps/web/node_modules:z \
  -v "$work/dist":/src/apps/web/dist:z \
  -v "$work/results":/results:z -e PW_OUTPUT_DIR=/results/shots-out \
  -v "$work/hub":/hub:ro,z -e DSPI_TERM="/hub/$cmd" \
  -v "$out":/shots:z -e SHOTS_DIR=/shots "${link[@]}" \
  -v "$work/home":/home/web:z -v "$work/tmp":/wtmp:z -e HOME=/home/web -e TMPDIR=/wtmp \
  -e SHOTS_ONLY="${SHOTS_ONLY:-}" -e CI=1 -w /src/apps/web "$PLAYWRIGHT" \
  npx playwright test --config playwright.shots.config.ts ${PW_ARGS:-}
