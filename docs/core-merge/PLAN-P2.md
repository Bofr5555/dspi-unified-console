# PRO phase 2: the hub as a server

*WP PLANP2, 2026-11-15, written on `main` at `f60845ef` (now `ab475933`, docs only since); revised after the Fable review
`work/review-planp2/REPORT.md` (accept with changes: M1–M3, S1–S9, N1–N8 applied; decisions recorded in section 4). The spec is PRO1's report (`work/pro1/REPORT.md`: section 2 rows F1, F2,
M1, M3, M4, S6, I1, I2, I5, I6, I7; section 4 phases 2 and 3; section 5; section 6; section 7 questions 5 to 8) with the
owner's decisions below. Phase 1 (`PLAN-CLIENTS.md` section 5.9) is the structure this file copies. This file is the plan
only: no code, no edits to `PLAN-CLIENTS.md`.*

**Goal.** A hub that keeps running with no console open and is useful on its own: it knows every DSPi it has ever seen, can
make one blink, keeps a shared history of what happened and who did it, raises alarms and tells other systems about them,
holds the shared copy of each device's snapshots, runs schedules, and lets other programs (a script, a venue controller,
Home Assistant) read and change parameters by name under the roles and limits it already enforces. **Hub-first and
core-first:** the logic lands once in `crates/dspi-hub` (and in `dspi-core`/`dspi-session` where a client needs it); each
client adds one page or dialog. The multi-device and admin pages are the **web console served by the hub**; Qt, Swift and
Android get only what a single-device user needs (shared snapshots, the hub's history, alarm banners, Identify).

**Owner decisions this phase builds on:**
- Three roles only: **Admin, Control, Viewer** (no fourth). Pairing with a PIN gives at most Control; Admin is given on the
  hub's machine (WP PAIRADM, `docs/integrator/hub.md`).
- Snapshots are local today; phase 2 adds **a shared copy on the hub, and the hub's copy wins** (OD-P1b).
- Limits are enforced only when a profile exists (OD-P1 and P1LM/P1HL). Every phase-2 write path (API, REST, MQTT,
  schedules, hub-side recall) goes through the same limits and lock checks; none bypasses them.
- **DSPi Link is not encrypted**, and hub-identity pinning is not to be added (owner). Phase 2 states plainly what that
  means for tokens, API keys and webhook secrets; it adds no TLS on the Link port.
- The 1.20 V core-voltage rule and the `OVERVOLT` word stay as they are. No phase-2 surface reaches `EXT_CORE_VREG` (it has
  no registry path; the extension opcode is Admin-only in the policy, and the API refuses raw opcodes).
- No Qt or Swift tabs: the multi-device view is the web console served by the hub.
- Naming: **Snapshot** is a host whole-device state; **Scene** is the hub's audio-link system scene (SW2). "alink" in code
  for the audio link, "DSPi Link" (or "dlink") for the network link, never bare "link". No host-device nouns in user text.

**What exists today (checked in the code on `f60845ef`):**

| Phase-2 item | State | Builds on |
|---|---|---|
| Hub control plane | JSON over `/dspi/v1` (WebSocket). `server.rs::on_text` chains `lock::handle_message`, `apply_guard::handle_message`, `limits::handle_message`, then `on_request` (`device.*`, `poll.*`, `auth.*`, `system.*`). Every new family can be one more module in that chain | `crates/dspi-hub/src/server.rs` (1656–1900) |
| One timer thread | `hub.rs::system_poll_loop` already wakes for the audio-link poll, the guard deadlines (`apply_next_due`) and limit holds (`limits_next_due`) | `crates/dspi-hub/src/hub.rs:1324` |
| Device registry | Online and offline-for-60 s devices by handle and serial, with identity (serial, vid, platform, firmware, beta, outputs, board name) and names; forgotten after the grace | `registry.rs` (`DeviceIdentity`, `RegistryEvent`, `OFFLINE_GRACE`) |
| Store | `hub-store.json`: hub id, name, clients (token hashes), device names, the alink system model, limits, PIN hashes, pinned modes; atomic, 1 MiB cap | `store.rs` |
| Roles and policy | Viewer/Control/Admin, command classes from `commands.json`; the extension opcode `0x00` is unlisted, so Admin-only | `policy.rs`, `tests/fixtures/link/policy/commands.json` |
| Write attribution | Notifications attributed to the writing session within 500 ms | `router.rs` (`ATTRIBUTION_WINDOW`) |
| Alarms (alink only) | `AlarmBook`: pure raise/acknowledge/clear by code and subject, 64 cleared kept; `system.alarms`, `system.alarm.ack` | `system/alarms.rs` |
| Hub-side writes with a way back | Guarded apply: image `0xA0`/`0xA1` plus the settings outside it (`apply_extras.rs`), hub-session writes | `apply_guard.rs`, `apply_extras.rs` |
| Snapshot cache | One bulk image per device, kept fresh from `PARAM_CHANGED` (`device.snapshot`); not the host snapshot store | `snapshot.rs` |
| Polls | Shared poll groups per device with budgets; a group is one read whoever asks | `polls.rs` |
| `hub.stats`, `hub.rename`, `fw.install` | Answer `unsupported` (Admin) or `denied` | `server.rs:1886` |
| Event log (client half) | JSON-lines per serial, rotation, several writers under a lock, `EventSink` trait for the hub half, decoders for notifications and the ext events | `crates/dspi-session/src/events.rs`, `crates/dspi-core/src/events.rs`, `docs/integrator/event-log.md` |
| Snapshot store (client) | Folder per serial, `index.json`, atomic, "the index format is one for every client, so a hub's copy can merge with it later" | `crates/dspi-session/src/snapshot_store.rs` |
| Parameter registry | 186 paths with kinds, ranges, units, hazards; `write_param` with the typed setters' guards and `write_refusal` | `crates/dspi-proto/src/registry.rs`, `crates/dspi-core/src/write_param.rs` |
| Typed core calls for hub messages | The pattern to copy: `hub_admin.rs` (`hub_limits_get`, events through `LinkEvents::hub_event`, a listener, `hub_admin_events` on the web) | `crates/dspi-core/src/hub_admin.rs` |
| Firmware facts | Identify (`EXT_IDENTIFY` 0x0004), reset reason and uptime (`EXT_RESET_INFO` 0x0005), clip event (0xE0 behind `EXT_NOTIFY_FEATURES` bit 1) are on `bofr-main` **and** decoded host-side (WP FWX); gain clamps and NaN refusal (FW2) are on `bofr-main` (b964f01) | `crates/dspi-core/src/ext_diag.rs`, `~/prog/DSPi-bofr/firmware/DSPi/{identify,reset_info}.h` |
| Web fleet seam | `FleetBackend`: `installations`, `access`, `hubDevices`, `hubCredentials`, `pinnedMode`; only `LocalBackend` exists | `apps/web/src/fleet/types.ts`, `local.ts` |
| `uconsole` | Links `dspi-core` (WP TMCORE) and `dspi-hub`; `uconsole hub` has flags, a prompt (`pair`, `clients`, `role`, ...) | `apps/terminal/src/hub.rs` |

**Corrections to PRO1 found while checking:**
- PRO1 section 5 lists FW1, FW4 and FW5 as proposals; they are built on `bofr-main` and decoded by the core (WP FWX), and FW2
  is built too. Question 8 is therefore about upstream, not about building them (OD-P2d).
- PRO1 says clip flags are "cleared on read". They are a **sticky latch cleared by `REQ_CLEAR_CLIPS`**
  (`crates/dspi-proto/src/packets.rs:2270`). A poll sees only "clipped since someone cleared": the hub cannot count clips
  per minute from the latch and **must never send `REQ_CLEAR_CLIPS` itself** (it would clear every client's indicator). Clip
  counts come from the 0xE0 event where the firmware has it; without it, the hub knows only "clipped since cleared".
- PRO1 I1 proposes the REST mirror under `/dspi/v1/devices/...`. The Link spec (4.2) reserves `/dspi/` and says "there is
  deliberately no REST API for device control". Phase 2 keeps the REST surface **outside** `/dspi/` (OD-P2a).
- PRO1 F1 says `device.list` gains the offline devices. `device.list` is spec 7.4 (handles, a 60 s grace) and every client
  reads it; the inventory is a separate `inventory.*` family instead, and `device.list` is unchanged.

**Not in this phase:** groups, linked parameters, bulk apply, firmware rollout, the health board and the fleet screens
(phase 3, section 3 below); OSC and the MIDI control map (PRO1 phase 6); several hubs under one page (phase 4).

---

## 1. Goal and scope, item by item

Each row says what the hub does, what it builds on, and what the clients see. "Message" means a JSON control-plane message
on `/dspi/v1`; each new family is additive to Link 1.2, announced in `hello.caps`, answers carry `"v": 1` (the P1HA/P1HL
convention), and unknown types still answer `unknown_type`.

| Item | What the hub does | Builds on | Clients |
|---|---|---|---|
| **F1 inventory** | Every DSPi ever attached, persisted in `<data>/inventory.json`: serial, platform, firmware and beta, board and hardware revision (from `EXT_IDENTIFY` where listed), output count, the names, location, note and tags set by an Admin, first and last seen, online and handle, last reset reason and uptime. `inventory.list`, `inventory.set` (Admin), `inventory.forget` (Admin, offline only), event `inventory.changed`. Bounded (1024 devices) | `registry.rs` events (`Added`/`Changed`/`Removed`), `RegisteredDevice::to_json`, the store's atomic write | Web device table (P2WB1); the core's typed call (P2CC1) |
| **F2 identify** | `device.identify {serial or handle, seconds, tone_output?}` (Control, Admin): the hub sends `EXT_IDENTIFY` OUT as one of its own writes (`INTERNAL_SESSION_ID`, section 2's closed list; the blink whatever the lock, the tone `locked` under another session's lock); refused `unsupported` where `EXT_LIST` lacks 0x0004; at most one request per device per 2 s; the tone only when asked, at the firmware's own -40 dBFS through the output chain | `dspi_session::ext::IdentifyRequest`, `ext_diag.rs` codecs | An Identify button in every client (web P2WB1; Qt, Android, Swift where missing) |
| **M1 event history (hub half)** | One shared log per device in `<data>/events/`, the same file format and lock rule as the clients' (`event-log.md`), so `EventLog::export_in` and the diagnostics bundle read it unchanged. Written from what the hub sees: attach and detach, decoded notifications with the writing session and its client name (`session`, new field `by`), clip events, temperature crossings and counter steps from the health poll, resets with their reason, firmware changes, hub-side recalls, schedule runs, alarms. `event.list {serial, after_id, limit}`, `event.watch {on}` then `event.added`, `event.export` | `dspi_session::events` (`EventRecord`, `EventSink`, `decode_ext_event`), router attribution | Each client's events view gains "On the hub" (P2CC1 call; web P2WB1, Qt/Android/Swift P2QT/P2AN/P2SW) |
| **S6 watchdog events** | The hub notices: a device that stops answering (transfers time out for N s while attached: `unresponsive`), a re-enumeration (offline then online), and on every attach reads `EXT_RESET_INFO` (reason power-on, brown-out, run pin, watchdog, software, debugger, glitch; previous uptime; hang count). A watchdog or glitch reset is an event and raises `device_restarted` (M3); a power-on is an event only | `EXT_RESET_INFO` decode (FWX), registry online/offline | "Restarted at hh:mm (watchdog)" notice in every client (from the alarm and P2CC1) |
| **Health poll** (feeds S6, M3, phase 3's F7) | One hub-internal subscriber per device on the existing poll groups (so a client's meter poll and the hub's are one read): status every 2 s (CPU, clip latch), temperature every 30 s, `EXT_RESET_INFO` (uptime, hang count) every 60 s, all inside `POLL_BUDGET_BPS`. `health.get {serial?}` returns one record per device; `device.facts` on the hub's internal bus | `polls.rs` (shared groups, budgets), the status packet decode | Phase 3's health board; phase 2 shows CPU and temperature in the web device table |
| **M3 alarms** | Device alarms beside the alink ones, on the same `AlarmBook`: `device_offline` (N s), `unresponsive`, `device_restarted` (watchdog, glitch, brown-out), `clip` (0xE0 count per minute over a threshold; latch-only firmware raises "clipped" once until cleared), `temperature` (over a threshold), `cpu` (over 90 % for N s), `schedule_failed`, `clock_unset`. Rules (thresholds, on/off, an opt-in "mute on clip storm" action) in `<data>/alarms.json` (Admin). `alarm.list` (every role; the alink alarms included with `source: "alink"`), `alarm.ack` (Control, Admin), `alarm.rules.get`/`set`, event `alarm.changed`. `system.alarms` and `system.alarm.ack` keep working | `system/alarms.rs` (`AlarmBook`, `AlarmState`) | Alarm banner and list in every client (P2CC2's call); web board (P2WB1) |
| **M4 notifications out** | Opt-in targets in `<data>/outbound.json` (Admin): a webhook (JSON POST), an ntfy-style push (plain-text POST to a URL), and MQTT publish (P2MQ); each picks event kinds (alarm raised/cleared, restart, schedule ran or failed). A bounded queue (64 per target), backoff, never on a device worker; HMAC-SHA256 signature header with a per-target secret; no target by default, no cloud of ours | the internal bus (P2SV), `sha2` (in the graph) | Web hub settings (P2WB2) |
| **I1 parameter API** | `param.list {serial}` (registry rows: path, label, plain, unit, kind, range, indices and their layout on this device, writable, hazard), `param.get {serial, path, idx}`, `param.set {serial, path, idx, value}`, `param.get_many`. Writes run through **`dspi-core`'s `write_param` inside the hub** (feature `api`), on a core opened over a child session of the caller (its role, its lock rights, its own session id for attribution), so every typed setter's guard, `write_refusal`, the limits and the lock are the same as in the consoles; the router judges the transfers as for any client | `dspi-core` `write_param.rs`, `local.rs` (the in-process transport pattern), `policy.rs`, `limits.rs` | Terminal `uconsole api` (P2TM); integrators |
| **I1 REST mirror** | Off by default (`--rest`). `/api/v1/...` (outside `/dspi/`), a thin translator onto the same handlers: `GET /api/v1/devices`, `GET/PUT /api/v1/devices/{serial}/params/{path}?idx=0,1`, `POST .../presets/{n}/load`, `POST .../snapshots/{id}/recall`, `GET .../events?after=`, `GET /api/v1/alarms`, `POST /api/v1/alarms/{id}/ack`, `GET /api/v1/health`. Bearer API keys (OD-P2e). Any request with an `Origin` header is refused (no browser needs REST: the web console uses the wasm binding). Rate-limited per key | `server.rs` `answer` (HTTP head parsing, `respond`), `auth.rs` tokens | Integrators; `docs/integrator/hub-api.md` |
| **I2 presets and snapshots by name** | `preset.list {serial}` (slots with names), `preset.load {serial, slot or name}`, `preset.save` (Admin, with `"confirm": true`), `snapshot.recall {serial, id or name}` from the hub's store; audio-link scenes stay `system.scene.apply {scene}` (one word per concept) | `dspi-core` presets, P2SS, `system/scene.rs` | API, REST, schedules, MQTT |
| **Hub snapshot store** (OD-P1b) | `<data>/snapshots/`, the client store's own format (`snapshot_store::SnapshotStore`, one folder per serial, `index.json`). `snapshot.list {serial}`, `snapshot.get {id}`, `snapshot.put {serial, document}` (Control, Admin), `snapshot.rename`/`note` (Control, Admin), `snapshot.delete` (Admin), event `snapshot.changed`. Hub-side recall through the hub's core (P2PA's `recall.rs`) with an optional guard (P1HA). **The hub wins:** the same id on both sides with different content shows the hub's; the local one is kept as a local copy, never overwritten silently | `snapshot_store.rs`, `dspi-core` `snapshot.rs` (recall of a document's text), `apply_guard.rs` | Snapshots window/page in every client gains an "On the hub" section (P2QT, P2WB2, P2AN, P2SW) |
| **I5 scheduling** | Rules in `<data>/schedule.json` (Admin): days of the week and a local time, or once at a date and time; an action list (`preset.load`, `snapshot.recall`, `system.scene.apply`, `param.set` for volume and mute) and targets (serials; `group:<id>` reserved for phase 3). Runs as the principal "Schedule: <name>" with the Admin role, judged by limits (images held after the fact, as P1HL does for an Admin). Clock rules in OD-P2h. `schedule.list` (every role), `schedule.set`/`delete`/`run_now` (Admin), event `schedule.ran {id, results}`. Logged in the history; a failure raises `schedule_failed` | the action executor (P2PA), P2SS, the timer seam (P2SV) | Web hub settings (P2WB2); `uconsole` (P2TM) |
| **I6 home automation** | MQTT (opt-in) with Home Assistant discovery: per device a number for master volume (range from the registry), a switch for mute, a select for the preset (names from the device), a button for Identify, sensors for temperature and CPU, a binary sensor for clip; availability by last will. `set` topics act as an API key's principal (Control by default), so limits, the lock (whose scope is exactly volume, mute and preset) and attribution apply | P2MQC, P2MQ, the action executor | Web hub settings (P2WB2) |
| **I7 scripting** | No script engine in the hub (OD-P2g). `uconsole api <type> [json]` calls any hub message and prints the answer (`--json`, exit codes); `uconsole hub` gains `--rest`, and the prompt gains `inventory`, `alarms`, `schedule`, `api-key`; integrator examples for shell, Python (REST) and Home Assistant | `crates/dspi-cmd`, `apps/terminal/src/hub.rs`, P1TM's snapshot commands | Terminal |
| **`hub.stats`, `hub.rename`** | Spec 7.9 as written: `hub.stats` (Admin) `{sessions, uptime_s, devices: [{handle, serial, cmds, errors, avg_rtt_ms, notify_dropped}]}` (`serial` is our addition, additive, listed in `hub-api.md`'s deviations); `hub.rename {name}` (Admin) saves, updates `hello` and `/dspi/v1/info`, re-advertises over mDNS | `store.rs::set_name`, `advertise.rs`, the device worker | Web device table (P2WB1, Admin) |

**What the hub never does in phase 2:** send `REQ_CLEAR_CLIPS`; write the core voltage; replay a write; queue a write for a
device that is offline (a scheduled action on an offline device fails and is reported, never retried later); flash firmware
(`fw.install` stays `unsupported`); send anything to the internet unless an Admin entered the URL; write as
`LOCAL_SESSION_ID`, `INTERNAL_SESSION_ID` or `GUARD_SESSION_ID` on anyone's behalf. **The hub's own writes are a closed list
of three** (section 2): the guarded apply's revert, identify and mute on clip storm; every other write is a principal's,
in a session from the remote range, judged by role, limits, lock and the voltage rule.

---

## 2. Work packages

**Shared rules for every P2 WP.** `PLAN-CLIENTS.md` section 1 and the brief `~/.cache/dspi-build/work/wave1-brief.md` (not in
the repo). In particular: containers only, no tmpfs (hub tests put their data folder under
`~/.cache/dspi-build/work/wp-<ID>/`), no devices, scoped checks, commit as the global identity with the trailer. Hub rules:
- **One module per family.** After P2SV, a WP adds a new file under `crates/dspi-hub/src/` and one registration line in
  `service.rs`'s list under `// -- WP <ID> --`, plus its `mod` line in `lib.rs` (shared wiring, additive, listed in the
  report). It edits `server.rs`, `hub.rs` or `relay.rs` only if its entry says so.
- **Pure state, thin glue** (as `system/alarms.rs` and `registry.rs`): the caller passes the clock; state machines are
  tested without threads or sockets; one conformance test per family through the real server
  (`tests/conformance/`). Wave-B WPs compile against P2SV's typed bus messages, never against each other's modules.
- **Every write is judged.** API, REST, MQTT, schedule and hub-side recall writes go through the router as a session with a
  role, so `policy.rs`, `limits.rs` and the lock see them. A test per WP shows a Viewer refused and, where a write can cross a
  limit, a limit refusing it.
- **The session-id rule (review M2).** A *principal* (an API child session, a REST key's session, an MQTT key's session,
  `Schedule: <name>`) is a `Session` opened through `open_session(role, client_id)`, whose id comes from `next_session_id`
  (the remote range, `hub.rs:581-603`). **Never** `LOCAL_SESSION_ID`, `INTERNAL_SESSION_ID` or `GUARD_SESSION_ID`: those skip
  limits (`limits::admit` exempts LOCAL and GUARD, `limits.rs:1093`), the voltage rule (the router skips it for LOCAL,
  `router.rs:395`) or locks (INTERNAL passes them, `router.rs:295-301, 517-555`). A transport that submits as a principal
  must not copy `local.rs`'s `session_id()` (which answers `LOCAL_SESSION_ID`, `local.rs:222`).
- **The hub's own writes, a closed list (review M3, OD-P2o).** The hub originates exactly three writes of its own, all as
  `INTERNAL_SESSION_ID` (Admin): the guarded apply's revert (exists, P1HA, as `GUARD_SESSION_ID`), **identify** (P2ID) and
  **mute on clip storm** (P2AL). Identify blinks whatever the lock; its tone answers `locked` while another session holds the
  device lock. Mute on clip storm passes the lock (protective, mute only, never an unmute). Each is recorded in the history
  with the caller or the rule's name in `by`. Any other write is a principal's, under the session-id rule.
- **Bounded everything:** files, queues, records per message, rate per session; a corrupt data file never stops the hub
  (it is read entry by entry, kept aside as `<name>.bad-<time>`, and the hub runs with defaults and a notice in the log;
  parse errors are values, never panics). Caps (review S3): inventory 1024 devices; schedule rules 256 with 16 actions each;
  outbound targets 16; `alarms.json` 64 KiB; history and snapshots per OD-P2k; bus queues per subscriber.
- **Store files:** `<data>` is the hub's `--store` folder (`docs/integrator/hub.md`, "The store"), one per hub. Each family
  its own file there, written atomically, 0600 on Unix; `hub-store.json` is not touched by phase-2 WPs except P2RS (API keys
  are paired clients). **Diagnostics (review S6):** `crates/dspi-core/src/diag.rs` copies only `hub-store.json` today; the WP
  that adds a data file adds it to the bundle **redacted** (secrets replaced by "set"/"not set"), with a test that the bundle
  has the file and no secret: P2IN `inventory.json` (P2-B, the wave's only `diag.rs` edit); P2AL `alarms.json` and
  `outbound.json` (P2WH's file, added in P2-C so P2-B has one owner), P2SC `schedule.json`, P2MQ `mqtt.json` (P2-C, one
  marked block each).
- **Secrets are generated, never typed (review S2):** API keys and webhook secrets are made by the hub's CSPRNG
  (`getrandom`) and shown once; only a hash (keys) or the secret in the 0600 file (webhooks) is kept.
- **Docs:** P2DOC writes `docs/integrator/hub-api.md` with one section per family; each WP fills its own section (a
  disjoint section, so no textual overlap) and marks it built.
- Core bindings (P2CC1, P2CC2): binding wiring is shared and additive; run `ci/drift.sh write`, regenerate the ABI anchor and
  the Android contract, classify new methods in `WriteGuardedCore`.

Sizes are this project's units (one unit is about one hour of agent time with review). Estimates.

### Core and hub WPs

**P2SV: The hub's service seam.** [TODO] Size 2. Wave P2-A. **Owns:** new `crates/dspi-hub/src/service.rs` (the trait and
the list), new `crates/dspi-hub/src/bus.rs` (the internal event bus and its typed messages), new
`crates/dspi-hub/src/clock.rs` (wall and monotonic time, a test clock); the one-time edits in `server.rs` (the `on_text`
chain runs the services after `limits`; `hello.caps` gains each service's caps; the `hub.stats`/`hub.rename` arm leaves
`unsupported` to the services, `fw.install` stays), `hub.rs` (`system_poll_loop` takes the earliest `next_due` of the
services and runs their `run_due`; device online, offline and removed reach the services; `HubConfig` gains `data_dir`,
defaulting to the network store folder, and a `ServiceConfig` with each optional service off), `relay.rs` (generalises the
existing `set_tap`/`tap_packet` of the alink system, `relay.rs:142-150`, into a read-only tap: every notification packet,
with serial and attributed session, onto the bus; the alink tap keeps working), `lib.rs`. **Implements:** `trait HubService
{ name, caps, handle_message(hub, session, value) -> Option<String>, next_due, run_due(now), on_device(event) }`; the bus is
bounded per subscriber (drop oldest, count drops, the count readable by the subscriber) and never blocks a device worker or
the relay; a session label (client name and `cid` for a session id) for attribution; a helper that opens a principal
session by the session-id rule. **Typed bus messages (review S1)**, defined here so wave B builds against shapes:
`DeviceOnline/Offline/Removed {serial, handle}`, `Notification {serial, session, by, packet}`, `Facts {serial, identify?,
reset_info?, fw}`, `Health {serial, cpu, clip_mask, temp_c, uptime_s, hang_count}`, `Unresponsive {serial, since}`,
`Alarm {code, subject, state, text, at}`, `ScheduleRan {id, results}`, `Event {serial, record}`. No behaviour change for
existing messages. **Done when:** hub tests with a test service: it answers its message type, is consulted after `limits`,
appears in `hello.caps`, gets timer calls at its due time (test clock), sees device online and offline and a notification
with the right serial and session; a slow bus subscriber does not delay a command and sees its drop count; a principal
opened by the helper has a remote-range id; the alink tap still feeds the system; the existing conformance suite passes
unchanged. **Deps:** none. **Clients:** none.

**P2DOC: The hub API contract.** [TODO] Size 1. Wave P2-A. **Owns:** new `docs/integrator/hub-api.md` (one section per
family: inventory, identify, health, events, alarms, outbound, params, presets, snapshots, schedules, MQTT and Home
Assistant, REST, stats), `docs/integrator/parameter-api.md` (points to it), `docs/integrator/README.md` row,
`docs/GLOSSARY.md` rows (inventory, health, alarm rule, schedule, API key; Snapshot and Scene restated). **Implements:**
every message's request and answer shape, roles, events, error codes (`denied`, `bad_request`, `unsupported`, `not_found`,
`conflict`, `rate_limited`, `offline`, `locked`, the P1HL `limit_exceeded`/`installer_lock`/`image_refused` refusals), the
multi-target result shape `results: [{serial, ok, code?, msg?}]` that phase 3 reuses, addressing by serial (handle also
accepted where Link has it), plain JSON values with the registry's kind deciding, versioning (caps, `"v": 1`, additive
only), the REST mapping table, the unencrypted-network warning in one place, and a section **"Deviations from the DSPi Link
spec" (review S5)**: `/api/v1/` on the Link port against 4.2's "Nothing else is defined" and "no REST API" (one handler set
keeps that sentence's reason, one path for serialisation, locking and attribution, true), the `--web` pages and the console
under `/console/`, and `hub.stats`'s added `serial` in each device row. Marked "draft" per section until its WP marks it
built. **Done when:** every message type `t` in section 1 has a row (the list of types is in the report); the deviations
section exists; the no-host-noun check passes on its user-facing strings. **Deps:** none. **Clients:** none.

**P2EVW: A multi-device event writer.** [TODO] Size 1. Wave P2-A. **Owns:** new `crates/dspi-session/src/events/store.rs`
(and its tests), the `EventKind` enum and the record's optional fields in `crates/dspi-session/src/events.rs` (under a
marker). **Implements:** `EventStore`: one writer for many serials in one folder (the hub has many devices at once;
`EventLog` opens one device at a time), the same file names, rotation, lock rule and id rule; per-device and total size
caps (OD-P2k: `Limits { max_file_bytes: 512 KiB, rotated_files: 3 }` per device, 128 MiB in all); `forget(serial)` removes a
device's files. New kinds `unresponsive`, `alarm`, `schedule`, `access`, `events_dropped` (an older reader reads them as
`unknown`, as the format says); new optional field `by` (the client's or rule's name). **Done when:** tests: several serials
interleaved, each file's ids rising; a second writer (an `EventLog` of a client on the same folder) never writes the same id;
the total cap removes the oldest rotated file first; an old build's reader reads the new kinds as `unknown`; files under
`~/.cache/dspi-build/work/wp-P2EVW/`. **Deps:** none. **Clients:** none.

**P2MQC: An MQTT 3.1.1 client.** [TODO] Size 1.5. Wave P2-A. **Owns:** new `crates/dspi-hub/src/mqtt/codec.rs` and
`crates/dspi-hub/src/mqtt/client.rs` (and `mqtt/mod.rs`), not yet registered as a service. **Implements:** the packets the hub
needs (CONNECT with will, CONNACK, PUBLISH QoS 0 and 1 with PUBACK, SUBSCRIBE/SUBACK, PINGREQ/PINGRESP, DISCONNECT), pure
encode and decode with length limits; a blocking client over `std::net::TcpStream` on one thread with keep-alive, reconnect
with backoff, a bounded outgoing queue, username and password; no TLS (OD-P2f), no async runtime, no new crate. **Done
when:** codec tests against fixed byte vectors and fuzzed lengths (proptest is a dev-dependency already), no panic on any
input; a client test against an in-test fake broker: connect, publish retained, subscribe, a dropped broker reconnects and
re-subscribes, the queue drops the oldest when full. **Deps:** none. **Clients:** none.

**P2SS: The hub's snapshot store.** [TODO] Size 1.5. Wave P2-A (moved from B, review M1). **Owns:** new
`crates/dspi-hub/src/snapshots.rs`, the folder `<data>/snapshots/`. **Implements:** a pure store over
`snapshot_store::SnapshotStore` plus message handlers `(role, &Value) -> Option<String>` for the messages of section 1 except
recall, **not yet registered as a service** (P2MQC's pattern; P2PA registers it in P2-B): `snapshot.put` checks that the
text parses as a document with `meta.kind = "snapshot"` and the serial, limits a document to 256 KiB, 200 snapshots per
serial and 64 MiB in all (OD-P2k), and keeps the client's id (capture-time ids are unique, so the same snapshot keeps one id
on both sides). **Done when:** tests on the handlers: put, list, get, rename, note, delete with roles (Control may put and
rename, only Admin deletes, OD-P2j); a put with another device's serial refused; a torn `index.json` lists from the
documents; names with any Unicode; caps enforced with `conflict`; store under `~/.cache/dspi-build/work/wp-P2SS/`.
**Deps:** none. **Clients:** P2CC1, every client's Snapshots window.

**P2HP: The health poll and device facts.** [TODO] Size 1.5. Wave P2-B. **Owns:** new `crates/dspi-hub/src/health.rs`.
**Implements:** on attach, the hub reads `EXT_LIST`, then `EXT_IDENTIFY` IN (board, hardware revision, chip) and
`EXT_RESET_INFO` where listed (reads, as `INTERNAL_SESSION_ID`; no writes) and puts `Facts` on the bus; then a hub-internal
subscriber on the existing poll groups (status every 2 s, temperature every 30 s, reset info every 60 s; budgets as any
session's), deriving CPU, the clip latch (never cleared by the hub), temperature, uptime, hang count and an uptime that went
backwards (a reset seen between reads), as `Health`. A device that times out for 10 s while attached is `Unresponsive`.
`health.get {serial?}` (every role). **Done when:** tests on the demo device through the hub: the facts of a device with and
without the ext IDs; a client's meter poll and the health poll are one group (transfer count); no `REQ_CLEAR_CLIPS` is ever
sent (a transfer-log test over a whole run, plus a grep-level test that the hub's source never names it outside the
router's classification); the poll stops with the device; an unresponsive device (the mock transport's timeouts) is
reported once and recovers. **Deps:** P2SV. **Clients:** web device table (P2WB1).

**P2IN: The inventory.** [TODO] Size 1.5. Wave P2-B. **Owns:** new `crates/dspi-hub/src/inventory.rs`, the file
`<data>/inventory.json`, its entry in the diagnostics bundle in `crates/dspi-core/src/diag.rs` (under a marker; the one
`diag.rs` owner of P2-B). **Implements:** F1 as in section 1, from registry events and `Facts`; `inventory.list`,
`inventory.set {serial, location?, note?, tags?}` (Admin), `inventory.forget {serial}` (Admin; refused while online; also
forgets the device's history files through P2EVW's `forget` and its hub snapshots only when asked: `with_data: true`),
`inventory.changed`. Records are bounded (location and note 256 characters, 16 tags of 32); 1024 devices, the oldest
offline ones beyond that are dropped with a log line. `device.list` is unchanged. **Done when:** tests: a device seen,
detached and restarted with the hub keeps first-seen and gains last-seen; a firmware change updates the record and puts a
`firmware` event on the bus; Admin set, Control refused, Viewer reads; forget while online refused; a torn file reads what it
can; the diagnostics bundle carries `inventory.json`. **Deps:** P2SV, P2EVW (forget). **Clients:** web (P2WB1), core call
(P2CC1).

**P2ID: Identify.** [TODO] Size 0.5. Wave P2-B. **Owns:** new `crates/dspi-hub/src/identify.rs`. **Implements:** F2 as in
section 1, as one of the hub's own writes (`INTERNAL_SESSION_ID`, the shared rule): Control and Admin may ask, since the
extension opcode is Admin-only in the policy and the hub, not the caller, sends it; blink whatever the lock, the tone
`locked` while another session holds the device lock; one history record with the caller in `by`; the answer is the
device's `IdentifyStatus` after the write. **Done when:** tests on the demo device (its `fake/diag.rs` handles 0x0004):
blink, stop (seconds 0), a tone output out of range refused, the tone refused `locked` under another session's lock while
the blink passes, the rate limit, Viewer refused, `unsupported` on a device without 0x0004 and no transfer sent, the history
record. **Deps:** P2SV (P2EV's history through the bus; the test reads the bus). **Clients:** Identify buttons (P2WB1,
P2QT, P2AN, P2SW).

**P2HS: `hub.stats` and `hub.rename`.** [TODO] Size 0.75. Wave P2-B. **Owns:** new `crates/dspi-hub/src/stats.rs`; the
per-device counters (commands, errors, round-trip time, dropped notifications) in `crates/dspi-hub/src/device.rs` under a
marker. **Implements:** spec 7.9 as in section 1 (`serial` added to each device row: additive, listed in the deviations
section). **Done when:** tests: counts rise with commands, errors with a mock failure, an average round trip is reported;
`hub.rename` saves, a new `hello` and `/dspi/v1/info` carry it, the mDNS announcement is renewed (the HB5 test's loopback
browser); Control and Viewer `denied`. **Deps:** P2SV. **Clients:** web (P2WB1, Admin).

**P2EV: The hub's event history and watchdog events.** [TODO] Size 2. Wave P2-B. **Owns:** new
`crates/dspi-hub/src/history.rs`, the folder `<data>/events/`. **Implements:** M1's hub half and S6 as in section 1, from the
bus: registry events, attributed notifications decoded with `decode_ext_event` and the notification decoder (`by` from the
session label), `Facts` (a reset with its reason; a firmware change), `Health` steps (temperature crossing 5 degree bands,
counters stepping), `Unresponsive`, coalescing as the format does; bus messages it dropped become one `events_dropped {n}`
record (review S3), never silence. Messages `event.list`, `event.watch`, `event.added`, `event.export` (Admin: the whole
file). A client's own core keeps its local log as today. **Done when:** tests through the hub on the demo device: a write by
client A shows `session` and `by` = A's name in the record client B reads; a slider sweep is one coalesced record; a watchdog
reset records `reset` with `watchdog` (injected `Facts`); detaching and attaching records `disconnected`/`connected`; a
flooded subscriber writes `events_dropped`; `event.list` pages by id; a Viewer may read; the files read with
`EventLog::export_in`. **Deps:** P2SV, P2EVW; P2HP soft (its messages are P2SV's typed `Facts` and `Health`, injected in
tests). **Clients:** P2CC1, every client's events view.

**P2PA: The parameter API, the action executor and hub-side recall.** [TODO] Size 4. Wave P2-B. **Owns:** new
`crates/dspi-hub/src/api/` (`mod.rs`, `params.rs`, `presets.rs`, `exec.rs`, `transport.rs`, `recall.rs`; P2SR of the first
draft folded in, review M1), the registration of P2SS's handlers as the `snapshot` family beside `api` (two lines in
`service.rs`), `crates/dspi-hub/Cargo.toml` (a feature `api` = `dep:dspi-core`, as `app`; the only `Cargo.toml` edit this
wave), `crates/dspi-hub/src/session.rs` (a child session: same role and client, its own id from `next_session_id`; under a
marker). **Implements:** I1, I2's preset half and hub-side snapshot recall as in section 1. `transport.rs` is a `Transport`
that submits as a given principal session, modelled on `local.rs` **but answering the principal's own id from
`session_id()`, never `LOCAL_SESSION_ID`** (the session-id rule); `exec.rs` keeps one `DspiCore` per (principal, device)
opened lazily over a child session and closed after 60 s idle or when either goes, and is the **action executor** every
later family uses (`param.set`, `preset.load`, `snapshot.recall`, later `system.scene.apply`). Each open is a full attach
(probe and image read): a schedule on eight devices opens eight cores, and the idle close bounds them (review N8).
`param.set` refuses what `write_refusal` refuses and raw opcodes, never takes a gated path (`preset.save`, `preset.delete`,
`dev.save`, `dev.save.io`) unless the request is Admin's and carries `"confirm": true`, which is the API's explicit
confirmation (the hub's core then mints and spends the token inside, P1TK's discipline), and answers the value read back.
`recall.rs`: `snapshot.recall {serial, id or name, guard?}` from P2SS's store through the executor's core
(`DspiCore::snapshot_recall_document`: the image and the settings outside it, in its order) as the caller's child session,
so P1HL's rule holds (a Control client's image on a limited or locked device is `image_refused`); `guard: true` begins a
P1HA guard first and answers its id; a recall on a dropped device sends nothing. Rate: the caller's `max_inflight`. **Done
when:** tests through the real server on the demo device: list, get, set for a float, a bool, an indexed EQ field and a
choice by name; a set outside the registry range is `bad_request`; **a Control child session's set past a limits profile is
`limit_exceeded` and nothing is sent; a raw `EXT_CORE_VREG` write from a child session is `DENIED` by the worker** (the two
session-id tests); the child session's id is in the remote range; on a locked device a Control client may set volume, mute
and preset only; a Viewer may get, not set; a write by the API shows in another client's mirror as a foreign write (echo
rule) and in its undo as "changed elsewhere"; `preset.load` by name; recall restores every value (bulk and non-bulk), is
refused for Control on a limited device and accepted then held for Admin, a guarded recall reverts on timeout, and is
recorded in the history as a `recall` with its lines; the core opened for the API is closed when its client goes. **Deps:**
P2SV, P2SS. **Clients:** P2TM, integrators, every client's "Recall on the hub" (P2CC1).

