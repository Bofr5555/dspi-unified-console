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
# Xcode's output and results (out) and the lock under ~/work/matrix.
# -- WP SW-M1 -- Cargo's target dir is per tree: ~/work/matrix/targets/<tree>,
# <tree> = this worktree's directory name (DSPI_MACVM_TREE overrides it). One
# shared target let a run reuse another tree's newer core build, so its own
# older-mtime sources were never compiled (wp-P1SW3 round 2). Under the lock,
# a run removes target dirs unused for 7 days, keeps the 6 most recently
# used (a run marks its own before it waits for the lock), and removes the old shared ~/work/matrix/target once nothing in it has
# changed for 2 days (a run queued with an older copy of this script may still
# use it until then).
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
# -- WP SW-M1 -- the tree's key: letters, digits, dot, dash, underscore only
tree=${DSPI_MACVM_TREE:-$(basename "$(git rev-parse --show-toplevel)")}
tree=$(printf '%s' "$tree" | tr -c 'A-Za-z0-9._-' '_')
case "$tree" in ''|.|..) tree=default ;; esac   # never the targets dir itself or its parent
vm() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }

who=$(vm who </dev/null 2>&1) || { echo "macvm: $host is not reachable: $who"; exit 3; }
grep -qw console <<<"$who" || { echo "macvm: nobody is logged in to the GUI of $host"; exit 3; }

# One ssh session: take the lock, unpack the tree from stdin, run, unlock. The
# lock holds the session shell's pid, so a run that died (VM restart, lost ssh)
# leaves a lock the next run removes.
echo "macvm: copying the tree to $host:~/work/matrix/src and running $script (cargo target: targets/$tree)"
# Tracked and untracked files, without ignored ones: what the container rows see.
git ls-files -z --cached --others --exclude-standard |
  tar --null --ignore-failed-read -T - -cf - 2>/dev/null |
  vm "set -u
    mkdir -p ~/work/matrix
    # -- WP SW-M1 -- marked before waiting, so the running job's cleanup keeps a queued tree's target
    mkdir -p ~/work/matrix/targets/$tree && touch ~/work/matrix/targets/$tree/.last-used
    while ! mkdir ~/work/matrix/lock 2>/dev/null; do
      pid=\$(cat ~/work/matrix/lock/pid 2>/dev/null)
      if [ -n \"\$pid\" ] && ! kill -0 \"\$pid\" 2>/dev/null; then rm -rf ~/work/matrix/lock; continue; fi
      echo \"macvm: waiting for the run holding ~/work/matrix/lock (pid \$pid)\"; sleep 30
    done
    echo \$\$ >~/work/matrix/lock/pid
    # -- WP SW-M1 -- one cargo target dir per tree; stale ones are removed
    touch ~/work/matrix/targets/$tree/.last-used
    for d in ~/work/matrix/targets/*/; do
      d=\${d%/}; [ \"\${d##*/}\" = $tree ] && continue
      if [ ! -e \"\$d/.last-used\" ] || [ -n \"\$(find \"\$d/.last-used\" -mtime +7)\" ]; then
        echo \"macvm: removing the unused target dir \$d\"; rm -rf \"\$d\"
      fi
    done
    ls -1t ~/work/matrix/targets/*/.last-used 2>/dev/null | tail -n +7 | while read -r f; do
      d=\${f%/.last-used}; [ \"\${d##*/}\" = $tree ] && continue
      echo \"macvm: removing the old target dir \$d (more than 6)\"; rm -rf \"\$d\"
    done
    if [ -d ~/work/matrix/target ] && [ -z \"\$(find ~/work/matrix/target -mindepth 1 -maxdepth 3 -mtime -2 -print 2>/dev/null | head -n 1)\" ]; then
      echo 'macvm: removing the old shared target dir ~/work/matrix/target'; rm -rf ~/work/matrix/target
    fi
    rm -rf ~/work/matrix/src && mkdir -p ~/work/matrix/src && tar -xf - -C ~/work/matrix/src || { rm -rf ~/work/matrix/lock; exit 1; }
    sudo launchctl asuser \"\$(id -u)\" sudo -u \"\$(id -un)\" env \\
      PATH=\"\$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin\" HOME=\"\$HOME\" \\
      CARGO_TARGET_DIR=\"\$HOME/work/matrix/targets/$tree\" CI_OUT=\"\$HOME/work/matrix/out\" \\
      nice -n 10 \$HOME/work/matrix/src/$script </dev/null
    rc=\$?
    rm -rf ~/work/matrix/lock
    exit \$rc"
rc=$?
# -- WP QD21b -- the .dmg, an unpublished artefact, next to the matrix logs
if [ "${platform%-dmg}" != "$platform" ] && [ "$rc" = 0 ]; then   # -- WP REL013M -- swift-dmg too
  dest=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/matrix/$(git rev-parse --short=12 HEAD)
  mkdir -p "$dest"
  scp -q -o BatchMode=yes "$host:work/matrix/out/$platform/uConsole-for-DSPi-*-macos.dmg" "$dest/" || rc=1
  echo "macvm: the .dmg is in $dest"
fi
exit "$rc"
