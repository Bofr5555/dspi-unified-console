#!/bin/bash
# -- WP CIPAR -- The full CI in parallel lanes: every step of ci/check.sh --all,
# and the qt-packages row (ci/release-desktop.sh --test) as the release run
# has it. Steps in a lane run one after another; lanes run side by side, each
# with its own cargo target and work directory (DSPI_TARGET_KEY, DSPI_WORK_KEY:
# ci/run.sh), so no lane waits on another's cargo lock.
#
#   ci/full.sh                     every lane
#   ci/full.sh --lanes rust,qt     only these lanes (ci/full.sh --list names them)
#   ci/full.sh --list              the lanes and their steps
#
# A step that fails stops nothing: its lane goes on with its next step and the
# other lanes are not touched. At the end: a table of every step (lane, exit
# code, time, when it started) and the wall time; exits 1 if any step failed.
#
# The budget (the host is shared):
#   DSPI_CI_LANES   lanes running at once (default 8: all; the longest start first)
#   DSPI_CI_JOBS    cargo jobs over all running lanes (default 24): each lane's
#                   DSPI_JOBS is DSPI_CI_JOBS / DSPI_CI_LANES (at least 2), the
#                   proptest lane twice that (its build is the critical path)
#   DSPI_TEST_PAR   test binaries at once in test and proptest (default 2 here, as
#                   the other lanes load the host too; ci/check.sh alone uses 4)
#   DSPI_SMOKE_SHARDS  qt-offscreen app instances (default 4) plus its launches
#   Gradle steps take a slot of ~/.cache/dspi-build/work/gradle-slot.sh when it exists.
# DSPI_CI_SEED=<target dir name>: a lane whose target directory does not exist
# yet starts from a reflink copy of $DSPI_BUILD_CACHE/target/<name> (and
# <name>-cross for the cross lane) instead of from nothing.
#
# Logs, summary.txt and summary.md: $DSPI_BUILD_CACHE/work/ci-full/<commit>-<time>/
# (never tmpfs). qt-packages builds from a commit: run it on a clean tree; its
# files end up in that directory too (release-<version>-ci), never in
# work/release-<version>, and it will not run while that folder exists.
set -uo pipefail
cd "$(dirname "$0")/.."
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}

# name | target/work key suffix (- : the branch's own) | steps, in order.
# Grouped by what they build: steps in a lane reuse each other's artefacts.
lanes=(
  "prop|prop|proptest"
  "pkgs|-|qt-packages"
  "android|-|android-tests android-aab"
  "qt|qt|qt qt-ts terminal qt-offscreen"
  "cross|x|windows-msvc android wasm wasm-core web"
  "rust|-|fetch fmt clippy test windows-gnu drift kotlin swift"
  "misc|misc|ios-check licences deny android-contract firmware-digests conformance-ts mcp-conformance"
  "qtwin|qtwin|qt-windows"
)

only=""
case "${1:-}" in
  "") ;;
  --lanes) only=",${2:?--lanes needs lane names},"
    for w in ${2//,/ }; do
      found=""; for l in "${lanes[@]}"; do [ "${l%%|*}" != "$w" ] || found=1; done
      [ -n "$found" ] || { echo "--lanes: unknown lane '$w' (ci/full.sh --list)" >&2; exit 2; }
    done ;;
  --list) for l in "${lanes[@]}"; do IFS='|' read -r n _ s <<<"$l"; printf '%-8s %s\n' "$n" "$s"; done; exit 0 ;;
  -h|--help) sed -n '2,/^set -uo/p' "$0" | sed '$d'; exit 0 ;;
  *) echo "unknown argument $1" >&2; exit 2 ;;
