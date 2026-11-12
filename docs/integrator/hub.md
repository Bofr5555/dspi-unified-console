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

Exit status 1 (the command line or the store) and 4 (the port in use) are not
restarted by the systemd unit: read the journal.

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
- Changing between PIN and open access forgets every paired client, and needs
  `--forget-clients`.

## Roles

| Role | May | Command classes (`tests/fixtures/link/policy/commands.json`) |
|---|---|---|
| viewer | read, poll, receive notifications | `read` |
| control | also change settings | `read`, `control` |
| admin | everything, including the device configuration | `read`, `control`, `config` |

A new client is `control` unless it asks for less. The core-voltage extension
has its own rule: no client on the network raises it above 1.20 V, whatever its
role; do that on the hub machine. Reference: `crates/dspi-hub/src/policy.rs`.

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
