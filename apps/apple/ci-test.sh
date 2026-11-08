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
# WP SW4: then the transport tests (FakeTransportTests, the wire tests) run
# again behind the launch flag, with the Rust core carrying every request
# (CoreBackedTransport over the Swift FakeTransport); they must all pass.
#
# The simulator needs the GUI session: from ssh, run through
#   sudo launchctl asuser "$(id -u)" sudo -u "$USER" apps/apple/ci-test.sh ios
# Output in $CI_OUT (default apps/apple/build/ci, gitignored): DerivedData, the
# .xcresult and its test list as JSON.
set -euo pipefail

platform=${1:-}
case "$platform" in macos|ios) ;; *) echo "usage: $0 macos|ios" >&2; exit 2 ;; esac
[ "$(uname -s)" = Darwin ] || { echo "ci-test: macOS only (the macOS VM)" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)

"$here/build-xcframework.sh"

cd "$here"
out=${CI_OUT:-$here/build/ci}
mkdir -p "$out"
rm -rf "$out/$platform".xcresult "$out/$platform"-*.xcresult "$out/$platform"*-tests.json
args=(-project "DSPi Console.xcodeproj" -derivedDataPath "$out/dd-$platform")
alone=()   # test classes run in a pass of their own, after the others
# -- WP SW4 -- The transport tests again behind the launch flag
# (DSPI_CORE_TRANSPORT=1): the Rust core carries every request, over the
# Swift FakeTransport (CG26). The wire tests only in the macOS bundle are
# added below.
bundle="DSPi ConsoleTests"
core_pass=(CoreBackedTransportSW4Tests FakeTransportTests ExtensionChannelWireTests
           ExtensionChannelViewModelTests ExtensionChannelPresetTests OutputLatencyWireTests)

case "$platform" in
  macos)
    args+=(-scheme "DSPi Console" -destination "platform=macOS,arch=$(uname -m)")
    core_pass+=(AdatWireTests ControlSurfacesWireTests I2sClockPinModeWireTests I2sSlaveWireTests
                LimiterWireTests PsybassWireTests RtaWireTests SiggenWireTests SpdifInputWireTests
                SubharmWireTests TubeWireTests UpmixerWireTests)   # -- WP SW4 --
    core_pass+=(SaveCoordinationTests)   # -- WP SW8 -- the view model over the core's mirror
    # In a VM: skip the tests that failed there before and after the move to
    # this repository (MR1S): Metal precision on the paravirtual GPU, frame and
    # timing limits on shared vCPUs. Other rendering and benchmark tests fail
    # there now and then when the host is busy, so a failed test runs up to
    # twice more. A real Mac runs them all once. A VM shows in
    # kern.hv_vmm_present, or on Intel in the CPU's hypervisor flag (OpenCore
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
      args+=(-retry-tests-on-failure -test-iterations 3)
      # Its setUp waits only 20 ms on the main run loop for an async reply: in
      # the VM it fails inside the full suite (other cases each run), not alone.
      alone+=("DSPi ConsoleTests/RtaSharingTests")
      echo "ci-test: in a VM: skipping 5 GPU/timing tests that fail only there, retrying failures, RtaSharingTests alone"
    fi
    ;;
  ios)
    # Any available iPhone simulator; xcodebuild boots it.
    sim=$(xcrun simctl list devices available | sed -n 's/^ *\(iPhone[^(]*\) (\([0-9A-F-]\{36\}\)).*/\2 \1/p' | head -n 1)
    [ -n "$sim" ] || { echo "ci-test: no iPhone simulator available" >&2; exit 1; }
    echo "simulator: $sim"
    args+=(-scheme "DSPi Console iOS" -destination "id=${sim%% *}")
    bundle="DSPi Console iOSTests"   # -- WP SW4 --
    ;;
esac

