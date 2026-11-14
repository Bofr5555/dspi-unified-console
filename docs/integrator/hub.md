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
  On a device with a limits profile or a lock only an Admin may (see Limits and
  the installer lock). <!-- WP P1HL -->
  <!-- WP P1BFIX -->
- Stopping the hub (sharing off) reverts every running guard first.

Reference: `crates/dspi-hub/src/apply_guard.rs`.

## Limits and the installer lock

<!-- WP P1HL -->
The hub keeps a limits profile per device (by serial) and the installer PIN
beside it, and judges every network client's write against them: the same
check the consoles run on USB ([limits-profiles.md](limits-profiles.md)).
Additive to Link 1.2: a hub that has it lists `limits` in `hello.caps`, and every
answer carries `"v": 1`. With no profile a device behaves as before.

| Message | Role | What |
|---|---|---|
| `limits.get {id, handle}` | every role | `ok {serial, handle, profile, pin_set, locked, unlocked, held, wrong_tries, retry_in_ms}`: the profile (as the `limits` block of a limits document) or `null`; whether a PIN is set; whether the device is locked, and whether it is unlocked for this session; the rows the hub last held; the wrong PINs since the last good one and the refusal window left, ms (0: none) |
| `limits.set {id, handle, profile}` | admin | sets the profile (a limits document, a document carrying one in `meta.limits`, or the profile object); `null` clears it. Its `serial` is the device's. A device outside the new profile is held at once: `ok {serial, held}` |
| `limits.pin {id, handle, pin}` | admin | sets the PIN (4 to 64 characters), or clears it with `null`, which removes the lock |
| `limits.unlock {id, handle, pin}` | control, admin | unlocks the device for this session (an Admin needs no PIN). A wrong PIN is `bad_pin`. The consoles' rule: 3 wrong PINs are free, each further one opens a refusal window of 30 s, doubling to 15 min, inside which every attempt is refused unchecked (`retry_later`, `retry_in_ms`); a good PIN resets the count |
| `limits.lock {id, handle}` | control, admin | locks the device again for this session |
| `mode.pinned {id, handle?}` | every role | `ok {role, mode, locked?}`: the mode the hub pins for this session's role (`simple`, `advanced`, `installer`) or `null` (the user's choice), and with a handle whether that device is locked for this session |
| `mode.pin {id, role, mode}` | admin | pins a role's mode, or unpins it with `null` |

Events to every session: `limits.changed {v, serial, handle}` after a profile or
PIN change, `limits.held {v, serial, handle, rows}` after the hub held a device,
`mode.changed {v, role, mode}`.

- **A write outside a limit** (output gain, output or channel delay, master
  volume, a PEQ boost, the high-pass an output must keep, a limiter that must
  stay on below a ceiling, a limiter link group that would adopt settings past
  it) is refused before it reaches the device: `DENIED` with a JSON payload
  `{"refused": "limit_exceeded", "limit", "msg"}`, from Admin and Control
  alike. Muting is always allowed. Every index is read as the firmware reads it
  (the output of a gain or delay write is `wValue & 0xFF`, so `0x0100` is output
  0), and a judged write the hub cannot read (a short payload, a value that is
  not a number) is refused the same way, never passed unjudged.
- **Bulk images** (`0xA1`, `0xA3`) from a Control client are refused while the
  device has a profile or a lock (`"refused": "image_refused"`): an import, a
  snapshot recall, an A/B toggle or a guarded apply that writes an image is
  Admin-only there. An Admin's image is accepted and then held.
- **The lock.** A device is locked when a PIN is set and its profile names what
  the lock covers (`lockedPages`). A Control client of a locked device may change
  the volume, the mute and the preset only (`"refused": "installer_lock"` otherwise)
  and run the analyser (`RTA_CONTROL`, `RTA_SET_CONFIG`: it changes no audio)
  until it unlocks; reads stay open. Unlocking lasts for the session.
- **Wrong PINs** are counted per device, whichever session sends them, before
  the slow hash runs, so attempts sent at once all see the count (decided:
  security over availability; one client's guesses delay another's unlock, an
  Admin never needs the PIN). The count and the window are kept in the store
  (`device_pin_tries`, the window's end on the wall clock, never trusted past
  15 min), so a restart does not reset them.
- **Held after the fact.** When the whole state changes in a way no write could be
  judged for (a preset loaded by anyone, a scene's preset load, an Admin's image,
  a factory reset), the hub reads the device back, writes every value outside the
  profile to the limit (a high-pass raised or added, a limiter switched on) and
  sends `limits.held`. The hold passes a session's device lock, as a revert does.
- **The hub's own writes** obey it: a system scene whose op crosses a unit's limit
  is refused before anything is sent (`system.scene` answers `denied` naming the
  unit and `limit_exceeded`); keep-aligned and direct scene writes are judged like
  a client's. A guarded apply's revert is never refused, and its own change is
  never held (told by its origin, the hub's session); anyone else's slot load or
  image right after it is.
- **The console the hub runs in** is not judged by the hub: its own core keeps its
  own copy of the profile.
- **The PIN** is kept only as a salted, slow hash (PBKDF2-HMAC-SHA256, a random
  salt with the serial) under `device_pins` in `hub-store.json`; no message
  answers it, and a diagnostics bundle removes it. Profiles are under
  `device_limits`, pinned modes under `role_modes`. Like the consoles' PIN it is a
  UI lock: whoever has the device on USB can change it with any tool.

Reference: `crates/dspi-hub/src/limits.rs`.

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
clients (name, role, times, token hash) and device names; limits profiles,
PIN hashes and pinned modes (P1HL). One hub per store.
A diagnostics bundle includes it with the token hashes removed
([diagnostics.md](diagnostics.md)).
