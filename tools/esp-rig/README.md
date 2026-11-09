# esp-rig: the emulated ESP32 DSPi-Link bridge (WP LK3)

Upstream's ESP32 bridge firmware (WeebLabs/DSPi-Link `bridge/esp32`, at the conformance pin) runs in Espressif's QEMU.
Its DSPi UART goes to `esp-rig-tunnel`, which plays a DSPi on that UART: the demo device (`FakeDevice`, no hardware) or the
real DSPi on the host over USB. Its network is a tap device with netem in both directions, so delay, loss and
outages hit the ESP32's own lwIP TCP. Our client (the core over Link, in Android's order) and upstream's TypeScript
client run against it in scripted scenarios.

Used locally only. Nothing is pushed anywhere, and the firmware is built from a `git archive` of a local clone; the clone
is never changed.

```
 client(s) ── tap0 10.77.0.1 ══ netem both ways ══ open_eth 10.77.0.2 ── bridge firmware (QEMU esp32)
                                                                           │ UART1 (unix socket)
                                                       esp-rig-tunnel ─────┘ paced at the chosen baud, faults
                                                           │ Transport
                                                       demo device │ the DSPi on USB (read-only)
```

## The one deviation from upstream's firmware

QEMU does not emulate Wi-Fi. `firmware/net/net_qemu.c` replaces upstream's `components/net/net.c` and implements
the same `net.h` over QEMU's OpenCores Ethernet MAC (`CONFIG_ETH_USE_OPENETH`, `firmware/sdkconfig.defaults.rig`):

- a fixed address, 10.77.0.2/24 (`RIG_IP` at build time);
- mDNS with the same `_dspi._tcp` advertisement;
- the station state reported as "online";
- no setup network; scans find nothing and joins are refused.

Everything above lwIP is upstream's code, unchanged: hub, http_server, dspi_uart, dspi_frame, link_*, hub_auth,
settings, and lwIP with its TCP settings from upstream's `sdkconfig.defaults`. The firmware embeds upstream's web
pages. They are built with `npm ci --ignore-scripts` from upstream's lockfile in the Node image pinned in
`ci/container/images.conf`.

## What the tunnel does

- **Mapping.** Each UART request (`control_interfaces_spec.md` 3.2) is the USB vendor request it wraps:
  - a GET goes through `Transport::control_in_upto`;
  - a SET goes through `control_out`;
  - a STALL becomes `CTRL_STATUS_ERROR`;
  - a USB timeout gets no answer, so the bridge times out as on a silent DSPi;
  - a GET with `wLength` 0 reads the whole bulk image.
- **Notifications.** The device's notifications go out as `0x40` frames:
  - only at a frame boundary on an idle line (spec 3.6);
  - from a 31-entry ring, which drops the oldest entry as the firmware's ring does;
  - v2 packets only. The USB endpoint also delivers v1 packets and a one-byte keep-alive every 100 ms, which the
    firmware never sends on its UART, so the tunnel leaves them out and counts them (`notes_usb_only`).
- **UART-only facts it plays itself:**
  - `0xF6` (UART config) reports enabled, notify on, and the tunnel's baud;
  - `0xF9` (interface status) reports UART live;
  - `0xF5` and `0xF7` answer BLOCKED.
- **wIndex.** 0 and 2 (the vendor interface, which Link clients send) pass. Any other value is refused, because
  `Transport` cannot carry a wIndex.
- **Byte timing.** What the tunnel sends goes out at 10 bits per byte at `--baud`. QEMU's UART has no wire time, so
  the bridge's own sends arrive at once. See "Limits" below.
- **Faults.** `--flip-in/--drop-in` (bridge to DSPi), `--flip-out/--drop-out` (DSPi to bridge) and `--drop-reply`
  (the request runs, its answer is lost). All of them are per byte or per request, seeded and repeatable.
- **Writes.** With `--device usb`, every SET is refused with ERROR before it reaches the device, unless
  `--allow-writes` is given.

## Running it

Everything runs in rootless podman with no published port: QEMU from `IDF_IMAGE` (`images.conf`, ESP-IDF v6.1 by
digest), the tunnel and netem in `localhost/lk3-netem:1` (Fedora 44 with iproute-tc, built on first use), the TS client
in `NODE`. Work files go to `$ESP_RIG_WORK`, default `~/.cache/dspi-build/work/esp-rig`, never tmpfs. It needs the
DSPi-Link clone in `~/.cache/dspi-build/work/consoles/DSPi-Link`, as `conformance-ts` does.

