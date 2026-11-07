#!/usr/bin/env bash
# Run a command inside the pinned build container (container/Containerfile).
#
#   tools/dev.sh <cmd...>          run <cmd> in the container, cwd preserved
#   tools/dev.sh                   interactive shell
#   tools/dev.sh --build-image     (re)build the image
#   tools/dev.sh --print-image     print the image reference
#
# Nothing is installed on the host. The whole monorepo checkout (or worktree) is
# bind-mounted at its host path, not only apps/android, because the Rust workspace
# (Cargo.toml, crates/) is at its top; caches live in NAMED podman volumes:
#   dspi-android-cargo    CARGO_HOME (crates.io registry/git cache)
#   dspi-android-gradle   shared wrapper dists and downloaded dependencies
#   dspi-android-target-<hash of the app's path>   CARGO_TARGET_DIR (per worktree);
#                                                  also TMPDIR, so rustc and mktemp
#                                                  stay off any tmpfs, and
#                                                  GRADLE_USER_HOME, see below
# DSPI_JOBS caps cargo's parallel jobs per container (default 4), as ci/run.sh does.
#
# Gradle home per checkout: Gradle asks a lock's holder to let go over UDP on
# localhost, and a holder in another container cannot hear it, so a shared home
# fails with "Timeout waiting to lock journal cache" when runs overlap. Downloads
# stay shared: the shared volume is the read-only GRADLE_RO_DEP_CACHE, and the
# wrapper, which only takes file locks, is a link to it. What a checkout finds
# missing there it downloads for itself. To add it for everyone, with no other
# Gradle running, use the shared volume as the home for one run:
#   DSPI_GRADLE_SHARED_HOME=1 tools/dev.sh ./gradlew <tasks>
# Two Gradle runs in one checkout still collide (they share build/ as well).
#
# HARD RULE: the container gets no USB/hardware access. No --device, no
# --privileged, no /dev/bus/usb. Do not add any. An empty directory covers
# /sys/bus/usb/devices (as in ci/run.sh), so nothing in here can even list the
# host's USB bus; the core's enumeration, and its tests, rely on that.
set -euo pipefail

IMAGE_TAG="2026-11-01-play"
IMAGE="localhost/dspi-android-build:${IMAGE_TAG}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
TOP="$(git -C "$ROOT" rev-parse --show-toplevel)"

case "${1:-}" in
  --build-image)
    exec podman build -t "$IMAGE" -f "$ROOT/container/Containerfile" "$ROOT/container" ;;
  --print-image)
    echo "$IMAGE"; exit 0 ;;
esac

# Already inside the container: just run it.
if [[ "${DSPI_IN_CONTAINER:-}" == 1 ]]; then
  if [[ $# -eq 0 ]]; then exec bash; fi
  exec "$@"
fi

if ! podman image exists "$IMAGE"; then
  echo "dev.sh: image $IMAGE missing; building it (tools/dev.sh --build-image)" >&2
  podman build -t "$IMAGE" -f "$ROOT/container/Containerfile" "$ROOT/container" >&2
fi

# The image's user is 1000 (its /cache is owned by that uid): the host user becomes
# that uid, so bind mounts and volumes both belong to the host user, whatever its uid.
mounts=(-v "$TOP:$TOP:z")
# A git worktree keeps its metadata in the main repo's .git; mount that too so
# git works inside the container.
common="$(git -C "$TOP" rev-parse --path-format=absolute --git-common-dir)"
case "$common/" in
  "$TOP"/*) ;;
  *) mounts+=(-v "$common:$common:z") ;;
esac

empty="${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/empty"
mkdir -p "$empty"
mounts+=(-v "$empty:/sys/bus/usb/devices:ro,z")

wt_hash="$(printf '%s' "$ROOT" | sha256sum | cut -c1-10)"
mounts+=(
  -v dspi-android-cargo:/cache/cargo
  -v dspi-android-gradle:/cache/gradle
  -v "dspi-android-target-${wt_hash}:/cache/target"
)

workdir="$PWD"
case "$workdir/" in
  "$ROOT"/*) ;;
  *) workdir="$ROOT" ;;
esac

gradle_env=(-e GRADLE_USER_HOME=/cache/target/gradle-home -e GRADLE_RO_DEP_CACHE=/cache/gradle/caches)
if [[ -n "${DSPI_GRADLE_SHARED_HOME:-}" ]]; then
  gradle_env=(-e GRADLE_USER_HOME=/cache/gradle)
fi

tty=()
if [[ -t 0 && -t 1 ]]; then tty=(-t); fi

# DSPI_DEV_PUBLISH=hostport:containerport publishes one TCP port (only the
# DSPi Link fake hub uses it: tools/link-fakehub.sh). Network only, no devices.
publish=()
if [[ -n "${DSPI_DEV_PUBLISH:-}" ]]; then publish=(-p "$DSPI_DEV_PUBLISH"); fi

if [[ $# -eq 0 ]]; then set -- bash; fi

exec podman run --rm -i "${tty[@]}" \
  --userns=keep-id:uid=1000,gid=1000 \
  --user 1000:1000 \
  --security-opt no-new-privileges \
  --cap-drop=all \
  "${mounts[@]}" \
  "${publish[@]}" \
  -w "$workdir" \
  -e TERM="${TERM:-dumb}" \
  -e CI="${CI:-}" \
  -e CARGO_BUILD_JOBS="${DSPI_JOBS:-4}" \
  -e TMPDIR=/cache/target/tmp \
  "${gradle_env[@]}" \
  "$IMAGE" bash -c '
    mkdir -p "$TMPDIR" /cache/gradle/wrapper "$GRADLE_USER_HOME"
    [[ -z "${GRADLE_RO_DEP_CACHE:-}" ]] || ln -sfn /cache/gradle/wrapper "$GRADLE_USER_HOME/wrapper"
    exec "$@"' dev.sh "$@"
