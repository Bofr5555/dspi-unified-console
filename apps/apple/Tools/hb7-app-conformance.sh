#!/bin/bash
# WP SW-M3: HB7's Swift conformance client (crates/dspi-hub/tests/conformance/swift,
# with the Swift console's LinkClient, unmodified) against the Mac app's own DSPi Link
# hub: the Rust hub through its UniFFI wrapper (dspi_hub), hosted by the app's
# Core/CoreHub.swift, in the app's test host, on the hub's demo device. The done
# criterion of SW-M3.
#
#   apps/apple/Tools/hb7-app-conformance.sh            loopback only
#   apps/apple/Tools/hb7-app-conformance.sh --lan PORT  the app's hub on every interface
#                     on PORT, and the host reaches it from outside the guest through a
#                     QEMU port forward (the Local Network check; needs the VM's QMP socket)
#
# Copies the tree to the VM (DSPI_MACVM, default macvm) under ~/work/matrix/hb7-app/src
# and, under the matrix lock (~/work/matrix/lock, as ci/macvm.sh) in the GUI session:
# builds the core's XCFramework and the app for testing, compiles the client with swiftc
# (as run-clients.sh swift), then runs apps/apple/Tools/hb7_app_driver.py. Cargo's target
# is ci/macvm.sh's for this tree. Logs in ${DSPI_BUILD_CACHE:-~/.cache/dspi-build}/work/hb7-app/.
# Exit status: the client's (0: every scenario passed); 3 when the VM cannot run it.
set -euo pipefail
lan=
case "${1:-}" in
  "") ;;
  --lan) lan=${2:?--lan PORT}; [[ $lan =~ ^[0-9]+$ ]] || { echo "--lan PORT" >&2; exit 2; } ;;
  *) echo "usage: $0 [--lan PORT]" >&2; exit 2 ;;
esac
root=$(cd "$(dirname "$0")/../../.." && pwd)
host=${DSPI_MACVM:-macvm}
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
out=$B/work/hb7-app
mkdir -p "$out"
log=$out/run.log
: >"$log"
say() { echo "$*" | tee -a "$log"; }
vm() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }
tree=$(basename "$(git -C "$root" rev-parse --show-toplevel)" | tr -c 'A-Za-z0-9._-' '_')

who=$(vm who </dev/null 2>&1) || { say "hb7-app: $host is not reachable: $who"; exit 3; }
grep -qw console <<<"$who" || { say "hb7-app: nobody is logged in to the GUI of $host"; exit 3; }

say "hb7-app: copying the tree to $host:~/work/matrix/hb7-app/src"
(cd "$root" && git ls-files -z --cached --others --exclude-standard |
  tar --null --ignore-failed-read -T - -cf - 2>/dev/null) |
  vm 'rm -rf ~/work/matrix/hb7-app/src && mkdir -p ~/work/matrix/hb7-app/src && tar -xf - -C ~/work/matrix/hb7-app/src'

probe_pid=
if [ -n "$lan" ]; then
  # The host's loopback port to the guest's, through QEMU's user network: the
  # guest sees the peer as 10.0.2.2, a host on its network.
  qmp=${DSPI_MACVM_QMP:-$HOME/vm/reims-data/reims-ctl/vms/macr/qmp.sock}
  python3 - "$qmp" "$lan" <<'PY' | tee -a "$log"
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile("rw")
f.readline()
for cmd in ({"execute": "qmp_capabilities"},
            {"execute": "human-monitor-command",
             "arguments": {"command-line": f"hostfwd_add net0 tcp:127.0.0.1:{sys.argv[2]}-:{sys.argv[2]}"}}):
    f.write(json.dumps(cmd) + "\n"); f.flush(); print("qmp:", f.readline().strip())
