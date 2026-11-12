#!/bin/bash
# The build matrix's Apple rows (WP SW2): copy the working tree into the macOS
# VM and run apps/apple/ci-test.sh there, inside the GUI session.
#
#   ci/macvm.sh macos   macOS Swift app: build, unit tests
#   ci/macvm.sh ios     iOS app: build, unit and UI tests in the Simulator
#   ci/macvm.sh swift-dmg  Swift app: its .dmg (apps/apple/ci-dmg.sh), started
#                       from the .dmg on the demo device, copied back the same
#                       way (WP REL013M)
#
# Exits 3 when the VM cannot run it (no ssh, or nobody logged in to its GUI):
# ci/matrix.sh prints the row as "not run". One heavy job at a time in the VM:
# a lock directory there makes a second run wait for the first.
#
# DSPI_MACVM (default macvm) is the ssh host; the guest keeps the tree (src),
# cargo's target dir, Xcode's output and results (out) and the lock under
# ~/work/matrix.
set -uo pipefail
cd "$(dirname "$0")/.."
platform=${1:-}
case "$platform" in
  macos|ios) script="apps/apple/ci-test.sh $platform" ;;
  # -- WP QP0 -- The Qt .dmg row is dropped: the Swift app is the Mac release
  swift-dmg) script=apps/apple/ci-dmg.sh ;;  # -- WP REL013M --
  *) echo "usage: $0 macos|ios|swift-dmg" >&2; exit 2 ;;
esac
host=${DSPI_MACVM:-macvm}
vm() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }

who=$(vm who </dev/null 2>&1) || { echo "macvm: $host is not reachable: $who"; exit 3; }
grep -qw console <<<"$who" || { echo "macvm: nobody is logged in to the GUI of $host"; exit 3; }

# One ssh session: take the lock, unpack the tree from stdin, run, unlock. The
# lock holds the session shell's pid, so a run that died (VM restart, lost ssh)
# leaves a lock the next run removes.
echo "macvm: copying the tree to $host:~/work/matrix/src and running $script"
# Tracked and untracked files, without ignored ones: what the container rows see.
git ls-files -z --cached --others --exclude-standard |
  tar --null --ignore-failed-read -T - -cf - 2>/dev/null |
  vm "set -u
    mkdir -p ~/work/matrix
    while ! mkdir ~/work/matrix/lock 2>/dev/null; do
      pid=\$(cat ~/work/matrix/lock/pid 2>/dev/null)
      if [ -n \"\$pid\" ] && ! kill -0 \"\$pid\" 2>/dev/null; then rm -rf ~/work/matrix/lock; continue; fi
      echo \"macvm: waiting for the run holding ~/work/matrix/lock (pid \$pid)\"; sleep 30
    done
    echo \$\$ >~/work/matrix/lock/pid
    rm -rf ~/work/matrix/src && mkdir -p ~/work/matrix/src && tar -xf - -C ~/work/matrix/src || { rm -rf ~/work/matrix/lock; exit 1; }
    sudo launchctl asuser \"\$(id -u)\" sudo -u \"\$(id -un)\" env \\
      PATH=\"\$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin\" HOME=\"\$HOME\" \\
      CARGO_TARGET_DIR=\"\$HOME/work/matrix/target\" CI_OUT=\"\$HOME/work/matrix/out\" \\
      nice -n 10 \$HOME/work/matrix/src/$script </dev/null
    rc=\$?
    rm -rf ~/work/matrix/lock
    exit \$rc"
rc=$?
# -- WP QD21b -- the .dmg, an unpublished artefact, next to the matrix logs
if [ "${platform%-dmg}" != "$platform" ] && [ "$rc" = 0 ]; then   # -- WP REL013M -- swift-dmg too
  dest=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/matrix/$(git rev-parse --short=12 HEAD)
  mkdir -p "$dest"
  scp -q -o BatchMode=yes "$host:work/matrix/out/$platform/DSPi-uConsole-*-macos.dmg" "$dest/" || rc=1
  echo "macvm: the .dmg is in $dest"
fi
exit "$rc"
