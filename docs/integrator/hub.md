# The hub

A hub shares the DSPi devices on one machine's USB with clients on the network,
over the DSPi Link protocol (DSPi-Link's `spec/dspi_link_protocol_spec.md`). Any
client of this project connects to it: the desktop, Android and iOS apps, the
web console, `uconsole --link` or `--hub`.

Reference: `crates/dspi-hub` (design: [`docs/core-merge/HUB.md`](../core-merge/HUB.md)),
`apps/terminal/src/hub.rs` (`uconsole hub`), `uconsole hub --help`.

## Install

- `uconsole` from a Terminal release archive (`ci/release-terminal.sh`), or a
  build: `cargo build --release -p dspi-term`.
- On Linux, the udev rule for USB access without root: `sudo uconsole
  --install-udev`, or `70-dspi.rules` from the archive into `/etc/udev/rules.d/`.
- Run it: `uconsole hub`. Ctrl-C or SIGTERM stops it and lets go of USB.
- Check it: `uconsole doctor` on the hub machine; `uconsole --link HOST doctor`
  from a client.

To write: a headless single-board computer walk-through; ARM builds.

## Service units

Templates in the release archive's `service/` folder
(`apps/terminal/packaging/service/`, rendered by `render.sh`); each file's
header says how to install it.

| OS | File | Runs as |
|---|---|---|
| Linux | `uconsole-hub.service` | systemd user service (`loginctl enable-linger` to keep it without a login) |
| macOS | `se.bofr.uconsole.hub.plist` | launchd agent |
| Windows | `uconsole-hub-task.xml` | scheduled task at logon |

Exit status 1 (the command line, a store another hub holds, or a change of
access mode that needs `--forget-clients`) and 4 (the machine: a store that
cannot be created or opened, the port in use) are not restarted by the systemd
unit: read the journal.

To write: a system-wide (not per-user) unit and the account it should run as.

## Pairing and the PIN

PIN pairing is the default; `--open` lets every connection do anything and is
for a closed bench only.

- `pair` (typed at the running hub) opens a window of 2 minutes and shows a
  6-digit PIN. A client that enters it gets a token, stored by the client; the
  hub stores only the token's SHA-256.
- A PIN dies after 5 wrong tries from anywhere; wrong tries are also rate
  limited per address.
- At most 64 paired clients. `uconsole hub clients`, `role`, `revoke` manage
  them (with the hub stopped, or live at its prompt).
- A service has no terminal: stop it, pair from `uconsole hub` in a terminal,
  start it again (`service/README.md`). `--pair` prints the PIN to the log.
- A PIN gives at most `control`: a client that asks for `admin` with a PIN gets
  `control` (WP HWLK1F). Admin is given on the hub's own machine only:
  `uconsole hub role <cid> admin` (or `role <cid> admin` at its prompt), or
  `uconsole hub --pair-admin`, whose pairing window may give admin to the one
  client that pairs in it and asks for it.
- Changing between PIN and open access forgets every paired client, and needs
  `--forget-clients`.

## Roles

| Role | May | Command classes (`tests/fixtures/link/policy/commands.json`) |
|---|---|---|
| viewer | read, poll, receive notifications | `read` |
| control | also change settings | `read`, `control` |
| admin | everything, including the device configuration | `read`, `control`, `config` |

A new client is `control` unless it asks for less; asking for more with a PIN
still gives `control` (see Pairing). The core-voltage extension
has its own rule: no client on the network raises it above 1.20 V, whatever its
role; do that on the hub machine. Reference: `crates/dspi-hub/src/policy.rs`.

## Guarded apply

<!-- WP P1HA -->
A client about to change a device over Link (an import, a snapshot recall) can
ask the hub to guard the change: if the client does not confirm it in time, or
goes away, the hub writes the device's previous settings back itself. Additive
to Link 1.2: a hub that has it lists `apply_guard` in `hello.caps`, and every
answer carries `"v": 1`. A client checks the cap before sending them; other hubs and the upstream page are unaffected.

| Message | Role | What |
|---|---|---|
| `apply.begin {id, handle, timeout_ms?}` | control, admin | reads the device's whole image (`0xA0`, in the device's order behind what this client queued) and starts the guard; then the settings outside the image (below). `timeout_ms` defaults to 30 000 and is clamped to 1 000 to 600 000. Answer `ok {guard, serial, timeout_ms, captured}` |
| `apply.confirm {id, handle}` | control, admin | keeps the change. Only the session that began it, and only before the timeout. Answer `ok {guard, serial, handle, reason: "confirmed", result: "kept", at}` |
| `apply.cancel {id, handle}` | control, admin | writes the image back now and answers with the outcome, as `apply.ended` below |
| `apply.status {id, handle}` | every role | `ok {active, last}`: the running guard (`guard`, `serial`, `session`, `remaining_ms`, `reverting`) or `null`, and the last outcome on that device or `null` |