PY
  # Once the app's hub is up: its web page from outside the guest.
  (
    # No time limit: the run may wait long for the lock; the probe is killed with it.
    until vm 'test -s ~/work/matrix/hb7-app/work/conf/hub.json' </dev/null 2>/dev/null; do sleep 3; done
    sleep 1
    code=$(curl -s -o "$out/lan-page.html" -w '%{http_code}' -m 10 "http://127.0.0.1:$lan/" || echo fail)
    say "hb7-app: LAN check: GET http://<guest>:$lan/ from 10.0.2.2: HTTP $code, $(cat "$out/lan-page.html" 2>/dev/null | wc -c) bytes"
  ) &
  probe_pid=$!
fi

say "hb7-app: waiting for ~/work/matrix/lock, then building and running in the GUI session"
set +e
vm "set -u
  mkdir -p ~/work/matrix/targets/$tree
  while ! mkdir ~/work/matrix/lock 2>/dev/null; do
    pid=\$(cat ~/work/matrix/lock/pid 2>/dev/null)
    if [ -n \"\$pid\" ] && ! kill -0 \"\$pid\" 2>/dev/null; then rm -rf ~/work/matrix/lock; continue; fi
    echo \"hb7-app: waiting for the run holding ~/work/matrix/lock (pid \$pid)\"; sleep 30
  done
  echo \$\$ >~/work/matrix/lock/pid
  trap 'rm -rf ~/work/matrix/lock' EXIT
  touch ~/work/matrix/targets/$tree/.last-used
  sudo launchctl asuser \"\$(id -u)\" sudo -u \"\$(id -un)\" env \\
    PATH=\"\$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin\" HOME=\"\$HOME\" SWM3_LAN_PORT=$lan \\
    CARGO_TARGET_DIR=\"\$HOME/work/matrix/targets/$tree\" bash -c '
      set -uo pipefail
      src=\$HOME/work/matrix/hb7-app/src; work=\$HOME/work/matrix/hb7-app/work
      rm -rf \$work && mkdir -p \$work/client
      cd \$src/apps/apple
      echo \"== xcframework\"; ./build-xcframework.sh >\$work/xcframework.log 2>&1 || { tail -30 \$work/xcframework.log; exit 1; }
      echo \"== build-for-testing (macOS)\"
      xcodebuild build-for-testing -quiet -project \"DSPi Console.xcodeproj\" -scheme \"DSPi Console\" \\
        -destination \"platform=macOS,arch=\$(uname -m)\" -derivedDataPath \$work/dd >\$work/build.log 2>&1 ||
        { grep -E \"error:\" \$work/build.log | sort -u | head -30; exit 1; }
      echo \"== the HB7 Swift client (swiftc)\"
      c=\$src/crates/dspi-hub/tests/conformance; l=\"\$src/apps/apple/DSPi Console/Link\"
      for f in LinkClient.swift LinkClientProtocol.swift LinkFrames.swift LinkMessages.swift LinkRole.swift DeviceTransport.swift AuthStore.swift; do cp \"\$l/\$f\" \$work/client/; done
      cp \$c/swift/main.swift \$c/swift/Shims.swift \$src/apps/apple/Tools/hb7_client_shims.swift \"\$src/apps/apple/DSPi Console/AppIdentity.swift\" \$work/client/
      (cd \$work/client && xcrun swiftc -O -o conformance *.swift 2>&1 | grep -E error) ; test -x \$work/client/conformance || exit 1
      echo \"== the client against the hub of the app\"
      python3 \$src/apps/apple/Tools/hb7_app_driver.py \$src/apps/apple \$work/dd \$work/client/conformance \$work
    ' </dev/null" </dev/null 2>&1 | tee -a "$log"
status=${PIPESTATUS[0]}
set -e
# The probe ends with the run: it may still be waiting for a hub that never came.
if [ -n "$probe_pid" ]; then kill "$probe_pid" 2>/dev/null || true; wait "$probe_pid" 2>/dev/null || true; fi
grep -q '^DONE 13/13' "$log" && say "hb7-app: DONE 13/13 against the Mac app's hub" || say "hb7-app: not all scenarios passed (status $status)"
exit "$status"