```
tools/esp-rig/rig.sh build                 # firmware (about 3 min the first time), tunnel, our probe
tools/esp-rig/rig.sh build-before          # our probe on the pre-LK2 core (da91dec2), for comparisons
tools/esp-rig/rig.sh provision 115200      # a bridge image with its UART saved at 115200, detection off
tools/esp-rig/rig.sh provision 1000000

tools/esp-rig/rig.sh up r1 115200          # bridge + tunnel (demo device); waits for "Found DSPi"
tools/esp-rig/rig.sh shape r1 delay 50ms 20ms loss 10%    # netem words, each direction
tools/esp-rig/rig.sh rust r1 LK3_MODE=android LK3_SECONDS=30
tools/esp-rig/rig.sh rust-before r1 LK3_MODE=seq
tools/esp-rig/rig.sh ts r1 LK3_SECONDS=30 LK3_CLIENTS=2
tools/esp-rig/rig.sh outage r1 20          # 100 % loss for 20 s, then the old shaping
tools/esp-rig/rig.sh realhub r1            # crates/dspi-link/tests/real_hub's read-only set against the bridge
tools/esp-rig/rig.sh log r1                # the bridge's serial log
tools/esp-rig/rig.sh down r1               # also keeps the serial log in runs/r1/bridge.log
```

- **Tunnel options.** Anything after the baud in `up` goes to the tunnel, for example
  `rig.sh up r1 115200 --drop-reply 0.01 --flip-out 0.0001`. Its counters are in
  `$ESP_RIG_WORK/runs/r1/tunnel.err`, and one line per request (time, opcode, kind, request span, reply bytes, status)
  is in `tunnel.log`.
- **Baud.** The bridge cannot be told the line's rate by the emulator: QEMU's socket UART has no rate. So
  `provision` boots once, saves `uart.baud` with `detect_baud` off through `hub.config.set` (upstream's TS client),
  and keeps that flash as `fw/flash-BAUD.bin`. `up` boots a copy of it.
- **Scenarios.** `scenarios.sh smoke|matrix|outage|pollload|reread` prints `RESULT` lines, and `summary.py LOG`
  turns them into tables. Each line also counts the bridge's own log lines for its 10 s unwritable close
  ("has sent nothing for") and its queue-full drop reports.
- **CI.** `ci/check.sh --step esp-rig` (`ci.sh`) is opt-in and not in `--all`. It builds what is missing, then
  runs the smoke with our client and the TS client.

### The real DSPi (K340)

```
ESP_RIG_DSPI_CLI=1 tools/esp-rig/rig.sh build    # also the Terminal client, for the dump
tools/esp-rig/k340.sh dump  $ESP_RIG_WORK/k340/before
tools/esp-rig/rig.sh up k1 115200 --device usb   # read-only: SETs refused in the tunnel
...                                              # scenarios
tools/esp-rig/rig.sh down k1
tools/esp-rig/k340.sh dump  $ESP_RIG_WORK/k340/after
tools/esp-rig/k340.sh compare $ESP_RIG_WORK/k340/before $ESP_RIG_WORK/k340/after
```

Only one job may use the device at a time.

## Limits

- **The bridge's own sends take no wire time.** QEMU's UART returns `uart_wait_tx_done` at once, so a request
  reaches the tunnel at once. The bridge starts its reply timeout after that call on a real board too, so its
  timeouts are right. What is lost is the uart task being busy for the request's wire time: 0.53 s for a 6 KB bulk
  SET at 115200, about 1 ms for an ordinary request.
- **Received bytes are not lost to overruns.** When the guest's RX FIFO is full, QEMU stops reading the socket. A
  real UART would lose the bytes instead. Use `--drop-out` to model losses.
- **Not a real radio.** There is no Wi-Fi power save, no 802.11 retries, no airtime contention. netem's loss is
  independent per packet, whereas Wi-Fi loses packets in bursts, which `outage` stands in for.
- **The emulated ESP32 runs in real time but is not cycle-exact.** One QEMU process takes about 25 % of a host core.
- **real_hub's dark-network scenarios go dark in their own proxy.** That proxy keeps ACKing the bridge's TCP, so
  they never reach the bridge's 10 s no-write-progress close. Use `rig.sh outage` for that.
- **The demo device answers at once.** `--latency-us` (default 300) stands in for the firmware's main-loop
  dispatch.
