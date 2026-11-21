#!/bin/bash
# -- WP QTMAC -- The Qt app's macOS release file, made in the macOS VM from a
# clean commit, from the host:
#
#   apps/desktop/packaging/macos/build-macos.sh [--no-test] [output dir]
#
# Sends HEAD (git archive: the commit, nothing uncommitted) to the VM
# ($DSPI_MACVM, default macvm), runs make-dmg.sh there (Qt, CMake and Ninja from
# fetch-qt.sh's pins, kept in ~/qtmac-tools; the core for x86_64 and arm64;
# universal, ad hoc signed, not notarised), then test-dmg.sh inside the VM's
# GUI session (skipped with --no-test), and copies
# uConsole-for-DSPi-<version>-macos.dmg back. Output dir: default
# $DSPI_BUILD_CACHE/work/release-<version>-macos (never tmpfs); a file of that
# name already there is never replaced. The VM keeps the tree, the build and
# cargo's target under ~/qtmac-build/<commit>; it holds ci/macvm.sh's lock
# (~/work/matrix/lock) meanwhile, so one heavy job runs there at a time.
# Exits 3 when the VM cannot be reached or nobody is logged in to its GUI.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
test=1
if [ "${1:-}" = --no-test ]; then test=0; shift; fi
version=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$here/../../qt/CMakeLists.txt")
[ -n "$version" ] || { echo "build-macos: no version in qt/CMakeLists.txt" >&2; exit 1; }
out=${1:-$B/work/release-$version-macos}
file=uConsole-for-DSPi-$version-macos.dmg
[ -z "$(git -C "$repo" status --porcelain)" ] || { echo "build-macos: the work tree has changes; a release is built from a commit" >&2; exit 1; }
[ ! -e "$out/$file" ] || { echo "build-macos: $out/$file is there already; nothing replaced" >&2; exit 1; }
commit=$(git -C "$repo" rev-parse --short=12 HEAD)
host=${DSPI_MACVM:-macvm}
vm() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }
who=$(vm who </dev/null 2>&1) || { echo "build-macos: $host is not reachable: $who" >&2; exit 3; }
[ "$test" = 0 ] || grep -qw console <<<"$who" || { echo "build-macos: nobody is logged in to the GUI of $host" >&2; exit 3; }

echo "build-macos: $commit to $host:~/qtmac-build/$commit"
git -C "$repo" archive --format=tar HEAD |
  vm "set -u
    w=~/qtmac-build/$commit
    mkdir -p ~/work/matrix
    while ! mkdir ~/work/matrix/lock 2>/dev/null; do
      pid=\$(cat ~/work/matrix/lock/pid 2>/dev/null)
      if [ -n \"\$pid\" ] && ! kill -0 \"\$pid\" 2>/dev/null; then rm -rf ~/work/matrix/lock; continue; fi
      echo \"build-macos: waiting for the run holding ~/work/matrix/lock (pid \$pid)\"; sleep 30
    done
    echo \$\$ >~/work/matrix/lock/pid
    rm -rf \$w/src \$w/out && mkdir -p \$w/src && tar -xf - -C \$w/src || { rm -rf ~/work/matrix/lock; exit 1; }
    PATH=\"\$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin\" CARGO_TARGET_DIR=\$w/target \\
      nice -n 10 \$w/src/apps/desktop/packaging/macos/make-dmg.sh \$w/out </dev/null
    rc=\$?
    if [ \$rc = 0 ] && [ $test = 1 ]; then
      sudo launchctl asuser \"\$(id -u)\" sudo -u \"\$(id -un)\" env HOME=\"\$HOME\" PATH=/usr/bin:/bin:/usr/sbin:/sbin \\
        \$w/src/apps/desktop/packaging/macos/test-dmg.sh \$w/out/$file </dev/null
      rc=\$?
    fi
    rm -rf ~/work/matrix/lock
    exit \$rc"
mkdir -p "$out"
scp -q -o BatchMode=yes "$host:qtmac-build/$commit/out/$file" "$out/$file.part"
mv "$out/$file.part" "$out/$file"
echo "build-macos: $out/$file"