**P2WH: Webhooks and push.** [TODO] Size 1.5. Wave P2-B. **Owns:** new `crates/dspi-hub/src/outbound/` (`mod.rs`,
`http.rs`, `hmac.rs`), the file `<data>/outbound.json`. Its diagnostics entry is added by P2AL in P2-C (one `diag.rs` owner
per wave); until then the bundle simply lacks the file, which holds no secret it could leak. **Implements:** M4's HTTP half
as in section 1: a minimal HTTP/1.1 POST over `TcpStream` (`http://` only, OD-P2f), timeouts (5 s connect, 10 s total), the
response read limited to the status line and at most 8 KiB, nothing else parsed (review N6); a bounded queue per target (at
most 16 targets), backoff (1 s doubling to 5 min), the body `{v, hub, kind, serial?, at, text, alarm?}`, headers
`X-DSPi-Timestamp` and `X-DSPi-Signature: sha256=<HMAC(secret, timestamp + "." + body)>`; HMAC-SHA256 **hand-rolled over
`sha2`** (about 15 lines; `hmac` is not in `Cargo.lock`), tested against RFC 4231's vectors; secrets generated by the hub and
shown once (`outbound.set` with `secret: "new"`); `outbound.get` (Admin; secrets shown as set or not), `outbound.set`
(Admin), `outbound.test {target}` (Admin). **Done when:** tests against an in-test HTTP listener: delivery, signature
verifiable, RFC 4231 vectors, a refused or dead target backs off and never blocks the bus, a listener that sends an endless
response is cut at 8 KiB, the queue drops oldest with a count, an `https://` URL refused with a reason. **Deps:** P2SV.
**Clients:** web hub settings (P2WB2).

