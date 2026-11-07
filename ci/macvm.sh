#!/bin/bash
# The build matrix's Apple rows (WP SW2): copy the working tree into the macOS
# VM and run apps/apple/ci-test.sh there, inside the GUI session.
#
#   ci/macvm.sh macos   macOS Swift app: build, unit tests
#   ci/macvm.sh ios     iOS app: build, unit and UI tests in the Simulator
#
# Exits 3 when the VM cannot run it (no ssh, or nobody logged in to its GUI):
# ci/matrix.sh prints the row as "not run". One heavy job at a time in the VM:
# a lock directory there makes a second run wait for the first.
#
# DSPI_MACVM (default macvm) is the ssh host; the guest keeps the tree, cargo's
# target dir and the lock under ~/work/matrix.
set -uo pipefail
cd "$(dirname "$0")/.."
platform=${1:-}
case "$platform" in macos|ios) ;; *) echo "usage: $0 macos|ios" >&2; exit 2 ;; esac
host=${DSPI_MACVM:-macvm}
vm() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }

who=$(vm who </dev/null 2>&1) || { echo "macvm: $host is not reachable: $who"; exit 3; }
grep -qw console <<<"$who" || { echo "macvm: nobody is logged in to the GUI of $host"; exit 3; }

# One ssh session: take the lock, unpack the tree from stdin, run, unlock. The
# lock holds the session shell's pid, so a run that died (VM restart, lost ssh)
# leaves a lock the next run removes.
echo "macvm: copying the tree to $host:~/work/matrix/src and running ci-test.sh $platform"
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
      CARGO_TARGET_DIR=\"\$HOME/work/matrix/target\" \\
      nice -n 10 \"\$HOME/work/matrix/src/apps/apple/ci-test.sh\" $platform </dev/null
    rc=\$?
    rm -rf ~/work/matrix/lock
    exit \$rc"
