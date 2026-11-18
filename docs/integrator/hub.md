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
- <!-- WP SECDOC --> The PIN is shown with the hub key's fingerprint
  (`PIN 482913 · key SHA256:K7Q2-M9XD-4TNB-0ZRA`) and, on a terminal, a QR code
  of the pairing link `dspi://pair?h=<hid>&a=<host:port>&k=<key>&p=<PIN>`
  (`--json`: the `pairing` event's `fingerprint` and `pairLink`). Compare the
  fingerprint the client shows before typing the PIN; a phone that scans the
  QR code gets the key out of band. Pairing happens over TLS only, unless the
  hub runs `--tls off` or `--upstream-compat` (Encryption, below); the client
  pins the key it paired with ([security.md](security.md)).
- A PIN dies after 5 wrong tries from anywhere (`pairingClosed`, reason
  `wrongPins`: open a new window knowingly); wrong tries are also rate
  limited per address. A client retrying a revoked token is counted apart and
  never closes a window.
- <!-- WP SECDOC --> A console's token unused for 180 days is forgotten
  (`--token-idle-days <n>`, 0: never); API keys do not expire.
- At most 64 paired clients. `uconsole hub clients`, `role`, `revoke` manage
  them (with the hub stopped, or live at its prompt).
- A service has no terminal: stop it, pair from `uconsole hub` in a terminal,
  start it again (`service/README.md`). `--pair` prints the PIN to the log.
- A PIN gives at most `control`: a client that asks for `admin` with a PIN gets
  `control` (WP HWLK1F). No pairing gives more (WP PAIRADM, owner 2026-11-14).
  **Admin is given on the hub's own machine only:** find the client's number
  with `uconsole hub clients`, then run `uconsole hub role <cid> admin` (both
  with the hub stopped), or type `clients` and `role <cid> admin` at the running
  hub's prompt. At the prompt it applies live: the client's next message is
  judged as Admin, without a reconnect. The option `--pair-admin` of earlier
  builds is refused with a message that points here.
- Changing between PIN and open access forgets every paired client, and needs
  `--forget-clients`.

## Roles

| Role | May | Command classes (`tests/fixtures/link/policy/commands-1.2.json`) |
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
| `limits.pin {id, handle, pin}` | admin | sets the PIN (4 to 64 characters), or clears it with `null`, which removes the lock. A new PIN answers `ok {serial, pin_set, recovery_code}`: a recovery code (`XXXXX-XXXXX-XXXXX-XXXXX`, WP HUBADMIN, OD-P1c) that `limits.unlock` takes in place of the PIN (any case, spaces or dashes), shown in this reply only and kept as a salted, slow hash (`device_pin_recovery`, removed from a diagnostics bundle); it goes with the PIN it was made for |
| `limits.unlock {id, handle, pin}` | control, admin | unlocks the device for this session (an Admin needs no PIN). A wrong PIN is `bad_pin`. The consoles' rule: 3 wrong PINs are free, each further one opens a refusal window of 30 s, doubling to 15 min, inside which every attempt is refused unchecked (`retry_later`, `retry_in_ms`); a good PIN resets the count |
| `limits.lock {id, handle}` | control, admin | locks the device again for this session |
| `mode.pinned {id, handle?}` | every role | `ok {role, mode, locked?, pins?}`: the mode the hub pins for this session's role (`simple`, `advanced`, `installer`) or `null` (the user's choice), and with a handle whether that device is locked for this session. An Admin also gets `pins {viewer, control, admin}`, every role's (WP HUBADMIN: its Installer page) |
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

The consoles' typed calls for these messages are `dspi_core::hub_admin` (WP
HUBADMIN): `installer_target` (which copy an Installer page edits: the hub's
for a device reached through a hub that keeps limits, this console's own only
for a directly connected one), `hub_limits_get`/`set`/`clear`,
`hub_pin_set`/`clear` and `hub_mode_pin`, and the events through
`dspi-link`'s message hook (`LinkEvents::hub_event`), after which the core
reads the lock again (`installer_lock_refresh`) and tells the app
(`HubAdminListener`, or `hub_admin_events` on the web). A console paired with
the hub's PIN is Control (no pairing gives more), so the Admin role is given on
the hub's machine: `uconsole hub clients` and `uconsole hub role <cid> admin`,
or `clients` and `role <cid> admin` at the running hub's prompt, where it
changes live; `hub_limits_get` answers with it. The read-only Installer page
says so in the notice row `installer.hub_role`.

