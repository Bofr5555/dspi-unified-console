#!/bin/bash
# Run a command in a build container, for the worktree this script is in.
#
#   ci/run.sh 'cargo test -p dspi-proto'
#   DSPI_PROFILE=cross ci/run.sh 'cargo xwin build -p dspi-core --target x86_64-pc-windows-msvc'
#
# Profiles:
#   default  localhost/dspi-coremerge:3 (ci/container/Containerfile): Linux,
#            Windows GNU cross, Qt 5/6, rustfmt, clippy, ALSA
#   cross    localhost/dspi-termport-xwin:1: cargo-xwin (MSVC; the owner accepted
#            Microsoft's CRT/SDK licence), Android NDK r27 + cargo-ndk, wasm32
#
# Never uses tmpfs (/tmp, /dev/shm, a session scratchpad): everything lives
# under ~/.cache/dspi-build (override with DSPI_BUILD_CACHE):
#   target/<branch>[-cross]/  CARGO_TARGET_DIR per branch (/target)
#   cargo-home/               shared registry and git caches
#   xwin-cache/               the MSVC CRT/SDK (cross profile)
#   work/<branch>/            scratch (/work; HOME and TMPDIR live there)
# SRC overrides the source tree (default: this worktree). DSPI_JOBS caps
# cargo's parallel jobs per container (default 4; several WPs build at once). No USB devices are
# visible in the container.
set -euo pipefail
here=${SRC:-$(cd "$(dirname "$0")/.." && pwd)}
branch=$(git -C "$here" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
branch=${branch//\//-}
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
profile=${DSPI_PROFILE:-default}
mkdir -p "$B/cargo-home/registry" "$B/cargo-home/git" "$B/work/$branch/home" \
         "$B/work/$branch/tmp" "$B/work/empty" "$B/xwin-cache"
common=(--rm -i --userns=keep-id
  -v "$here":/src:z -v "$B/work/$branch":/work:z
  -v "$B/work/empty":/sys/bus/usb/devices:ro,z
  -e HOME=/work/home -e TMPDIR=/work/tmp -e CARGO_BUILD_JOBS="${DSPI_JOBS:-4}" -w /src)
case "$profile" in
  default)
    mkdir -p "$B/target/$branch"
    exec podman run "${common[@]}" -v "$B/target/$branch":/target:z \
      -v "$B/cargo-home/registry":/opt/cargo/registry:z -v "$B/cargo-home/git":/opt/cargo/git:z \
      -e CARGO_TARGET_DIR=/target \
      "${DSPI_IMAGE:-localhost/dspi-coremerge:3}" bash -c "$*" ;;
  cross)
    mkdir -p "$B/target/$branch-cross" "$B/cargo-home-cross"
    exec podman run "${common[@]}" -v "$B/target/$branch-cross":/target:z \
      -v "$B/cargo-home/registry":/cache/cargo/registry:z -v "$B/cargo-home/git":/cache/cargo/git:z \
      -v "$B/xwin-cache":/xwin:z \
      -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/target -e RUSTUP_HOME=/opt/rustup \
      -e XWIN_CACHE_DIR=/xwin -e ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.2.12479018 \
      -e ANDROID_HOME=/opt/android-sdk \
      -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
      "${DSPI_IMAGE:-localhost/dspi-termport-xwin:1}" bash -c "$*" ;;
  *) echo "unknown DSPI_PROFILE $profile" >&2; exit 2 ;;
esac