esac

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
branch=${branch//\//-}
commit=$(git rev-parse --short=12 HEAD)
[ -z "$(git status --porcelain)" ] || echo "full.sh: the tree has uncommitted changes; qt-packages will refuse to build" >&2
max=${DSPI_CI_LANES:-8}
jobs=$(( ${DSPI_CI_JOBS:-24} / max )); [ "$jobs" -ge 2 ] || jobs=2
out=$B/work/ci-full/$commit-$(date +%Y%m%d-%H%M%S)
mkdir -p "$out/logs"
sum=$out/summary.txt
slot=$B/work/gradle-slot.sh
export DSPI_TEST_PAR=${DSPI_TEST_PAR:-2} DSPI_SMOKE_SHARDS=${DSPI_SMOKE_SHARDS:-4}
started=$(date +%s)
echo "full.sh: commit $commit, $max lanes at once, DSPI_JOBS $jobs per lane (proptest $((jobs * 2))), DSPI_TEST_PAR $DSPI_TEST_PAR, DSPI_SMOKE_SHARDS $DSPI_SMOKE_SHARDS; logs $out" | tee "$sum"

# qt-packages: build-release.sh empties and fills work/release-<version>, the
# release folder's name. A folder already there is never touched (the step
# fails instead); the one this run makes moves into the run's directory.
qt_packages() {
  local v
  v=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' apps/desktop/qt/CMakeLists.txt)
  if [ -e "$B/work/release-$v" ]; then
    echo "full.sh: $B/work/release-$v is there (a release's files?); move it away to run qt-packages" >&2
    return 1
  fi
  ci/release-desktop.sh --test
  local rc=$?
  [ ! -d "$B/work/release-$v" ] || mv "$B/work/release-$v" "$out/release-$v-ci"
  return "$rc"
}

# One lane: its steps in order, each logged; a failure goes on to the next
lane() {
  local name=$1 key=$2 steps=$3 s log t0 rc
  if [ "$key" != - ]; then
    export DSPI_TARGET_KEY=$branch-$key DSPI_WORK_KEY=$branch-$key
    # The lanes whose builds a branch's target has (proptest's profile and the
    # MinGW release are in none)
    local x=""
    [ "$name" != cross ] || x=-cross
    case "$name" in
      misc|qt|cross)
        if [ -n "${DSPI_CI_SEED:-}" ] && [ ! -e "$B/target/$DSPI_TARGET_KEY$x" ] && [ -d "$B/target/$DSPI_CI_SEED$x" ]; then
          cp -a --reflink=always "$B/target/$DSPI_CI_SEED$x" "$B/target/$DSPI_TARGET_KEY$x" &&
            echo "SEED $name $B/target/$DSPI_TARGET_KEY$x from $DSPI_CI_SEED$x" >> "$sum"
        fi ;;
    esac
  fi
  export DSPI_JOBS=$jobs
  [ "$name" != prop ] || DSPI_JOBS=$((jobs * 2))
  for s in $steps; do
    log=$out/logs/$s.log
    t0=$(date +%s)
    echo "START $name $s +$((t0 - started))s" >> "$sum"
    case "$s" in
      qt-packages) qt_packages ;;
      android-tests|android-aab) if [ -x "$slot" ]; then "$slot" ci/check.sh --step "$s"; else ci/check.sh --step "$s"; fi ;;
      *) ci/check.sh --step "$s" ;;
    esac > "$log" 2>&1 < /dev/null
    rc=$?
    echo "END $name $s rc=$rc dur=$(( $(date +%s) - t0 ))s start=+$((t0 - started))s log=$log" >> "$sum"
    [ "$rc" = 0 ] || echo "full.sh: $name/$s FAILED (exit $rc), log $log"
  done
  echo "LANEDONE $name +$(( $(date +%s) - started ))s" >> "$sum"
}

running=0
for l in "${lanes[@]}"; do
  IFS='|' read -r name key steps <<<"$l"
  [ -z "$only" ] || [[ "$only" == *",$name,"* ]] || continue
  if [ "$running" -ge "$max" ]; then wait -n; running=$((running - 1)); fi
  echo "full.sh: lane $name: $steps"
  ( lane "$name" "$key" "$steps" ) &
  running=$((running + 1))
done
wait
wall=$(( $(date +%s) - started ))

{
  echo "# Full CI: commit $commit, branch $branch"
  echo
  echo "| Lane | Step | Result | Time | Started |"
  echo "|---|---|---|---|---|"
  while read -r _ lane step rc dur start _; do
    rc=${rc#rc=}; [ "$rc" = 0 ] && r=OK || r="FAILED ($rc)"
    echo "| $lane | $step | $r | ${dur#dur=} | ${start#start=} |"
  done < <(grep '^END ' "$sum")
  echo
  grep '^LANEDONE ' "$sum" | awk '{printf "%s lane ended at %s; ", $2, $3}'
  echo
  echo
  echo "Wall time $wall s ($((wall / 60)) min). Lanes at once $max, DSPI_JOBS $jobs per lane. Logs: $out/logs"
} > "$out/summary.md"
cat "$out/summary.md"
grep -q '^END .* rc=[^0]' "$sum" && exit 1
exit 0