status=0
skip=()
# ${a[@]+"${a[@]}"}: macOS's bash 3.2 calls an empty array unbound under set -u.
for t in ${alone[@]+"${alone[@]}"}; do skip+=("-skip-testing:$t"); done
echo "-- xcodebuild test ${args[*]} ${skip[*]+${skip[*]}}"
xcodebuild test -quiet "${args[@]}" ${skip[@]+"${skip[@]}"} -resultBundlePath "$out/$platform.xcresult" || status=$?
xcrun xcresulttool get test-results tests --path "$out/$platform.xcresult" >"$out/$platform-tests.json"
reports=("$out/$platform-tests.json")
for t in ${alone[@]+"${alone[@]}"}; do
  name=${t##*/}
  echo "-- xcodebuild test-without-building, $name alone"
  xcodebuild test-without-building -quiet "${args[@]}" "-only-testing:$t" \
    -resultBundlePath "$out/$platform-$name.xcresult" || status=$?
  xcrun xcresulttool get test-results tests --path "$out/$platform-$name.xcresult" >"$out/$platform-$name-tests.json"
  reports+=("$out/$platform-$name-tests.json")
done

# -- WP SW4 --
echo "-- xcodebuild test-without-building, the transport tests through the core (DSPI_CORE_TRANSPORT=1)"
# Without a test from the build above (it failed), the products are stale.
grep -q '"Test Case"' "$out/$platform-tests.json" || { echo "ci-test: no build to test" >&2; status=1; }
only=()
for c in "${core_pass[@]}"; do only+=("-only-testing:$bundle/$c"); done
TEST_RUNNER_DSPI_CORE_TRANSPORT=1 xcodebuild test-without-building -quiet "${args[@]}" "${only[@]}" \
  -resultBundlePath "$out/$platform-core.xcresult" || status=$?
xcrun xcresulttool get test-results tests --path "$out/$platform-core.xcresult" >"$out/$platform-core-tests.json"
python3 - "$out/$platform-core-tests.json" <<'EOF' || status=1
import json, sys
from collections import Counter
results, classes = Counter(), Counter()
flag = "did not run"
def walk(node, cls=None):
    global flag
    if node.get("nodeType") == "Test Suite":
        cls = node.get("name")
    if node.get("nodeType") == "Test Case":
        results[node.get("result", "?")] += 1
        if node.get("result") == "Passed":
            classes[cls] += 1
        if node.get("name", "").startswith("testSW4LaunchFlagPutsTheCoreUnderTheTests"):
            flag = node.get("result")
        if node.get("result") == "Failed":
            print(f"FAILED (core) {node.get('nodeIdentifier', node.get('name'))}")
            def messages(n):
                for c in n.get("children", []):
                    if c.get("nodeType") == "Failure Message":
                        print(f"    {c.get('name', '')[:400]}")
                    messages(c)
            messages(node)
    for child in node.get("children", []):
        walk(child, cls)
for node in json.load(open(sys.argv[1])).get("testNodes", []):
    walk(node)
print("tests through the core: " + ", ".join(f"{n} {r.lower()}" for r, n in sorted(results.items())))
print("  passed per class: " + ", ".join(f"{c} {n}" for c, n in sorted(classes.items())))
print(f"  the flag reached the tests (testSW4LaunchFlagPutsTheCoreUnderTheTests): {flag}")
need = ["CoreBackedTransportSW4Tests", "FakeTransportTests", "ExtensionChannelViewModelTests"]
sys.exit(0 if results["Failed"] == 0 and flag == "Passed" and all(classes[c] > 0 for c in need) else 1)
EOF

# The totals, and WP SW2's tests by name: a run without them does not count.
python3 - "${reports[@]}" <<'EOF' || status=1
import json, sys
from collections import Counter
results = Counter()
sw2 = {}
def walk(node):
    if node.get("nodeType") == "Test Case":
        results[node.get("result", "?")] += 1
        if node.get("result") == "Failed":
            print(f"FAILED {node.get('nodeIdentifier', node.get('name'))}")
            def messages(n):
                for c in n.get("children", []):
                    if c.get("nodeType") == "Failure Message":
                        print(f"    {c.get('name', '')[:400]}")
                    messages(c)
            messages(node)
        if node.get("name", "").startswith("testSW2"):
            sw2[node["name"]] = node.get("result")
    for child in node.get("children", []):
        walk(child)
for report in sys.argv[1:]:
    for node in json.load(open(report)).get("testNodes", []):
        walk(node)
print("tests: " + ", ".join(f"{n} {r.lower()}" for r, n in sorted(results.items())))
want = ["testSW2CoreInfo()", "testSW2OpensTheDemoDevice()", "testSW2TestHostOpensNoRealDevice()"]
for name in want:
    print(f"  CoreBindingsSW2Tests.{name}: {sw2.get(name, 'did not run')}")
sys.exit(0 if all(sw2.get(n) == "Passed" for n in want) else 1)
EOF
[ "$status" = 0 ] && echo "ci-test $platform: passed" || echo "ci-test $platform: FAILED" >&2
exit "$status"