Every end is also sent to every session as
`apply.ended {v, guard, serial, handle, reason, result, status?, taken_by?, at, captured, restored?, notes?}`:
`reason` is `confirmed`, `timeout`, `session_closed`, `cancelled`,
`hub_stopping` or `taken_over`; `result` is `kept`, `reverted`, `revert_failed`
(with the Link `status`; it is not tried again) or `not_written` (no device with
that serial was online: nothing was written).

The settings outside the image <!-- WP P1CFIX -->: `apply.begin` also reads the
device name, the input source names, the output latency and the control-surface
configuration (groups, macros, IR commands, bindings and their names, the
display configuration and pages), as the beginning client, and lists what it
holds in `captured` (`device_name`, `source_names`, `output_latency`,
`control_surfaces`; a part the firmware lacks is left out). A write back first
writes each held part that differs from the device then, in the order a
snapshot recall does, then the image; `restored` lists the parts written and
`notes` what was refused. One write each, never repeated; the same lock and
serial rules as the image. An output configuration kept apart from presets
(pins and limiters, `output_config_mode` independent) is not put back. An
outcome without `captured` comes from an older hub, which writes back the
image only.

- One guard per device: a second `apply.begin` is `busy`. Another session's
  confirm or cancel is `denied`; a confirm after the timeout is `conflict`.
- Another session taking the device's lock (`lock.acquire`, or the hub app's
  own) ends the guard at once **without** a write back: the lock holder is in
  charge now. The end is `reason: "taken_over"`, `result: "kept"`, with
  `taken_by` the session that took the lock; the guard's own client hears it as
  `apply.ended` like everyone. The guard's own session renewing its lock does
  not end it. <!-- WP P1BFIX -->
- The write back is the hub's own (`0xA1`, origin the hub's session) and is
  never refused by another session's lock (one already held when the deadline
  passes, e.g. one taken while the write back was starting). It only ever goes
  to the board with the serial the guard was begun on: after a replug of another
  board nothing is written.
- The guard does not narrow what a role may send: a Control client can write the
  whole image (`0xA1`, class `control` in the policy) itself today, so a snapshot
  recall by a Control client over Link writes its image; P1HL narrows that.
  <!-- WP P1BFIX -->
- Stopping the hub (sharing off) reverts every running guard first.

Reference: `crates/dspi-hub/src/apply_guard.rs`.

## Network and firewall

| What | Port | Notes |
|---|---|---|
| DSPi Link (WebSocket) and `GET /dspi/v1/info` | TCP 11915 (`--port`) | the one port clients need |
| Discovery | UDP 5353, mDNS, `_dspi._tcp` | optional: clients can be given the address; Windows advertises through its DNS Client service |
| The web console, this machine only (`uconsole web`) | TCP 11916, loopback | not reachable from the network |

- `--loopback` listens on this machine only and does not advertise.
- `--web` also serves the web console and the hub's page to browsers on the
  network, on the Link port.
- A PIN crosses the network in plain text, and the protocol has no TLS: pair on
  a network you trust, and keep the hub off networks you do not.

To write: firewall commands per OS (firewalld, ufw, Windows Defender Firewall,
the macOS application firewall); VLAN and routed-network notes (mDNS does not
cross routers).

## The store

`hub-store.json` in the store folder (`--store`; default the user's local data
folder, `uconsole/hub`): the hub id, its name, the access mode, the paired
clients (name, role, times, token hash) and device names. One hub per store.
A diagnostics bundle includes it with the token hashes removed
([diagnostics.md](diagnostics.md)).
