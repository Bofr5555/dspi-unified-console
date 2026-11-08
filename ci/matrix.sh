#!/bin/bash
# One commit, one command: build and test every client, then print the matrix.
# This is M0's gate (docs/core-merge/PLAN-CLIENTS.md, section 0.2).
#
#   ci/matrix.sh                run every row (ci/check.sh --all-clients is the same)
#   ci/matrix.sh --only a,b     only the rows with these ids; an unknown id fails
#   ci/matrix.sh --list         print the row ids
#
# Runs from a clean checkout: missing images are built from their committed
# Containerfiles, the Rust cache is filled.
# A failing row does not stop the others. Exits 0 only if no row ran and failed;
# "not run" rows are the VM's and do not fail the run. The VM rows (ci/macvm.sh)
# are "not run" when the macOS VM is down or nobody is logged in to its GUI.
#
# Logs and the printed matrix: $DSPI_BUILD_CACHE/work/matrix/<commit>/ (never tmpfs).
set -uo pipefail
cd "$(dirname "$0")/.."
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}

# id | client | where | ci/check.sh steps (empty: not run, with the reason in place of a command)
rows=(
  "images|All images, from their Containerfiles|podman|=ci/container/build-images.sh"
  "fetch|Rust crates, every target|container|fetch"
  "fmt|Workspace: rustfmt|container|fmt"
  "clippy|Workspace: clippy|container|clippy"
  "test|Workspace: tests|container|test"
  "proptest|Workspace: property tests|container|proptest"
  "deny|Licences: cargo-deny|container|deny"
  "licences|Licences: iOS inventory|container|licences"
  "linux-qt6|Linux Qt 6|container|qt"
  "linux-qt6-demo|Linux Qt 6, demo device, offscreen|container|qt-offscreen"
  "linux-qt5|Linux Qt 5|container|qt5"
  "linux-qt5-demo|Linux Qt 5, demo device, offscreen|container|qt5-offscreen"
  "linux-terminal|Linux Terminal|container|terminal"
  "windows-qt|Windows Qt (MinGW), zip and installer|container|qt-windows"
  "windows-gnu|Windows workspace (GNU)|container|windows-gnu"
  "windows-terminal|Windows core and Terminal (MSVC)|xwin, container|windows-msvc"
  "android-libs|Android core libraries, three ABIs|cross container|android"
  # -- WP MR3 --
  "android-tests|Android modules: contract, lint, unit tests, assemble|Gradle container|android-tests"
  "android-aab|Android AAB|Gradle container|android-aab"
  "wasm|wasm check|cross container|wasm"
  # -- WP WB1 --
  "wasm-core|wasm: session and core, scenario under Node (JSPI)|cross container|wasm-core"
  "ios-check|iOS check (no link)|container|ios-check"
  # -- WP WB5 --
  "web|Web client on the wasm core's demo, Chromium and Firefox|container|web"
  "kotlin|Kotlin bindings compile|container|kotlin"
  "swift|Swift bindings compile (Linux swiftc)|container|swift"
  "drift|Generated code matches its sources|container|drift"
  # -- WP SW2 -- Not run (exit 3) when the VM is down or has no GUI login.
  "macos-swift|macOS Swift app|macvm|=ci/macvm.sh macos"
  "ios-sim|iOS Simulator|macvm|=ci/macvm.sh ios"
  # -- WP QD21b -- built, then started from the .dmg on the demo device
  "qt-dmg|Qt .dmg, demo device|macvm|=ci/macvm.sh qt-dmg"
)

only=""
case "${1:-}" in
  "") ;;
  --only) only=",${2:?--only needs row ids},"; ;;
  --list) for r in "${rows[@]}"; do echo "${r%%|*}"; done; exit 0 ;;
  -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
  *) echo "unknown argument $1" >&2; exit 2 ;;
esac

# -- WP MR3 -- A mistyped id must not pass by running nothing.
if [ -n "$only" ]; then
  IFS=, read -ra wanted <<<"$2"
  [ ${#wanted[@]} -gt 0 ] || { echo "--only: no row ids in '$2'" >&2; exit 2; }
  for w in "${wanted[@]}"; do
    found=""
    for r in "${rows[@]}"; do [ "${r%%|*}" != "$w" ] || found=1; done
    [ -n "$found" ] || { echo "--only: unknown row id '$w' (ci/matrix.sh --list)" >&2; exit 2; }
  done
fi

commit=$(git rev-parse --short=12 HEAD)
state=""
[ -z "$(git status --porcelain)" ] || state=" (with uncommitted changes)"
logs=$B/work/matrix/$commit
mkdir -p "$logs"
report=$logs/matrix.md
results=()
failed=0
started=$SECONDS

for r in "${rows[@]}"; do
  IFS='|' read -r id client where steps <<<"$r"
  [ -z "$only" ] || [[ "$only" == *",$id,"* ]] || continue
  log=$logs/$id.log
  start=$SECONDS
  case "$steps" in
    -*) results+=("$client|$where|not run|${steps#-}|"); echo "-- $id: not run"; continue ;;
    =*) read -ra cmd <<<"${steps#=}" ;;  # -- WP SW2 -- a command with arguments
    *) cmd=(ci/check.sh); for s in $steps; do cmd+=(--step "$s"); done ;;
  esac
  echo "-- $id: ${cmd[*]}"
  if "${cmd[@]}" >"$log" 2>&1 </dev/null; then
    result=OK
  elif [ $? = 3 ] && [ "$where" = macvm ]; then
    # -- WP SW2 -- the VM could not run it: its reason, not a failure
    results+=("$client|$where|not run|$(tail -n 1 "$log")|$((SECONDS - start)) s")
    echo "-- $id: not run"
    continue
  else
    result=FAILED
    failed=1
    echo "-- $id: FAILED, log $log"
  fi
  results+=("$client|$where|$result||$((SECONDS - start)) s")
done

{
  echo "# Build matrix: commit $commit$state"
  echo
  echo "| Client | Where | Result | Time |"
  echo "|---|---|---|---|"
  for x in "${results[@]}"; do
    IFS='|' read -r client where result note time <<<"$x"
    echo "| $client | $where | $result${note:+ $note} | $time |"
  done
  echo
  echo "Total $((SECONDS - started)) s. Logs: $logs"
} | tee "$report"
exit "$failed"
