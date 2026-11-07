#!/bin/bash
# The Swift app's rows of the build matrix (WP SW2). macOS only: run in the
# macOS VM through ci/macvm.sh, which copies the tree there first.
#
#   apps/apple/ci-test.sh macos   build the core's XCFramework, then build the
#                                 macOS app and run its unit tests
#   apps/apple/ci-test.sh ios     the same for the iOS app: unit and UI tests in
#                                 an iPhone Simulator
#
# Both runs must include WP SW2's CoreBindingsSW2Tests (core_info and the demo
# device through the generated bindings), passed. Under XCTest the app opens no
# real device (DSPi Console/TestHostGuard.swift).
#
# The simulator needs the GUI session: from ssh, run through
#   sudo launchctl asuser "$(id -u)" sudo -u "$USER" apps/apple/ci-test.sh ios
# Output in apps/apple/build/ci/ (gitignored): DerivedData and the .xcresult.
set -euo pipefail

platform=${1:-}
case "$platform" in macos|ios) ;; *) echo "usage: $0 macos|ios" >&2; exit 2 ;; esac
[ "$(uname -s)" = Darwin ] || { echo "ci-test: macOS only (the macOS VM)" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)

"$here/build-xcframework.sh"

cd "$here"
out=$here/build/ci
mkdir -p "$out"
result=$out/$platform.xcresult
rm -rf "$result"
args=(-project "DSPi Console.xcodeproj" -derivedDataPath "$out/dd-$platform" -resultBundlePath "$result")

case "$platform" in
  macos)
    args+=(-scheme "DSPi Console" -destination "platform=macOS,arch=$(uname -m)")
    # Tests that fail only in a VM (MR1S, the same before and after the move to
    # this repository): Metal precision on the paravirtual GPU, frame and timing
    # limits on shared vCPUs. A real Mac runs them.
    # kern.hv_vmm_present, or on Intel the CPU's hypervisor flag (OpenCore
    # hides the first).
    if [ "$(sysctl -n kern.hv_vmm_present 2>/dev/null)" = 1 ] ||
       sysctl -n machdep.cpu.features 2>/dev/null | grep -qw VMM; then
      for t in PeqGraphEditorTests/testGPUResponseMatchesDSPMath \
               RtaRenderingActiveReopenTests/testBarsResumeAfterCloseButtonWithoutANewFrame \
               RtaSharingTests/testBarsAloneReadNoBins \
               TubeLayoutBenchmark/testSyntheticDragCost \
               TubeRenderingTests/testAudioChangesOnlyOpacityAndStopsWhileHidden; do
        args+=("-skip-testing:DSPi ConsoleTests/$t")
      done
      echo "ci-test: in a VM, skipping 5 GPU/timing tests that fail only there"
    fi
    ;;
  ios)
    # Any available iPhone simulator; xcodebuild boots it.
    sim=$(xcrun simctl list devices available | sed -n 's/^ *\(iPhone[^(]*\) (\([0-9A-F-]\{36\}\)).*/\2 \1/p' | head -n 1)
    [ -n "$sim" ] || { echo "ci-test: no iPhone simulator available" >&2; exit 1; }
    echo "simulator: $sim"
    args+=(-scheme "DSPi Console iOS" -destination "id=${sim%% *}")
    ;;
esac

echo "-- xcodebuild test ${args[*]}"
status=0
xcodebuild test -quiet "${args[@]}" || status=$?

# The totals, and WP SW2's tests by name: a run without them does not count.
xcrun xcresulttool get test-results tests --path "$result" >"$out/$platform-tests.json"
python3 - "$out/$platform-tests.json" <<'EOF' || status=1
import json, sys
from collections import Counter
results = Counter()
sw2 = {}
def walk(node):
    if node.get("nodeType") == "Test Case":
        results[node.get("result", "?")] += 1
        if node.get("name", "").startswith("testSW2"):
            sw2[node["name"]] = node.get("result")
    for child in node.get("children", []):
        walk(child)
for node in json.load(open(sys.argv[1])).get("testNodes", []):
    walk(node)
print("tests: " + ", ".join(f"{n} {r.lower()}" for r, n in sorted(results.items())))
want = ["testSW2CoreInfo()", "testSW2OpensTheDemoDevice()", "testSW2TestHostOpensNoRealDevice()"]
for name in want:
    print(f"  CoreBindingsSW2Tests.{name}: {sw2.get(name, 'did not run')}")
sys.exit(0 if all(sw2.get(n) == "Passed" for n in want) else 1)
EOF
[ "$status" = 0 ] && echo "ci-test $platform: passed" || echo "ci-test $platform: FAILED" >&2
exit "$status"