**P2AL: Device alarms.** [TODO] Size 2. Wave P2-C. **Owns:** new `crates/dspi-hub/src/device_alarms.rs`, the file
`<data>/alarms.json` (64 KiB cap), the diagnostics entries for `alarms.json` and `outbound.json` (redacted) in
`crates/dspi-core/src/diag.rs` (the one `diag.rs` owner of P2-C besides P2MQ's and P2SC's lines; see the Waves note).
**Implements:** M3 as in section 1 on `system::alarms::AlarmBook` (used, not edited), from the bus (`Health`, `Facts`,
registry, `Unresponsive`, `ScheduleRan`); defaults: offline 30 s, unresponsive 10 s, clip 10 events a minute, temperature
70 °C, CPU 90 % for 10 s; the "mute on clip storm" action off by default and, when on, one of the hub's own writes
(`INTERNAL_SESSION_ID`, the shared rule): the mute only, past the lock, never an unmute, recorded with the rule's name in
`by` and announced. Alarms and acknowledgements go to the history and the bus as `Alarm` (so P2WH and P2MQ deliver them).
`alarm.list` merges the alink book (`source`), `alarm.ack` acknowledges either. **Done when:** tests with the test clock:
each code raised and cleared; a latch-only device raises "clipped" once until cleared; acknowledged stays on until cleared;
the mute action mutes once per storm, passes another session's lock, never unmutes, and its history record names the rule;
rules set by Admin only; a webhook receives a raised alarm (with P2WH); the bundle carries both files with no secret.
**Deps:** P2HP, P2EV, P2WH (all P2-B). **Clients:** P2CC2, every client's banner.

