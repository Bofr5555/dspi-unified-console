#!/usr/bin/env python3
"""WP SW-M3: HB7's Swift conformance client against the Mac app's own hub (guest side).

Run by apps/apple/Tools/hb7-app-conformance.sh inside the macOS VM's GUI session, under
the matrix lock, after the app is built for testing and the client is compiled:

    hb7_app_driver.py <project dir> <derived data> <client binary> <work dir>

1. Starts the app's test host on one test, CoreHubSWM3Tests/testSWM3ServeHB7SwiftClient,
   which runs the app's hub service (Core/CoreHub.swift, the Rust hub through dspi_hub)
   with a demo device and serves serve.rs's file protocol in <work dir>/conf.
2. Once `hub.json` is there, runs the client (crates/dspi-hub/tests/conformance/swift,
   with the Swift console's LinkClient, unmodified) against it, and answers its `@ctl`
   lines through `cmd` and `ack`, as run-clients.sh does for the Rust test binary.

Exit status: the client's (0 when every scenario passed), or 1 if the app's side failed.
SWM3_LAN_PORT=<port> binds the app's hub to every interface on that port (the Local
Network check), loopback only otherwise.
"""
import json
import os
import platform
import subprocess
import sys
import time

project, derived, client_bin, work = sys.argv[1:5]
conf = os.path.join(work, "conf")
os.makedirs(conf, exist_ok=True)
for name in ("hub.json", "cmd", "ack"):
    try:
        os.remove(os.path.join(conf, name))
    except FileNotFoundError:
        pass

arch = "arm64" if platform.machine() == "arm64" else "x86_64"
bundle = os.path.join(work, "serve.xcresult")
subprocess.run(["rm", "-rf", bundle], check=False)
env = dict(os.environ, TEST_RUNNER_DSPI_SWM3_CONF=conf, TEST_RUNNER_DSPI_SWM3_SECS="900")
if os.environ.get("SWM3_LAN_PORT"):
    env["TEST_RUNNER_DSPI_SWM3_PORT"] = os.environ["SWM3_LAN_PORT"]
    env["TEST_RUNNER_DSPI_SWM3_LAN"] = "1"
xc_log = open(os.path.join(work, "serve-xcodebuild.log"), "w")
xc = subprocess.Popen(
    ["xcodebuild", "test-without-building", "-project", os.path.join(project, "DSPi Console.xcodeproj"),
     "-scheme", "DSPi Console", "-destination", f"platform=macOS,arch={arch}",
     "-derivedDataPath", derived,
     "-only-testing:DSPi ConsoleTests/CoreHubSWM3Tests/testSWM3ServeHB7SwiftClient",
     "-resultBundlePath", bundle],
    env=env, stdout=xc_log, stderr=subprocess.STDOUT)


def say(text):
    print(text, flush=True)


hub_json = os.path.join(conf, "hub.json")
deadline = time.time() + 600
while not os.path.exists(hub_json):
    if xc.poll() is not None or time.time() > deadline:
        say(f"the app's hub did not start (xcodebuild exited {xc.poll()}; see {xc_log.name})")
        sys.exit(1)
    time.sleep(0.2)
time.sleep(0.2)
hub = json.load(open(hub_json))
say(f"the app's hub {hub['hub_id']} is up on port {hub['port']}")

client = subprocess.Popen(
    [client_bin],
    env=dict(os.environ, HB7_URL=f"ws://127.0.0.1:{hub['port']}/dspi/v1", HB7_HUB_ID=hub["hub_id"]),
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
n = 0
for line in client.stdout:
    line = line.rstrip("\n")
    say(line)
    if not line.startswith("@ctl "):
        continue
    n += 1
    tmp = os.path.join(conf, ".cmd.tmp")
    with open(tmp, "w") as f:
        f.write(f"{n} {line[5:]}\n")
    os.rename(tmp, os.path.join(conf, "cmd"))
    reply = '{"error":"no answer from the hub"}'
    end = time.time() + 20
    while time.time() < end:
        try:
            ack = open(os.path.join(conf, "ack")).read().strip()
        except FileNotFoundError:
            ack = ""
        if ack.split(" ", 1)[0] == str(n):
            os.remove(os.path.join(conf, "ack"))
            reply = ack.split(" ", 1)[1] if " " in ack else "{}"
            break
        time.sleep(0.02)
    say(f"  <- {reply}")
    client.stdin.write(reply + "\n")
    client.stdin.flush()
status = client.wait()
try:
    xc_status = xc.wait(timeout=180)
except subprocess.TimeoutExpired:
    xc.kill()
    xc_status = -1
xc_log.close()
summary = subprocess.run(["xcrun", "xcresulttool", "get", "test-results", "summary", "--path", bundle],
                         capture_output=True, text=True).stdout
passed = '"passedTests" : 1' in summary or '"passedTests": 1' in summary
say(f"client exited {status}; the app's serving test: xcodebuild {xc_status}, passed {passed}")
sys.exit(status if status != 0 else (0 if passed else 1))