Reference: `crates/dspi-hub/src/limits.rs`.

## Encryption

<!-- WP SECDOC (PLAN-SEC 4.1-4.4, 4.11-4.12) -->
The hub speaks TLS 1.3 on its port with its own P-256 key, made at the first start
with TLS and kept in the store (`tls/`). Clients pin the key's hash when they pair
and refuse any other key from then on. Why, and what each client shows:
[security.md](security.md). The protocol side: [hub-api.md](hub-api.md) section 2.

`--tls <mode>` decides what the port serves. **`require` is the default** of
`uconsole hub`; `off` is the default with `--loopback` and for `uconsole web`, where
nothing crosses a network. A library embedder (`NetworkConfig`) chooses its own.

| Over plaintext | `require` (default) | `on` | `--upstream-compat` (`on` + compatibility) | `off` |
|---|---|---|---|---|
| TLS served | yes | yes | yes | no, and no key is made |
| pages, `/dspi/v1/info` | `426`, naming `https://` and the fingerprint | yes | yes | yes |
| the WebSocket, with a token | upgraded, then closed 4003 `tls_required` | yes, unless the token was used over TLS before | yes, as `on` | yes |
| `auth.pair` with a PIN | no | `tls_required` | yes | yes |
| REST with an API key | `426` | only a key minted `plaintext_ok` | every key | every key |
| DNS-SD `tls=1` | yes | yes | no | no |
| start-up warning | none | none | **SECURITY DOWNGRADED**, listing what is off | **SECURITY DOWNGRADED** (on a network address) |

- `--upstream-compat` is **off by default**, for upstream DSPi Link clients, which
  cannot encrypt. It is refused with `--tls require`. The warning it starts with
  lists exactly what it turns off (TLS no longer required, plaintext connections
  readable and changeable, no key pinning on them, plaintext pairing, every API key
  in plaintext, no `tls=1`); every client's hub settings show the same list.
  Our own clients keep using TLS with such a hub.
- `--host-name <name>` (repeatable): a DNS name the hub is reached by besides its
  `.local` name and its addresses, for the plaintext `Host` check (DNS rebinding).
- `--token-idle-days <n>`: a console's token unused this long is forgotten (default
  180; 0: never).

**The key**, on the hub's machine (at a running hub's prompt, type the same without
`uconsole hub`; from a shell, `rotate` needs the hub stopped):

| Command | Does |
|---|---|
| `uconsole hub tls fingerprint` | the fingerprint (`SHA256:K7Q2-M9XD-4TNB-0ZRA`) and curl's `--pinnedpubkey` form |
| `uconsole hub tls cert` | the certificate, PEM: `> hub.pem` for curl `--cacert`, Python, a browser's store |
| `uconsole hub tls rotate` | a new key, served at once, announced for 30 days signed by the old one: paired clients follow it by themselves |
| `uconsole hub tls rotate --forget-old` | a new key with no signature (the old one was exposed): every client must pair again |

The certificate is issued again, with the same key, when the hub's `.local` name
changes or it is within 30 days of its end (825 days); a DHCP address change never
does. A pin is the key's hash, so clients do not notice. A browser that accepted the
old certificate warns once more.