**P2RS: REST mirror and API keys.** [TODO] Size 2. Wave P2-C. **Owns:** new `crates/dspi-hub/src/rest.rs`; in `server.rs`,
`answer`'s routing of `/api/v1/` and the request-body read (PUT and POST, under a marker); in `auth.rs` and `store.rs`, a
client kind `api` (an API key is a paired client minted without a PIN: same table, same `MAX_PAIRED_CLIENTS` cap of 64 shared
with the consoles, same `revoke`; `auth.list` shows the kind, OD-P2q); `ServerLimits` for REST requests. **Implements:** the
REST table of section 1 over the same handlers, on the Link port (OD-P2p); `auth.mint {name, role}` (Admin over Link;
never above the minting session's role; the key generated by the hub and shown once) and `uconsole hub api-key` (P2TM);
`Authorization: Bearer <key>`; **any request carrying an `Origin` header is `403`** (the web console uses the wasm binding,
so no browser needs REST, and there is no CORS question, review S2); the body read under `ServerLimits::request_timeout`,
`Content-Length` only, at most 64 KiB; JSON errors with the same codes; `--rest` off by default, and `/api/v1/` answers
`404` while off. Each request's principal follows the session-id rule. **Done when:** conformance tests: each endpoint, a
missing or revoked key `401`, a Viewer key's PUT `403`, a request with any `Origin` `403`, a body over the cap `413`, a
rate-limited key `429`; `hostile_lan.rs`-style tests: a trickled PUT body cut at the timeout, a wrong or missing
`Content-Length`, a body longer than its length; the 65th paired client or key refused; the WebSocket and the existing
pages unaffected, `/dspi/` unchanged. **Deps:** P2PA (recall included), P2EV (events endpoint). **Clients:** integrators.

**P2SC: Schedules.** [TODO] Size 2. Wave P2-C. **Owns:** new `crates/dspi-hub/src/schedule.rs`, the file
`<data>/schedule.json`, its diagnostics entry (under a marker in `diag.rs`), and `Cargo.toml`'s time-zone dependency (OD-P2h;
under a marker, the only `Cargo.toml` edit this wave). **Implements:** I5 as in section 1 (at most 256 rules of at most 16
actions; a malformed file read by the shared corrupt-file rule), with the clock rules of OD-P2h: local time from the hub
machine's time zone; no catch-up after downtime beyond a 5-minute grace; a forward jump of the wall clock skips (and records)
what it passed; a backward jump never runs a rule twice in a day; a non-existent local time (spring forward) runs at the next
valid minute, a repeated one (fall back) once; a wall clock before the build date does not run anything and raises
`clock_unset`. Actions run through the executor as the principal `Schedule: <name>` (Admin role, the session-id rule); an
offline target fails at once and is reported (`results`), never retried later; **an action on a device with a pending
guarded apply is refused `conflict` and reported** (review S4); `run_now` runs as the schedule's principal with the calling
Admin in `by`. **Done when:** tests with the test clock in two time zones across both DST changes, a jump forward and back,
the grace, an offline target, a target with a pending guard, a limit refusing an action, Admin-only edits, `run_now` with
`by`, the history record and `schedule.ran`, the caps, the bundle entry. **Deps:** P2PA, P2SV. **Clients:** web hub settings
(P2WB2), P2TM.

