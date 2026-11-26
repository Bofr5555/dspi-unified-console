#!/bin/bash
# -- WP INTE -- The build caches a worktree leaves behind, for when it is removed.
#
#   tools/prune-worktree.sh [--work] [--yes] [--key KEY] <worktree>
#   tools/prune-worktree.sh --orphans [--yes]
#
# For <worktree> (a path; it may be gone already when --key names its key):
#   - the podman volume dspi-android-target-<first 10 hex of sha256 of "<worktree>/apps/android">
#     (apps/android/tools/dev.sh's Rust target per worktree);
#   - ~/.cache/dspi-build/target/<key> and its lanes' <key>-cross, -prop, -qt, -qtwin, -misc, -x,
#     -x-cross (ci/run.sh, ci/full.sh);
#   - with --work also work/<key>, its lanes' work/<key>-<lane> and work/web/<key>.
# <key> is what ci/run.sh names the worktree's caches by: its branch with "/" as "-", or its short
# commit when detached. Only those exact names are taken, never a pattern: wp-WEB's caches are not
# wp-WEB6's. A key that another worktree in `git worktree list` has is refused.
# --orphans lists the dspi-android-target-* volumes that match no `git worktree list` entry (and
# with --yes removes them). Without --yes nothing is removed: it prints what it would remove.
# DSPI_BUILD_CACHE overrides ~/.cache/dspi-build; PODMAN the podman command.
set -euo pipefail
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
PODMAN=${PODMAN:-podman}
LANES=(cross prop qt qtwin misc x x-cross)
usage() { sed -n '4,5p' "$0" | sed 's/^# *//' >&2; exit 2; }

yes=0 work=0 orphans=0 key="" wt=""
while [ $# -gt 0 ]; do
  case "$1" in
    --yes) yes=1 ;;
    --work) work=1 ;;
    --orphans) orphans=1 ;;
    --key) key=${2:?--key needs a value}; shift ;;
    -h|--help) usage ;;
    -*) echo "prune-worktree: unknown option $1" >&2; usage ;;
    *) [ -z "$wt" ] || usage; wt=$1 ;;
  esac
  shift
done

# The volume's name for a worktree path (as dev.sh makes it)
volume_of() { printf 'dspi-android-target-%s' "$(printf '%s' "$1/apps/android" | sha256sum | cut -c1-10)"; }
# Every worktree of this repository (absolute paths)
worktrees() { git -C "$(dirname "$0")" worktree list --porcelain | sed -n 's/^worktree //p'; }
# A worktree's cache key (ci/run.sh)
key_of() {
  local b
  b=$(git -C "$1" rev-parse --abbrev-ref HEAD 2>/dev/null) || return 1
  [ "$b" != HEAD ] || b=$(git -C "$1" rev-parse --short HEAD)
  printf '%s' "${b//\//-}"
}
rmvol() { "$PODMAN" volume rm "$1" >/dev/null; }
act() { # act <what> <command…>: do it with --yes, else say it
  local what=$1; shift
  if [ "$yes" = 1 ]; then echo "remove $what"; "$@"; else echo "would remove $what"; fi
}

if [ "$orphans" = 1 ]; then
  [ -z "$wt" ] && [ -z "$key" ] && [ "$work" = 0 ] || usage
  declare -A known=()
  while IFS= read -r p; do known[$(volume_of "$p")]=1; done < <(worktrees)
  n=0
  while IFS= read -r v; do
    [ -n "$v" ] || continue
    [ -z "${known[$v]:-}" ] || continue
    n=$((n + 1))
    act "podman volume $v" rmvol "$v"
  done < <("$PODMAN" volume ls --format '{{.Name}}' | grep '^dspi-android-target-' || true)
  echo "prune-worktree: $n orphaned android-target volume(s)"
  exit 0
fi

[ -n "$wt" ] || usage
case "$wt" in /*) ;; *) wt=$(cd "$(dirname "$wt")" 2>/dev/null && pwd)/$(basename "$wt") ;; esac
wt=${wt%/}
if [ -z "$key" ]; then
  key=$(key_of "$wt") || { echo "prune-worktree: $wt is not a git worktree; name its key with --key" >&2; exit 2; }
fi
case "$key" in ""|.|..|*/*) echo "prune-worktree: bad key '$key'" >&2; exit 2 ;; esac
# Another worktree's caches are not this one's
while IFS= read -r p; do
  [ "${p%/}" != "$wt" ] || continue
  k=$(key_of "$p" 2>/dev/null) || continue
  for l in "" "${LANES[@]/#/-}"; do
    if [ "$k$l" = "$key" ] || [ "$k" = "$key$l" ]; then
      echo "prune-worktree: key $key is (a lane of) the worktree $p's; nothing removed" >&2
      exit 1
    fi
  done
done < <(worktrees)

vol=$(volume_of "$wt")
if "$PODMAN" volume exists "$vol" 2>/dev/null; then act "podman volume $vol" rmvol "$vol"
else echo "no podman volume $vol"; fi
dirs=("$B/target/$key")
for l in "${LANES[@]}"; do dirs+=("$B/target/$key-$l"); done
if [ "$work" = 1 ]; then
  dirs+=("$B/work/$key" "$B/work/web/$key")
  for l in "${LANES[@]}"; do dirs+=("$B/work/$key-$l"); done
fi
for d in "${dirs[@]}"; do
  [ -e "$d" ] || continue
  act "$d" rm -rf -- "$d"
done
[ "$yes" = 1 ] || echo "prune-worktree: dry run; --yes removes"