**Upgrading a hub from before TLS.** Nothing to do: at the first start with TLS the
key is made, the hub id and the paired clients stay, and an updated client gets in
with its old token over TLS and pins the key then (the one first-use moment; it
shows the key's fingerprint once). With `--tls on` a client not yet
updated keeps working in plaintext, until the same token has been used over TLS
(by the updated client): from then on it is TLS only. With the
default `require` it is told `tls_required` (4003) and must be updated; it does not
need to pair again. REST keys minted before TLS work over https; in `on` they are
refused in plaintext (`426`) unless minted `plaintext_ok`. Tested in
`crates/dspi-hub/tests/wp_secdef_upgrade.rs` and
`apps/terminal/tests/wp_secdef_default.rs`.

**What stays plaintext:** mDNS (a hint only), loopback, and hubs started with
`--tls off`. USB and the audio link are local: see [security.md](security.md).

## Security deviations

<!-- WP SECDOC (PLAN-SEC 4.12) -->
What this hub does beyond DSPi Link 1.2 (the spec copy:
`apps/apple/Documentation/dspi_link_protocol_spec.md`). Upstream is told nothing
automatically; offering it upstream is the owner's call.

| # | Spec | This hub | Old clients |
|---|---|---|---|
| 1 | 3.1: `tls=1` means "connect with `wss://`" | `tls=1` whenever TLS is served (`require`, `on`), not with upstream compatibility; no other TXT key (the plan's `tlsk` was not needed: SECDISC) | an upstream client that reads `tls=1` tries `wss://` and fails on the self-signed certificate: use `--upstream-compat` for it |
| 2 | 4: `wss://` "when TLS is advertised" | TLS and plaintext on the same port, by the connection's first byte (`0x16`: TLS) | additive |
| 3 | 6.4: TOFU keyed by `hid` | the key's SPKI hash, pinned at pairing (or a QR code's `k=`), refused on change with no override; only a rotation signed by the pinned key is followed | stricter, and consistent |
| 4 | 7.3 `ok`, 7.2 `hello.hub` | `ok.hub_key` (the SPKI hash the connection met) on `auth.token`, `auth.pair`, `auth.rotate` over TLS; `hello.hub.key_next {spki, prev_spki, sig}` during a rotation | optional fields, ignored |
| 5 | 6, 7.1 | `auth.rotate`, idle token expiry (180 days), close code 4003 `tls_required`, error `tls_required`; `auth.pair2` (PAKE, SECPAKE) | additive |
| 6 | 6.2 | `auth.pair` over plaintext refused with `tls_required` in `on`, unless upstream compatibility | upstream consoles pair only with `--upstream-compat` |
| 7 | 4, 3.2 | in `require`: HTTP `426` with the fingerprint; a WebSocket upgraded, then closed 4003 | upstream consoles cannot use a `require` hub; `--upstream-compat` serves them |
| 8 | 3.2 | `/dspi/v1/info` gains `tls` (served), `tls_spki` (the pin, base64url), `upstream_compat`, `proxy` | readable over plaintext in `on` |
| 9 | 4 | plaintext requests must name the hub (its addresses, `.local` name, `--host-name`), else `421` | browsers only |
| 10 | 6.2 | a token once used over TLS is refused over plaintext (`tls_only`) | a client not updated keeps working until its token meets TLS |

## Firmware rollout

<!-- WP P3FRZ (PLAN-P3 F6, OD-P2b, OD-P3e, OD-P3n) -->
The hub can update the firmware of the DSPis on its own USB with nobody at the rack:
an Admin starts it from a console (the web console's rollout screen, `uconsole api`,
or this hub's prompt), one device at a time, the settings exported to the hub's
snapshots first, the firmware guard asked with nobody to type, the device checked
after it restarts, and an alarm when it does not come back. What it does step by
step: [hub-api.md](hub-api.md) section 21; why it is safe enough:
[security.md](security.md) (The hub's firmware rollout).

| Switch | Does | Default |
|---|---|---|
| `--rollout` | serves the rollout (`fw.*`, cap `rollout`) to Admins, **over TLS only**; refused at the start when the hub would serve no TLS (`--tls off`, or `--loopback` without `--tls on`/`require`) | off |
| `--allow-unverified-firmware` | with `--rollout` only: images the release's manifest does not list may be flashed too | off |

- `--rollout` is **off by default**. Without it no `fw.*` message is served and no
  console shows the rollout. With it the hub prints `Firmware rollout: on, for admins
  over TLS only (verified images only)` at the start.
- `--allow-unverified-firmware` is **off by default**, the owner's bench switch.
  Without it only an image whose name and bytes the release's manifest lists
  (`verified`) goes on a device. With it the hub starts with a **WARNING** block
  (also on standard error with `--json`, so it is never only in a log):

  ```text
  WARNING: --allow-unverified-firmware. This hub flashes firmware images the release does
  not list: an admin's console can put any image it uploads on the DSPis on this machine's
  USB (the firmware guard still refuses an image the board's core voltage does not allow).
  Use it on a bench, never at a venue.
  ```

  The image list shows each image `verified` or `NOT VERIFIED` either way.
- Images go into the hub's store (`firmware/`, The store below) by an Admin's upload
  from a console, or on the hub's machine: `firmware add <file.uf2>` at the running
  hub's prompt, or `uconsole hub firmware add <file.uf2>` with the hub stopped
  (`uconsole hub firmware list` lists them). The same checks apply either way.
- **Stopping the hub during a flash.** Ctrl-C or SIGTERM never cuts a flash short: the
  hub waits for the flash in progress to end, which can take up to about 60 s, and
  then stops. A rollout stopped while it waited for a board or a restart ends there;
  at the next start its record reads `failed` at the step it was at, with the
  device's `rollout_failed` alarm raised and the members after it `not_tried`. A
  service manager's stop timeout must allow those 60 s: the systemd unit and the
  launchd agent in the release's `service/` folder give the hub 90 s
  (`TimeoutStopSec=90`, `ExitTimeOut` 90); keep that if you write your own. A
  Windows task ended from Task Scheduler is killed at once: do not end it while a
  rollout is flashing.
- The hub never raises a board's core voltage, never flashes two boards at once,
  never mounts a bootloader drive and never retries a flash: see section 21.4 of
  hub-api.md.

## Network and firewall

| What | Port | Notes |
|---|---|---|
| DSPi Link (WebSocket) and `GET /dspi/v1/info` | TCP 11915 (`--port`) | the one port clients need |
| Discovery | UDP 5353, mDNS, `_dspi._tcp` | optional: clients can be given the address; Windows advertises through its DNS Client service |
| The web console, this machine only (`uconsole web`) | TCP 11916, loopback | not reachable from the network |

- `--loopback` listens on this machine only and does not advertise.
- `--web` also serves the web console and the hub's page to browsers on the
  network, on the Link port (`https://<hub>:11915/console/`).
- <!-- WP SECDOC --> Everything on the port is TLS 1.3 by default (Encryption,
  below): nothing a client sends, PIN and token included, can be read on the
  network. Still keep the hub off the internet: never behind router port
  forwarding; use a VPN for remote access (spec 6.4). With `--tls off` or
  `--upstream-compat`, unencrypted connections can be read by anyone on the
  network: then pair on a network you trust only.

To write: firewall commands per OS (firewalld, ufw, Windows Defender Firewall,
the macOS application firewall); VLAN and routed-network notes (mDNS does not
cross routers).

## The store

`hub-store.json` in the store folder (`--store`; default the user's local data
folder, `uconsole/hub`): the hub id, its name, the access mode, the paired
clients (name, role, times, token hash, and `tls_only` once a token has been
used over TLS) and device names; limits profiles, PIN hashes and pinned modes
(P1HL). One hub per store.
A diagnostics bundle includes it with the token hashes removed
([diagnostics.md](diagnostics.md)).

<!-- WP SECDOC --> Beside it (the folder is 0700, every file 0600 on Unix):

| Path | What |
|---|---|
| `tls/key.pem` | the hub's TLS key; never in a diagnostics bundle |
| `tls/cert.pem` | its self-signed certificate (`uconsole hub tls cert`) |
| `tls/identity.json` | the names the certificate was issued for, and a rotation's announcement |
| `proxy-pins.json` | `uconsole web` only: the keys it pinned for other hubs |

A corrupt or missing `key.pem` is set aside as `key.pem.bad-<time>` and a new key is
made, with a loud log line: every paired client then reports that the hub's key
changed and must pair again. Back up `tls/` with the store if the hub is moved to
another machine.

<!-- WP P3DOC --> The fleet's files (PRO phase 3, [hub-api.md](hub-api.md) sections 19 to 21; built by the WP
named), in the data folder (`--data`; by default the store folder), 0600 and written atomically; a file the
hub cannot read is kept aside as `<name>.bad-<time>` and the hub goes on with what it could read:

| Path | What | Bound | Diagnostics bundle |
|---|---|---|---|
| `groups.json` | the groups: id, name, note, members (P3GR, built) | 64 groups of 64 members | `hub/groups.json` |
| `rollout.json` | the firmware rollouts' records: target, image name and hash, steps and their texts, who started it (P3RO, built) | the newest 64 records | `hub/rollout.json` |
| `firmware/` | the firmware images an Admin uploaded or `uconsole hub firmware add` put in, for a rollout (P3RO, built) | 4 images of at most 8 MiB | never |