**P2MQ: MQTT on the hub.** [TODO] Size 1. Wave P2-C. **Owns:** new `crates/dspi-hub/src/mqtt/service.rs`, the file
`<data>/mqtt.json`, its redacted diagnostics entry (under a marker in `diag.rs`). **Implements:** the broker setting (host,
port, client id, username, password, base topic `dspi/<hub id>`), off by default; publishes the bus's `Alarm` and `Event`
messages to `<base>/<serial>/event` and the hub's availability as a will; `mqtt.get` (Admin; the password as set or not),
`mqtt.set`, `mqtt.status`. **Done when:** tests against the fake broker: publish on an injected `Alarm` bus message (the
end-to-end test with a real alarm runs in P2HA), the will on a lost hub, settings Admin-only, a wrong password reported in
`mqtt.status`, no password in the diagnostics bundle. **Deps:** P2MQC, P2SV; P2AL soft (its messages are P2SV's typed
`Alarm`). **Clients:** web hub settings (P2WB2).

**P2CC1: Typed core calls for the hub's new families.** [TODO] Size 2.5. Wave P2-C. **Owns:** new
`crates/dspi-core/src/hub_server.rs` (and `hub_server/` tests); in `crates/dspi-core/src/snapshot.rs`, the merged list
(under a marker); binding wiring (shared). **Implements:** as `hub_admin.rs` does for limits: `hub_caps()`, `hub_inventory()`,
`hub_identify(seconds, tone)` (and on USB the existing ext call, so a client has one Identify), `hub_health()`,
`hub_stats()`, `hub_events_read(serial, after_id, limit)` and the `event.added` stream, `hub_snapshots_*` (list, get, put,
rename, note, delete, recall); the client's snapshot list on a DSPi Link session to such a hub merges both sides, each entry
with `where: local | hub | both` and the hub's winning on a differing id (the local one listed as a local copy); "Save to
the hub" and "Copy here". Events reach the app through the existing hub-event hook and a listener (UniFFI callback, C
callback, `hub_server_events` on the web). Every call answers `unsupported` without the cap and sends nothing. **Done
when:** tests through `dspi-link-fakehub` or the real hub in-process: each call with and without the cap; the merge rules
including a differing id; the same text through C, UniFFI and wasm. **Deps:** P2IN, P2ID, P2HP, P2HS, P2EV, P2SS, P2PA (all
P2-A or P2-B). **Clients:** every client WP.

**P2HA: Home Assistant.** [TODO] Size 2. Wave P2-D. **Owns:** new `crates/dspi-hub/src/mqtt/ha.rs`. **Implements:** I6 as in
section 1: retained discovery under `homeassistant/<component>/<hub>_<serial>_<entity>/config` with a device block (name,
serial, firmware, model from the facts), state topics updated from the bus (volume and mute from attributed notifications,
the preset from `PRESET_LOADED`, sensors from health), `set` topics through the executor as the MQTT API key's principal
(OD-P2c, the session-id rule), removal of a forgotten device's entities. **Done when:** tests against the fake broker:
discovery for a device, a volume set moves the demo device and comes back as state, a set past a limit is refused and the
state stays, a locked device takes volume, mute and preset only, a Viewer-role key changes nothing, a forgotten device's
entities removed, and the end-to-end alarm: a real P2AL alarm published by P2MQ. **Deps:** P2MQ, P2PA, P2IN, P2AL.
**Clients:** web hub settings (P2WB2: the switch and the key).

**P2CC2: Core calls for alarms and schedules.** [TODO] Size 1. Wave P2-D. **Owns:** a second section of
`crates/dspi-core/src/hub_server.rs` (P2CC1's file, done in P2-C); binding wiring (shared). **Implements:** `hub_alarms()`,
`hub_alarm_ack(id)`, the `alarm.changed` stream, `hub_schedules()` (read), and a core notice row per alarm code
(`alarm.device_restarted` "Restarted at {time} ({reason})", and so on) in `assets/notices/notices.json` (P1NT's table), in
shared wording with no host-device nouns. **Done when:** tests as P2CC1's; every alarm code has a notice row (a test fails on
a code without one). **Deps:** P2CC1, P2AL, P2SC. **Clients:** every client's banner.

**P2TM: Terminal.** [TODO] Size 1.5. Wave P2-D. **Owns:** a new `crates/dspi-cmd` command `api`, `apps/terminal/src/main.rs`'s
table (additive, below TM15's macro block), `apps/terminal/src/hub.rs` (flags `--rest`, the `ServiceConfig` from flags; prompt
commands `inventory`, `alarms`, `schedule`, `api-key new|list|revoke`; `uconsole hub api-key` with the hub stopped),
`docs/integrator/` script examples (the I7 section of `hub-api.md`). **Implements:** I7 as in section 1. **Done when:**
command tests against an in-process hub with the demo device: `uconsole --hub HOST api param.get '{...}'` prints the answer,
exit codes for `denied`, `offline`, `limit_exceeded`; an API key minted at the prompt works for REST; examples run in the
container. **Deps:** P2PA, P2RS, P2SC, P2AL. **Clients:** Terminal.

### Firmware-dependent items (`bofr-main`, extension opcode 0x00)

Nothing in phase 2 needs new firmware. What it uses is already on `bofr-main` and decoded by the core (WP FWX); every use is
gated on `EXT_LIST` and degrades on stock Weeb Labs firmware:

| Item | Firmware (bofr-main) | Hub WP | On stock firmware |
|---|---|---|---|
| FW4 identify | `EXT_IDENTIFY` 0x0004 | P2ID, P2HP (board, hardware revision) | Identify hidden (`unsupported`); no tone fallback (OD-P2i) |
| FW5 reset reason, uptime, hang count | `EXT_RESET_INFO` 0x0005 | P2HP, P2EV, P2AL | Restarts seen only as a detach and attach, reason "unknown" |
| FW1 clip events | 0xE0 behind `EXT_NOTIFY_FEATURES` bit 1 (0x0003) | P2EV, P2AL | Clip alarm from the sticky latch only: "clipped since cleared", no rate |
| FW2 gain clamps, NaN refused | b964f01 | none (the API's own checks come first) | The registry and limits checks of the API still apply |

Optional, not planned: **FW6** (temperature in the status packet) would save the health poll one read every 30 s; not
worth a wire change now. **FW3** (fault input) belongs to S3 and phase 3's health board. Any new firmware item stays a
sub-command of the extension channel on `bofr-main`, never a change to an upstream opcode.

### Client WPs (one per client; the web two)

Each client WP builds on P2CC1 and P2CC2 only; no client talks JSON to the hub itself (the web's Worker goes through the
wasm binding, as for `hub_admin`). User text from the core's notice table and CG10's placeholders; no host-device nouns;
"Snapshot" and "Scene" as defined; a hub feature is shown only when `hub_caps()` has it.

**P2WB1: Web: the hub's devices, health, alarms and history.** [TODO] Size 3. Wave P2-E. **Owns:** new
`apps/web/src/fleet/hub.ts` (a `HubBackend` for a console served by a hub: the serving hub as one installation whose
devices come from the inventory, offline ones included; `hubDevices` from it; `access` from the session's role;
`pinnedMode` from `mode.pinned`), the backend choice in `apps/web/src/fleet/env.ts`, new `apps/web/src/screens/hub/`
(a device table: name, location, online, firmware, CPU, temperature, last restart, Identify, open in the console; Admin edits
location, note and tags and sees `hub.stats`; an alarm board with acknowledge; the alarm banner), the "On the hub" source in
`screens/events/`. **Done when:** unit and e2e tests (Chromium, Firefox, 375 px with no sideways scroll) against the
in-process hub with the demo device: the table lists an offline device, Identify answers, an alarm raised by the test is
shown and acknowledged, the events view reads the hub's records with `by`. **Deps:** P2CC1, P2CC2. **Clients:** web.

**P2WB2: Web: hub settings and shared snapshots.** [TODO] Size 2.5. Wave P2-E. **Owns:** new
`apps/web/src/screens/hub-settings/` as **its own route** (review S7: the link from P2WB1's hub screen is one additive line
added at merge), the "On the hub" section of `apps/web/src/screens/snapshots/`. **Implements:** Admin: notification targets
with Test, MQTT broker and the Home Assistant switch, schedules editor, API keys mint and revoke with the key shown once,
alarm rules; for a Control or Viewer session the settings page is read-only with the `installer.hub_role` notice's sibling
row saying Admin is given on the hub's machine. **Done when:** e2e as P2WB1's: a webhook target saved and tested against the
test listener, a schedule saved and `run_now`, a key minted and revoked, a snapshot saved to the hub, listed as `hub`,
recalled; Control sees read-only settings. **Deps:** P2CC1, P2CC2. **Clients:** web.

**P2QT: Qt: shared snapshots, the hub's history, alarm banner, Identify.** [TODO] Size 2. Wave P2-E. **Owns:** the hub section
of `SnapshotsController` and `qml/snapshots/*`; a new `EventsController` and `qml/events/EventsWindow.qml` (local and "On the
hub", through the registrar); alarm notices in `NoticeCenter`; Identify in the device menu (this wave's one menu WP: the
`MenuTable.qml`/`Menus.qml` entries). **Done when:** smoke sections on the demo device and through the in-process hub, both
schemes. **Deps:** P2CC1, P2CC2. **Clients:** Qt.

**P2AN: Android: the same.** [TODO] Size 2. Wave P2-E. **Owns:** the hub section of the Snapshots feature, a new events
screen in `feature-settings` (or the module that holds diagnostics), alarm notices in `AlertHost`, Identify in the device
sheet; `WriteGuardedCore` classification of the new methods. **Done when:** view-model and Robolectric tests; Gradle through
the slot; no visible layout growth. **Deps:** P2CC1, P2CC2. **Clients:** Android.

**P2SW: Swift: the same, and the Mac hub's services.** [TODO] Size 2.75. Wave P2-F (VM). **Owns:** the hub section of the
`Snapshots*.swift` views and `CoreSnapshots`, a new `Events*.swift` view, alarm notices in the notice host (P1SW3's),
Identify; `ServiceConfig` into `crates/dspi-hub/src/ffi.rs`'s `HubStartOptions` (SW-M3's file; every service off by
default, review S8). **Done when:** macOS and iOS test bundles on the VM; a hub started from the Mac app with default options
runs no optional service. **Deps:** P2CC1, P2CC2; P1SW3 (the notice host); SW-M3 merged. **Clients:** Swift.

### Waves

No two WPs in one wave own the same file, and no WP depends on another WP of its own wave (review M1). Service registration
lines in `service.rs` and the binding wiring are additive under markers and regenerated at merge. `crates/dspi-core/src/diag.rs`
takes one additive, marked line block per WP that adds a data file (P2IN in B; P2AL, P2SC, P2MQ in C, each its own block):
three blocks in one wave is the one place this plan relies on additive edits to one file outside the wiring.

| Wave | WPs (parallel) | Size | Needs |
|---|---|---|---|
| P2-A | P2SV, P2DOC, P2EVW, P2MQC, P2SS | 7 | `main` (`ab475933` or later). **SW-M3** (`wp/SWM3`) touches `crates/dspi-hub/{Cargo.toml, src/lib.rs, src/ffi.rs}` and not `server.rs`, `hub.rs`, `relay.rs`, `session.rs`, `auth.rs`, `store.rs` or `device.rs`: merge it before P2-A if it is ready; otherwise P2SV may start now (disjoint files) and rebases onto `main` before its merge. Any other branch touching `crates/dspi-hub/src/{server,hub,relay}.rs` merges first; the coordinator checks |
| P2-B | P2HP, P2IN, P2ID, P2HS, P2EV, P2PA, P2WH | 11.75 | P2-A |
| P2-C | P2AL, P2RS, P2SC, P2MQ, P2CC1 | 9.5 | P2-B |
| P2-D | P2HA, P2CC2, P2TM | 4.5 | P2-C |
| P2-E | P2WB1, P2WB2, P2QT, P2AN | 9.5 | P2-D |
| P2-F | P2SW (VM) | 2.75 | P2-E; P1SW3; SW-M3 |
| | **Total: 25 WPs** | **45** | hub 25.75, core 4.5 (P2EVW, P2CC1, P2CC2), docs 1, Terminal 1.5, web 5.5, Qt 2, Android 2, Swift 2.75 |

Soft dependencies inside a wave are on P2SV's typed bus messages, not on another WP: P2EV on P2HP's `Facts`/`Health`, P2MQ
on P2AL's `Alarm` (each tested with injected messages; the end-to-end alarm path is tested in P2HA).

PRO1 estimated phase 2 at about 22 hub and 2 core units; this plan is larger by the seam (P2SV), a hand-written MQTT client
(P2MQC, OD-P2c), hub-side recall and API keys, the REST hardening, and the client pages PRO1 counted per client. Each wave
ends with one Fable batch review; the full gate runs after P2-C (the hub complete but for Home Assistant) and after P2-F.
Wave P2-B is the widest: if the coordinator runs fewer agents, P2PA and P2HP go first (they gate the most).

---

## 3. Phase 3 outline: fleet in the web console

So that phase 2's shapes fit, here is what phase 3 adds (about 25 units, PRO1 F3 to F8 and U1), and what it reuses.

| Phase 3 | Reuses from phase 2 | Shape decided now |
|---|---|---|
| **F3 groups** (`group.list/set/delete`, Admin): named sets of serials with a note | inventory records (`tags` today; `groups` added), addressing by serial | schedule targets already accept `group:<id>` (refused `unsupported` until phase 3) |
| **F4 linked parameters** (`group.param.set {group, path, idx, value, offsets?}`) | the executor (`param.set` per member), limits per member | the multi-target `results: [{serial, ok, code?, msg?}]`; partial failure is reported, never retried |
| **F5 bulk apply** (a snapshot or document to a group, a P1HA guard per device, one report) | P2PA's recall with `guard`, the hub snapshot store | same `results`; guards per device as today, one at a time or in parallel by a setting |
| **F6 firmware rollout** (USB devices of the hub, one at a time, export first, verify after reboot) | the inventory (firmware column), the history, alarms | `fw.install` stays the reserved name; OD-P2b decides whether it ships and how it writes |
| **F7 health board** | `health.get`, `alarm.list`, `hub.stats`, the history | the health record of P2HP is the board's row; thresholds are P2AL's rules |
| **F8 fleet screens** (inventory, health, groups, bulk apply, history, rollout, open a device in the console) | `HubBackend` (P2WB1), the hub screens | `FleetBackend` gains optional `inventory`, `health`, `alarms`, `groups` methods, so a fleet server implements the same; every id is (hub id, serial) |
| **U1 System view** (all outputs in one table) | the existing session view | web first |
| Several hubs (phase 4, only if asked) | the API's serial addressing and `results` shape | `hub_id` in every event and REST answer from phase 2 on, so a fleet server can merge hubs without reshaping |

The web console opens a device by its existing session view; the fleet screens never become a second console (PRO1 F8's
risk). The hub serves the page (`--web`), so a desktop user gets the multi-device view in a browser tab on the same machine.

---

## 4. Decisions

**Recorded 2026-11-15 (coordinator, after the Fable review):** OD-P2a, c and e to q are **DECIDED as recommended** below
(firm). **OD-P2b** and **OD-P2d part (2)** are the **owner's and open**; phase 2 does not depend on either
(`fw.install` stays `unsupported`, and phase 2 uses the firmware items on `bofr-main` whatever happens upstream). Owner
questions, one line each:

1. **OD-P2b:** may the hub ever flash a device with nobody at the rack (phase 3, Admin only, one device at a time, export
   first, verify after reboot, alarm if it does not return)? yes / no / decide at phase 3.
2. **OD-P2d (2):** open an upstream pull request to Weeb Labs for FW2 alone (gain clamps and NaN refusal, no protocol
   change)? yes now / later / no.

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-P2a** | **The parameter API's shape and the public REST contract** (PRO1 question 5). **DECIDED** | (1) JSON over the existing `/dspi/v1` WebSocket only; (2) WebSocket plus a REST mirror over the same handlers; (3) REST only. Contract: (i) public and versioned now; (ii) experimental until phase 3, then frozen as v1; (iii) internal only | **(2) with (ii)**: the WebSocket family is the primary API (`hello.caps` `param_api`); REST is a thin, opt-in (`--rest`) translator under **`/api/v1/`**, outside the protocol's `/dspi/`; documented in `hub-api.md` as "v1, experimental" until phase 3's fleet needs are in, then frozen (additive changes only within v1) | REST is what Home Assistant, curl and a venue controller speak, and the WebSocket is what the consoles already hold. One handler set keeps the Link spec's reason for "no REST" (serialisation, locking and attribution in one path) true. `/api/` stays out of the protocol's `/dspi/` namespace, but spec 4.2 defines the whole HTTP surface of the port ("Nothing else is defined"), so it is still a deviation: recorded in `hub-api.md`'s deviations section (P2DOC). Freezing after phase 3 avoids a v2 for groups |
| **OD-P2b** | **Unattended firmware rollout** (PRO1 question 7; phase 3's F6). **OWNER, open**: whether a flash with nobody at the rack is ever acceptable is risk appetite; phase 2 is the same either way | (1) never remote: a flash is always a local act; (2) remote by the mass-storage drive copy (needs the hub machine to mount the RP2 drive: on a headless Linux hub that means udisks or root); (3) remote by a PICOBOOT writer in the core (the bootrom's vendor interface, no mount; 3 units; a udev rule for the boot-mode IDs) | If the owner says yes: **(3) in phase 3, Admin only**, one device at a time, behind the CG8 firmware guard and the K340 voltage guard, with the configuration exported first, verify-after-reboot (firmware version, `EXT_LIST`, and on a K340 the stored 1.20 V core setting read back) and an alarm if the device does not come back; refuse when more than one boot-mode device is present. Until then `fw.install` stays `unsupported` | A drive copy on a headless hub is the least portable step there is, and the owner's rule "anything not clear is dangerous" favours a writer we control and can verify. Nobody at the rack is acceptable only with the verify and alarm steps: there is no rollback slot (FW10), so the export is the way back |
| **OD-P2c** | **MQTT and Home Assistant in the hub** (PRO1 question 6). **DECIDED** | (1) in the hub, opt-in, with discovery; (2) leave it to third parties on top of REST; (3) a separate bridge program over REST | **(1), off by default, with a hand-written MQTT 3.1.1 client on std threads** (no async runtime, no new crate); `set` topics act as an API key's principal (Control by default) | Discovery gives zero-configuration entities, and MQTT pushes state, which REST polling does badly for a volume knob. The hub already has the threads, the roles and the limits; a bridge would duplicate them. A small client avoids pulling an async stack into a hub that has none |
| **OD-P2d** | **FW1, FW4, FW5 on bofr-main, and FW2 upstream** (PRO1 question 8). Part (1) **DECIDED**; part (2) **OWNER, open** (only the owner pushes upstream) | They are **already built** on `bofr-main` (0x0003 bit 1 clip event, 0x0004 identify, 0x0005 reset info; FW2 clamps in b964f01) and decoded by the core (FWX). Options: (1) keep them on `bofr-main`, gate every hub use on `EXT_LIST`; (2) also offer FW2 upstream as a standalone safety fix; (3) also propose FW1/FW4/FW5 upstream now | **(1) and (2)**: phase 2 uses them with graceful fallbacks; FW2 is worth an upstream pull request on its own (a bug fix with no protocol change), **opened by the owner** from the fork (we never push to Weeb Labs); FW1/FW4/FW5 wait for Weeb Labs' answer on reserving `0x00` | FW2 makes every host's checks a second line rather than the only one, for every user of the firmware. The extension items need the opcode reservation first; nothing in phase 2 waits for it |
| **OD-P2e** | **Auth for REST, MQTT and webhooks on an unencrypted network** (new). **DECIDED** | REST: (1) the paired Link token as bearer; (2) API keys: paired clients of kind `api` minted by an Admin (on the hub's machine or over Link), with a role, shown once, revocable; (3) HMAC-signed requests with a timestamp. Webhooks: (a) none; (b) an HMAC signature header with a per-target secret. MQTT: broker credentials in the hub's data folder | **REST (2); webhooks (b); MQTT credentials stored 0600 and redacted from diagnostics.** Tightened (review S2): any `Origin` on `/api/v1/` refused; keys and webhook secrets generated by the hub and shown once; keys count in the 64-client cap. The docs say plainly, in one place: anything on DSPi Link, the REST port and MQTT without TLS can be read by someone on the same network, keys included; keep the hub on a network you trust (the existing hub.md warning) | Keys are tokens the hub already knows how to hash, cap and revoke, and separate keys mean an integration can be revoked without unpairing a console. Signed requests (3) do not survive curl and Home Assistant's REST integration, and on a plaintext network they protect little that a key does not. A webhook signature lets the receiver tell the hub from anyone else, which is the one thing it needs. No hub-identity pinning (owner) |
| **OD-P2f** | **TLS for outbound connections** (new). **DECIDED** | (1) none in phase 2: `http://` webhooks and plain MQTT only; (2) `rustls` with the system's roots for `https://` webhooks and MQTT over TLS | **(1) now; (2) when an owner or user asks** (a later WP of about 1.5) | A LAN broker and a LAN receiver are the case phase 2 serves; `rustls` and `ureq` are already in the workspace through `dspi-tui`, but they would be new to the hub crate and the Mac's hub slice, and TLS needs a certificate story the LAN case does not. The `https://` refusal says why, so nothing fails silently |
| **OD-P2g** | **Scripting** (new; PRO1 I7). **DECIDED** | (1) no engine in the hub: scripts call the API from outside (`uconsole api`, REST, MQTT); (2) an embedded sandboxed engine (Rhai: no file or network access, an operation limit); (3) Lua (a C dependency) | **(1)** | Every rule the hub enforces already applies to an outside script, and a sandbox is a security surface we would have to keep sound on a network service. Schedules cover the timed case; phase 3's groups cover the many-device case. Revisit (2) only if an installer needs logic the hub must run unattended |
| **OD-P2h** | **The schedule's clock** (new). **DECIDED** | (1) the hub machine's wall clock and time zone, through a time-zone crate (`jiff`, reading the system's zone database); (2) UTC only, users convert; (3) a fixed UTC offset in the schedule file | **(1)** with the rules in P2SC (no catch-up past 5 minutes, jumps never run a rule twice or run what was skipped, DST gaps and repeats, nothing runs while the clock reads before the build date: `clock_unset`) | People schedule in local time across DST; (2) and (3) are wrong twice a year. The jump rules matter on a single-board hub without a battery clock, which boots with an old time until the network sets it |
| **OD-P2i** | **Identify without firmware support** (new). **DECIDED** | (1) hidden on firmware without `EXT_IDENTIFY`; (2) the hub plays a channel-identifying tone through the generator | **(1)** | A generator tone through an amplifier in a venue is not quiet, and the project's own test-audio rules exist because one fell through to real speakers. The firmware's tone is -40 dBFS through the output chain and only when asked |
| **OD-P2j** | **Who may change the hub's shared snapshots** (new). **DECIDED** | (1) Control may put, rename and note; Admin deletes; (2) Admin only; (3) any role but Viewer for everything | **(1)** | A Control user is who saves a state at a venue; deleting a shared state others rely on is an admin act. Recall rights are unchanged: P1HL already decides who may write an image on a limited device |
| **OD-P2k** | **Data retention on the hub** (new). **DECIDED** | Event history per device and in all; snapshots per device and in all; inventory size; cleared alarms kept | **History 2 MiB per device (about 10 000 records) and 128 MiB in all, oldest rotated file first; snapshots 200 per device, 256 KiB each, 64 MiB in all; inventory 1024 devices; 64 cleared alarms (today's `KEEP_CLEARED`); 2 MiB per device is `events::Limits { max_file_bytes: 512 KiB, rotated_files: 3 }`; `inventory.forget` removes a device's history, and its snapshots only when asked.** Admin-adjustable caps in the service config | Enough for months of a venue's history on a small SD card, and bounded so a chatty device cannot fill the disk (the project filled one once). Nothing is deleted silently except by rotation, which the format already documents |
| **OD-P2l** | **`device.list` and the inventory** (new). **DECIDED** | (1) `device.list` gains offline devices (PRO1 F1); (2) a separate `inventory.*` family; `device.list` unchanged | **(2)** | `device.list` is spec 7.4 with handles and a 60 s grace that every client and the upstream page read; changing its meaning breaks them. The inventory is keyed by serial and has no handles for offline devices |
| **OD-P2m** | **The principal of hub-run writes** (new). **DECIDED** | Schedules and MQTT: (1) schedules as Admin ("Schedule: <name>", created by Admins only), MQTT as its key's role; (2) everything as Control; (3) as the hub's own unjudged session | **(1)**; never (3). A principal is a `Session` with an id from `next_session_id`, never `LOCAL_SESSION_ID`, `INTERNAL_SESSION_ID` or `GUARD_SESSION_ID` (section 2's session-id rule) | Every write stays judged by limits, and an image from an Admin is held after the fact exactly as P1HL does today. (2) would make a scheduled snapshot recall fail on every limited device. (3) is how a guard's revert works, and nothing else should |
| **OD-P2n** | **The "mute on clip storm" alarm action** (new; PRO1 S3's host half). **DECIDED** | (1) offered, off by default; (2) not offered in phase 2 | **(1)** | It is the cheapest protection a venue can switch on, muting is always allowed (P1HL), and the hub never unmutes by itself, so it cannot fight a person |
| **OD-P2o** | **The principal and lock behaviour of the hub's own writes** (review). **DECIDED** | (1) a closed list sent as `INTERNAL_SESSION_ID`; (2) each as the asking client's session; (3) unspecified | **(1)**: exactly three, the guarded apply's revert (exists), identify (P2ID) and mute on clip storm (P2AL). Identify blinks whatever the lock; its tone answers `locked` while another session holds the device lock. Mute on clip storm passes the lock (mute only, never an unmute). The history names the caller or rule in `by` | The extension opcode is Admin-only in the policy, so a Control user's Identify cannot go out as that user; a mute that a lock could block would not protect anything. Keeping the list closed keeps every other write judged |
| **OD-P2p** | **REST on the Link port or a port of its own** (review). **DECIDED** | (1) the Link port; (2) a second port | **(1)** | One firewall rule; the existing per-peer HTTP limits and the `Origin` machinery apply; a second port buys nothing on a plaintext network |
| **OD-P2q** | **API keys in the paired-client table** (review). **DECIDED** | (1) the same table, with a kind; (2) a table of their own | **(1)**: same table, same `MAX_PAIRED_CLIENTS` (64, shared with the consoles), same `revoke`; `auth.list` shows the kind | One hash, cap and revoke path already tested; an integration is revoked like a console |

---

## 5. What not to do in phase 2

- Change `device.list`, `/dspi/` paths or any Link 1.2 message's meaning; everything is additive and announced in `caps`.
- Send `REQ_CLEAR_CLIPS`, write the core voltage, or let any API reach a raw opcode.
- Queue or replay a write for later: an offline target fails now and says so (the lost-Link rule, applied to the hub).
- Add TLS or identity pinning to DSPi Link (owner), or an account system on the hub: the three roles and paired clients are
  the boundary.
- Ship admin pages in Qt, Swift or Android: the hub's settings are the web console's (and `uconsole hub`).
- Build a fleet database or a cloud relay: JSON files in the hub's data folder are enough (PRO1 section 6).
