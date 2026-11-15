# The hub API

*WP P2DOC, 2026-11-15: the contract of PRO phase 2 ([`docs/core-merge/PLAN-P2.md`](../core-merge/PLAN-P2.md)). Status:
**v1, experimental** (OD-P2a): additive changes only, frozen as v1 once phase 3's fleet needs are in. Each family's
section is **draft** until the WP named in its heading builds it and marks it **built**; a draft section is the shape
that WP implements, not a promise that the hub answers it today.*

The hub ([hub.md](hub.md)) keeps running with no console open. This page is everything another program can ask it beyond
the DSPi Link 1.2 messages: its inventory of devices, Identify, health, the shared history, alarms and where they are sent,
parameters, presets and snapshots by name, schedules, MQTT and Home Assistant, a REST mirror, and the hub's statistics.
The DSPi Link spec is DSPi-Link's `spec/dspi_link_protocol_spec.md` (in this repository
`apps/apple/Documentation/dspi_link_protocol_spec.md`); where this page and the code disagree, the code is right and the
page is the bug.

Reference: `crates/dspi-hub` (one module per family, registered as a service in `service.rs`), the consoles' typed calls in
`crates/dspi-core/src/hub_server.rs`.

## Contents

| § | Family | Messages | Cap | Built by | State |
|---|---|---|---|---|---|
| 1 | [Conventions](#1-conventions) | | | P2DOC | built (this page) |
| 2 | [On an unencrypted network](#2-on-an-unencrypted-network) | | | P2DOC | built (this page) |
| 3 | [Inventory](#3-inventory) | `inventory.*` | `inventory` | P2IN | built |
| 4 | [Identify](#4-identify) | `device.identify` | `identify` | P2ID | draft |
| 3 | [Inventory](#3-inventory) | `inventory.*` | `inventory` | P2IN | draft |
| 4 | [Identify](#4-identify) | `device.identify` | `identify` | P2ID | built (P2ID) |
| 5 | [Health](#5-health) | `health.get` | `health` | P2HP | draft |
| 6 | [Event history](#6-event-history) | `event.*` | `events` | P2EV (records: P2EVW) | built |
| 7 | [Alarms](#7-alarms) | `alarm.*` | `alarms` | P2AL | draft |
| 8 | [Notifications out](#8-notifications-out) | `outbound.*` | `outbound` | P2WH | draft |
| 9 | [Parameters](#9-parameters) | `param.*` | `param_api` | P2PA | built (P2PA) |
| 10 | [Presets](#10-presets) | `preset.*` | `param_api` | P2PA | built (P2PA) |
| 11 | [Snapshots on the hub](#11-snapshots-on-the-hub) | `snapshot.*` | `hub_snapshots` | P2SS (store), P2PA (recall) | built (P2SS store, registered by P2PA; P2PA recall) |
| 8 | [Notifications out](#8-notifications-out) | `outbound.*` | `outbound` | P2WH | built |
| 9 | [Parameters](#9-parameters) | `param.*` | `param_api` | P2PA | draft |
| 10 | [Presets](#10-presets) | `preset.*` | `param_api` | P2PA | draft |
| 11 | [Snapshots on the hub](#11-snapshots-on-the-hub) | `snapshot.*` | `hub_snapshots` | P2SS (store), P2PA (recall) | draft |
| 12 | [Schedules](#12-schedules) | `schedule.*` | `schedule` | P2SC | draft |
| 13 | [MQTT and Home Assistant](#13-mqtt-and-home-assistant) | `mqtt.*` | `mqtt` | P2MQ, P2HA | draft |
| 14 | [REST and API keys](#14-rest-and-api-keys) | `auth.mint`, `/api/v1/` | `api_keys`, `rest` | P2RS | draft |
| 15 | [Statistics and the hub's name](#15-statistics-and-the-hubs-name) | `hub.stats`, `hub.rename` | `hub_stats` | P2HS | built |
| 16 | [Scripting](#16-scripting) | `uconsole api` | | P2TM | draft |
| 17 | [Deviations from the DSPi Link spec](#17-deviations-from-the-dspi-link-spec) | | | P2DOC | built (this page) |
| 18 | [Every message](#18-every-message) | | | P2DOC | built (this page) |

---

## 1. Conventions

*State: built (P2DOC). Every family below follows this section; a family section states only where it differs.*

### 1.1 Transport and envelope

- The primary API is JSON over the hub's DSPi Link WebSocket, `/dspi/v1`, after `hello` and authentication (spec 5, 6,
  7.1): one object per text frame, a string `t`, an integer `id` on requests, replies `ok` or `err` echoing `id`, events
  without `id`. JSON keys are `snake_case`. Unknown fields are ignored by both sides; an unknown request type answers
  `unknown_type`; a client drops unknown event types.
- `id` is only ever the request id. A field naming one thing of a family uses the family's noun instead (`snapshot`,
  `alarm`, `rule`, `target`), as `system.alarm.ack {alarm}` does; inside a returned object its own `id` is fine.
- Every answer and every event of these families carries `"v": 1` (the P1HA and P1HL convention). Every event and
  every REST answer also carries `hub`, the hub's id from `hello.hub.id`, so a later fleet server can merge several hubs
  without reshaping anything (PLAN-P2 section 3).
- Objects that belong to an existing file format keep that format's spelling inside the message: a document
  ([document-format.md](document-format.md)), a limits profile, a snapshot index entry (`savedMs`, `inputChannels`, ...),
  an event record ([event-log.md](event-log.md)). Everything else is `snake_case`.
- Times in messages are ISO-8601 UTC with `Z` (`"2026-11-15T09:30:00Z"`), as spec 7.1 says. The one exception is an event
  record, which keeps its file format's `t` (ms since the Unix epoch). Schedule times are local wall-clock times and say
  so (section 12).
- Optional REST mirror: section 14. Every REST request is translated onto the same handlers, with the same roles, checks,
  answers and codes.

### 1.2 Versioning and caps

A hub that has a family lists its cap in `hello.caps`. A client checks the cap before sending a family's messages; a
hub without it answers `unknown_type` (an older hub) or `unsupported`. Caps of this page:

| Cap | Family | Announced |
|---|---|---|
| `inventory` | section 3 | always, on a hub that has it |
| `identify` | section 4 | always; a device without `EXT_IDENTIFY` answers `unsupported` |
| `health` | section 5 | always |
| `events` | section 6 | always |
| `alarms` | section 7 | always (the audio link's `system.alarms` keep working beside it) |
| `outbound` | section 8 | always; no target is set by default |
| `param_api` | sections 9 and 10 | always |
| `hub_snapshots` | section 11 (`snapshot` is spec 7.5's cache, a different thing) | always |
| `schedule` | section 12 | always; no rule by default |
| `mqtt` | section 13 | always; the broker is off by default |
| `api_keys` | section 14 (`auth.mint`, `kind` in `auth.list`) | always |
| `rest` | section 14 (`/api/v1/`) | only while the hub runs with `--rest` |
| `hub_stats` | section 15 (`hub.stats`, `hub.rename`) | always |

Within v1 a family only gains fields, messages and codes; nothing changes meaning. A breaking change would be a new
cap (`param_api_2`), never a silent one.

### 1.3 Addressing a device

- By **serial** (`"serial": "E46058388B1A2E2C"`), the stable name of a DSPi across replugs, hubs and runs. Every
  request that addresses one device takes `serial`.
- **`handle` is also accepted** where DSPi Link has one (an online device, spec 2 and 7.4). Given both, they must name
  the same device (`bad_request` otherwise). An offline device has no handle: address it by serial.
- A serial the hub has never seen (or has forgotten) is `not_found`; one it knows but that is not attached now is
  `offline`; an unknown handle is `no_device` (spec 7.1).
- Several devices at once (schedules today, phase 3's groups) answer with the **multi-target result**, one row per
  device, in the order asked:

```json
{"t": "ok", "id": 9, "v": 1, "results": [
  {"serial": "E46058388B1A2E2C", "ok": true},
  {"serial": "E6614103E7A1B2C3", "ok": false, "code": "offline", "msg": "not attached"}
]}
```

  A row may carry more fields of its own (`value`, `action`). A partial failure is reported, never retried later
  (section 1.7). `group:<id>` targets are reserved for phase 3 and answer `unsupported` until then.

### 1.4 Values

Plain JSON values; the parameter's registry kind decides (`crates/dspi-proto/src/registry.rs`, `param.list` shows it):

| Kind | Value in JSON | Example |
|---|---|---|
| `float` | number, in the registry's unit, inside its range | `-20.0` (dB) |
| `int` | whole number, inside its range | `120` |
| `bool` | `true` or `false` | `true` |
| `choice` | the choice's word from the registry, or its number; answers give both (`value` the number, `choice` the word) | `"lowshelf"` or `3` |
| `mask` | whole number, one bit per channel or output | `5` |
| `text` | string, at most the registry's length | `"Stage left"` |
| `trigger` | `null` or no `value` | |
| `bytes`, `status` | not written by the API (`bad_request`); `param.get` answers `bytes` as `value_b64` | |

`idx` is an array of whole numbers, as many as the path's `indices` (`[]` or absent for none): `[2]` for output 3,
`[0, 4]` for channel 0 band 4. Indices count from 0.

### 1.5 Roles

The hub's three roles ([hub.md](hub.md), Roles): **viewer** reads, **control** also changes settings, **admin** does
everything, including the hub's own settings. Each message's row names the least role it needs; a lower one answers
`denied`. A PIN pairing gives at most control; admin is given on the hub's machine. An API key has the role it was minted
with (section 14).

### 1.6 Errors

`{"t": "err", "id": 7, "v": 1, "code": "<code>", "msg": "<one line for a person>"}`, with more fields where a row says.
`msg` is for logs and people; programs branch on `code`.

| Code | Meaning | Origin |
|---|---|---|
| `unauthenticated` | no token or key, or a revoked one | spec 7.1 |
| `denied` | the role does not allow it (and the voltage rule, `EXT_CORE_VREG`) | spec 7.1 |
| `bad_request` | malformed, a missing or wrong-typed field, a value outside the registry's range, a path not written by the API | spec 7.1 |
| `unknown_type` | the hub does not know the message | spec 7.1 |
| `no_device` | an unknown handle | spec 7.1 |
| `not_found` | an unknown serial, snapshot, rule, target, alarm or key | new |
| `offline` | the device is known but not attached; nothing was sent or queued | new |
| `locked` | another session holds the device's lock (spec 7.7) | spec 7.1 |
| `busy` | the session's in-flight limit, or one guard per device | spec 7.1 |
| `rate_limited` | too many requests (`retry_in_ms` when known) | spec 7.1 |
| `conflict` | the hub's state disagrees: a differing snapshot with the same id, a cap of a store reached, a pending guarded apply, a newer revision | new |
| `unsupported` | the device or the hub lacks it (no `EXT_IDENTIFY`, a `group:` target, `fw.install`) | spec 7.1 |
| `confirmation_required` | a gated write (`preset.save`, `preset.delete`, `dev.save`, `dev.save.io`) without `"confirm": true` from an admin | the core's token rule |
| `limit_exceeded` | the write crosses the device's limits profile; `limit` names it. Nothing was sent | P1HL |
| `installer_lock` | the device is locked and the write is not volume, mute or preset | P1HL |
| `image_refused` | a whole image (`0xA1`, `0xA3`: a recall, an import) from control on a device with a profile or a lock | P1HL |
| `internal` | the hub failed; the log says why | spec 7.1 |

A device's own refusal of a command (a DSPi Link status such as `STALL`) answers `err` with `code` `device` and `status` the
DSPi Link status name (spec 8.1).

### 1.7 Who writes: principals and attribution

- Every write of this API is a **principal's**: the asking session (or a child session with its role, lock rights and
  client, for the parameter API), a REST key's session, an MQTT key's session, or `Schedule: <name>` (admin). Each has a
  session id from the hub's remote range, so roles, the limits, the lock and the voltage rule judge it exactly as a
  console's write on DSPi Link. Nothing in this API reaches a raw opcode or `EXT_CORE_VREG`.
- **The hub's own writes are a closed list of three**, sent as the hub's internal session: the guarded apply's revert
  (P1HA), **identify** (section 4) and **mute on clip storm** (section 7). Each is recorded in the history with the caller
  or the rule in `by`.
- A write is attributed: other sessions see it as a foreign write (`origin` in the notification, spec 8.3), their undo
  as "changed elsewhere", and the hub's history records `session` and `by` (the client's name, the key's name, the
  schedule's or rule's name).
- The hub never queues or replays a write: a target that is offline fails now (`offline`), and the answer says so.
- The hub never sends `REQ_CLEAR_CLIPS`, never writes the core voltage and never flashes firmware (`fw.install` stays
  `unsupported`).

### 1.8 Bounds

Every request, list and store is bounded; past a bound a request answers `bad_request` (a request too large) or
`conflict` (a store full), never a partial write. Pages default to 100 rows and take at most 500. A data file the hub
cannot read is kept aside as `<name>.bad-<time>`, and the hub runs with defaults and a line in its log.

| Store (in the hub's `--store` folder) | Cap | Section |
|---|---|---|
| `inventory.json` | 1024 devices (the oldest offline ones dropped past it) | 3 |
| `events/` | 2 MiB per device (512 KiB per file, 3 rotated), 128 MiB in all, oldest rotated file first | 6 |
| `alarms.json` | 64 KiB; 64 cleared alarms kept | 7 |
| `outbound.json` | 16 targets, 64 queued deliveries each | 8 |
| `snapshots/` | 256 KiB per document, 200 per device, 64 MiB in all | 11 |
| `schedule.json` | 256 rules of at most 16 actions | 12 |
| `mqtt.json` | one broker | 13 |
| `hub-store.json` | 64 paired clients and API keys together | 14 |

Each file is written atomically and is 0600 on Unix. A diagnostics bundle ([diagnostics.md](diagnostics.md)) carries
each of them with every secret replaced by `set` or `not set`.

---

## 2. On an unencrypted network

*State: built (P2DOC). The one place this page says it.*

DSPi Link has no encryption, and the hub adds none (owner decision): no TLS on the DSPi Link port, no identity pinning.
The REST mirror (section 14) runs on the same port, and MQTT (section 13) and webhooks (section 8) go out as plain
`mqtt://` and `http://`. **Anyone on the same network can read all of it: PINs while pairing, tokens, API keys, broker
passwords, webhook bodies, and every parameter.** A webhook's signature proves the hub sent it; it hides nothing.

- Keep the hub on a network you trust, never behind router port forwarding (spec 6.4). For remote access, use a VPN.
- Give each integration its own API key with the least role it needs, so it can be revoked alone (`auth.revoke`).
- API keys and webhook secrets are made by the hub and shown once; the hub keeps a hash of a key, and a webhook secret
  only in its 0600 file.
- The REST mirror is off unless the hub runs with `--rest`, and it refuses every request that carries an `Origin`
  header, so a web page in a browser cannot use it (section 14).
- `https://` and MQTT over TLS are refused with a reason, not downgraded (OD-P2f).

---

## 3. Inventory

*State: built (P2IN). Off unless the hub's `ServiceConfig::inventory` is set; announced as `inventory` when on.*

Every DSPi the hub has ever seen, online or not, persisted in `inventory.json`. `device.list` (spec 7.4: handles, a
60 s grace) is unchanged; the inventory is keyed by serial and outlives a replug and a restart of the hub.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `inventory.list` | viewer | `serial?` or `handle?` (one device), `offset?` (default 0), `limit?` (default 100, at most 500) | `devices`: records, online first, then by last seen (newest first), then by serial; `total`; `next_offset` (`null` on the last page). A page also stops before about 512 KiB |
| `inventory.set` | admin | `serial` or `handle`, `location?`, `note?`, `tags?` (each replaces its field; `null` clears; absent keeps) | `device`: the record |
| `inventory.forget` | admin | `serial` or `handle`, `with_data?` (default `false`) | `serial`, `forgot`: a list of what went, in order (`"record"`, then `"events"` if it had history files, then `"snapshots"` if asked and it had any). `conflict` while the device is online; `not_found` for a serial not in the inventory |

A record:

| Field | Meaning |
|---|---|
| `serial` | the key |
| `handle` | the DSPi Link handle while online, else `null` |
| `online` | `true` while attached |
| `name` | the device's name as `device.list` gave it when last seen |
| `platform`, `fw`, `fw_beta`, `outputs` | from `REQ_GET_PLATFORM` (spec 7.4); `fw_beta` absent when unknown |
| `board`, `hw_rev`, `chip`, `chip_rev` | from `EXT_IDENTIFY` where the firmware lists it, else absent (`hw_rev` also when the build set none) |
| `location`, `note` | set by an admin, at most 256 characters each, any Unicode but control characters (a note may hold newlines and tabs); `null` when unset |
| `tags` | at most 16 strings of 1 to 32 characters, trimmed, duplicates dropped |
| `first_seen`, `last_seen` | times; `last_seen` is now while online, else when it went offline |
| `last_reset` | `{reason, prev_uptime_s?, at}` from `EXT_RESET_INFO` (reasons as section 5; `at` the reading's time less the uptime), else absent |

A value past a bound answers `bad_request` and changes nothing. `inventory.forget` removes the record and the device's
history files (`events/`); its snapshots on the hub only with `with_data: true` (each with its `snapshot.changed`). If a
file cannot be removed, the answer is `internal` and the record stays, so the forget can be tried again. Event to every
session: `inventory.changed {v, hub, serial, device}` (`device` `null` when forgotten, or dropped past the 1024-device
cap, oldest offline first, with a line in the hub's log). A firmware change updates the record and puts a `firmware`
history record on the hub's internal bus (stored by the history, section 6).

The file is written atomically, 0600 on Unix, one record per line inside one JSON object, so a torn file still reads
record by record: the hub keeps the whole file aside as `inventory.json.bad-<unix seconds>`, keeps what it could read, and
writes the file again. The diagnostics bundle carries it as `hub/inventory.json`.

## 4. Identify

*State: built (P2ID): `crates/dspi-hub/src/identify.rs`, on when the hub's `ServiceConfig::identify` is set.*

| `t` | Role | Request | Answer |
|---|---|---|---|
| `device.identify` | control | `serial` or `handle`, `seconds` (0 stops; at most 120, longer is shortened), `tone_output?` (an output index) | `serial`, `status`: `{remaining_s, last_tone, chip, chip_rev, board, hw_rev}` read after the write |

- `status` is `null` if the read after the write failed (the write went out). `last_tone` is the output of the last tone
  since the device started, or `null`; `chip` is `"RP2040"`, `"RP2350"` or `"chip <n>"`.

- The LED blinks for `seconds`. With `tone_output` the device also plays its quiet identify tone once (1 kHz, -40 dBFS,
  400 ms, through that output's whole chain); only when asked. There is no tone on firmware without `EXT_IDENTIFY`.
- It is one of the hub's own writes (section 1.7): the extension opcode is admin-only in the policy, so the hub sends
  it, not the caller. The blink goes out whatever the lock; the tone answers `locked` while another session holds the
  device's lock.
- `unsupported` (nothing sent) when the device's `EXT_LIST` lacks `EXT_IDENTIFY` or stalls (stock firmware; read once
  per attach); `bad_request` for a tone output the device does not have; `rate_limited` (with `retry_in_ms`) within 2 s
  of the last request sent to that device, whoever asked. A refused request does not count against the 2 s.
- Addressing as section 1.3: an unknown serial is `not_found` (a serial known only to the inventory, past the 60 s grace,
  is also `not_found` here), one known but not attached `offline`. A device that refuses the write answers `device` with
  `status`.
- Recorded in the history with the caller in `by` (and `session`): a record with `source` `identify`, `value` the
  seconds and `channel` the tone's output, put on the hub's bus for the history to store.

## 5. Health

*State: built (P2HP).*

On attach the hub reads the device's `EXT_LIST`, then `EXT_IDENTIFY` (board, hardware revision, chip) and
`EXT_RESET_INFO` where listed; firmware without the extension channel is asked nothing more. Then it reads the status
every 2 s, the temperature every 30 s and the reset record (uptime, hang count) every 60 s. The status read is a poll on
the same poll group the consoles' meters use (`REQ_GET_STATUS` 9 at the length a console asks for, so one read whoever
asks), inside the poll budget; a DSPi Link poll runs at 1 Hz at least, so the hub polls at 1 Hz and reports every 2 s.
The temperature and the reset record are single reads on the device's queue, not polls: a 1 Hz poll for a 30 s reading
would be 30 times the reads. If the poll budget refuses the hub's poll, it reads the status itself every 2 s and asks
again later. The hub's reads pass a session's device lock (a read changes nothing the lock protects). It reads; it
never writes, and never clears the clip latch.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `health.get` | viewer | `serial?` or `handle?` (both: they must name the same device) | `devices`: one record per online device (or the one asked) |

A record: `serial`, `handle`, `at` (the newest read), `cpu_pct` (the busier of the two cores), `clip_mask` (the sticky
latch: "clipped since someone cleared", one bit per channel), `temp_c` (to 0.01 °C), `uptime_s`, `hang_count`,
`unresponsive` (`true` once the device has given no answer for 10 s while attached, until its next answer),
`unresponsive_since` (while `unresponsive`), `reset` (`{reason, prev_uptime_s?}` from the attach, or from the read that
found the uptime gone backwards). A field the firmware does not give, or not yet read, is absent. Reset reasons:
`power_on`, `brownout`, `run_pin`, `watchdog`, `software`, `debugger`, `glitch`, `unknown`.
`uptime_s` and `hang_count` come from the reset-info read, every 60 s: they are as old as that read (`at` is the
newest read of any field), so two records 2 s apart usually carry the same uptime, and the uptime moves in 60 s
steps.

Errors: `not_found` for a serial the hub does not know, `offline` for one it knows that is not attached, `no_device` for
an unknown handle, `bad_request` for a wrong type or a serial and handle of two devices.

On the hub's internal bus (for its other services, not on the wire): `Facts` once per attach and again when the uptime
went backwards between two reset reads (a reset not seen as a detach); `Health` every 2 s; `Unresponsive` once per
silence. An embedder turns the service on with `ServiceConfig::health` (`HealthConfig::default()` has the periods above).

## 6. Event history

*State: built (P2EV; the record kinds and the writer: P2EVW). Reference: `crates/dspi-hub/src/history.rs`.*

One shared log per device in `events/`, in the event log's own file format and lock rule ([event-log.md](event-log.md)),
so a diagnostics bundle and `event_log_export` read it unchanged. The writer is `EventStore`
(`crates/dspi-session/src/events/store.rs`, built in WP P2EVW). The hub writes what it sees: attach and detach,
notifications with the writing session and its name, clip events, temperature and counter steps, resets with their
reason, firmware changes, recalls, identify, schedule runs, alarms.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `event.list` | viewer | `serial`, `after_id?` (default 0), `limit?` (default 100, at most 500) | `serial`, `records` (oldest first; with `after_id` 0 the newest `limit`), `more` (`true` if more follow `records`' last `id`), `newest_id` (the highest id the log holds; 0 when empty) |
| `event.watch` | viewer | `on` (bool), `serial?` (all devices when absent) | `on`, `serial` |
| `event.export` | admin | `serial`, `after_id?` | `serial`, `text` (the file format, whole records, at most about 48 KiB), `next_after_id` (`null` at the end), `newest_id` |

Event while watching: `event.added {v, hub, serial, record}`. A watch ends with the session. A device may be named
by `handle` instead of `serial` (section 1.3); a serial with no log on the hub is `not_found`.

**Ids can start again at 1** for a device: after `inventory.forget` removed its files, or after the total cap removed
every file of it. A client paging with an `after_id` above `newest_id` knows the numbering started
again and reads from 0. `after_id` 0 is a first read (the newest page); to page from a device's oldest record, take its
id from a first read with a large `limit`, or export.

A record held back for coalescing (a slider sweep, as the format does) is written out by any read of that device, or
when its second has passed. Bus messages the history could not take in time, and records it could not write, become one
`events_dropped` record per loss episode (`count` all of it) in the log of each device online (each device seen, when
none is). The record is written when the episode ends (the history has caught up), or once a minute while it lasts.

A device's `Facts` (its attach) record a `reset` when its uptime shows it restarted since the log last saw it, and a
`firmware` record when its version differs from the log's; the inventory's firmware change (P2IN) and the facts'
make one record, whichever comes first. One restart is one `reset` record, whether the facts or the next health
reading sees it first; a health reading that repeats the last uptime (section 5: it refreshes every 60 s) is no
restart.

Records are the file format's, with what phase 2 adds: the field **`by`** (the name of the client, key, schedule or rule
that caused it, next to `session`) and the kinds **`unresponsive`**, **`alarm`**, **`schedule`**, **`access`** and
**`events_dropped`** (`count` the records the hub could not write). An older reader reads the new kinds as `unknown`.

## 7. Alarms

*State: draft (P2AL).*

Device alarms beside the audio link's, on the same alarm book. Rules in `alarms.json` (admin).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `alarm.list` | viewer | `serial?`, `state?` (`active`: raised and acknowledged; `cleared`; `all`, the default) | `alarms`: device and audio link alarms together |
| `alarm.ack` | control | `alarm` (its `id`) | `alarm` |
| `alarm.rules.get` | viewer | | `rules` |
| `alarm.rules.set` | admin | `rules` (whole or in part; a code left out keeps its rule) | `rules` |

`system.alarms` and `system.alarm.ack` ([audio-link.md](audio-link.md)) keep working for the audio link's alarms. Event to
every session: `alarm.changed {v, hub, alarm}` when one is raised, acknowledged or cleared, `alarm` the whole object
below (the same object goes to notification targets, section 8, and MQTT, section 13).

An alarm in this family:

| Field | Meaning |
|---|---|
| `id` | a string, `"device:<n>"` or `"alink:<n>"`, unique for the hub's run |
| `source` | `device` or `alink` |
| `code` | below, or an audio link code |
| `subject`, `serial` | what it is about; `serial` where it is one device |
| `severity` | `warning` or `critical` |
| `message` | one line for a person |
| `state` | `raised`, `acknowledged`, `cleared` |
| `raised_at`, `cleared_at`, `acknowledged_by` | times, and who acknowledged it |
| `count` | how often it was raised again while on |

| Code | Raised | Cleared | Default rule |
|---|---|---|---|
| `device_offline` | a device in the inventory gone for N s | back | on, 30 s |
| `unresponsive` | attached, commands timing out for N s | answering | on, 10 s |
| `device_restarted` | a watchdog, glitch or brown-out reset | acknowledged | on |
| `clip` | clip events per minute over a threshold; on firmware without clip events, once on the latch ("clipped since cleared") | a minute under the threshold; for the latch, cleared by a person | on, 10 a minute |
| `temperature` | over a threshold | 5 °C under it | on, 70 °C |
| `cpu` | over a threshold for N s | under it | on, 90 % for 10 s |
| `schedule_failed` | a schedule's action failed | acknowledged | on |
| `clock_unset` | the hub's clock reads before its build date; schedules wait | the clock is set | on |

Rules: `{"<code>": {"on": bool, ...thresholds}, "clip": {"on": true, "per_minute": 10, "mute_on_storm": false}, ...}`
with the fields the table names (`after_s`, `above_c`, `above_pct`, `for_s`, `per_minute`). **Mute on clip storm** is
off by default; when on, a `clip` alarm mutes the device once per storm, as one of the hub's own writes (section 1.7):
the mute passes the lock, the hub never unmutes, and the history records it with the rule's name in `by`.

## 8. Notifications out

*State: built (P2WH, `crates/dspi-hub/src/outbound/`; MQTT publishing is section 13).*

Opt-in targets in `outbound.json` (admin); none by default, and with none the hub runs no delivery thread. Deliveries
go from a queue of their own (64 per target, the oldest dropped and counted past that), never from a device's worker;
each target has its own sender, so a dead or slow target never holds the bus or another target up. A failed try (no
connection, a timeout, a status other than 2xx) is tried again after the target's backoff, 1 s doubling to 5 min and
reset by a success; after 10 tries the delivery is dropped and counted in `dropped`. Queued deliveries are not kept
across a restart; the targets are.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `outbound.get` | admin | | `targets` (secrets as `set` or `not set`), each with `queued`, `dropped`, `last` (`{at, ok, status?, msg?}`, or `null` before the first try) |
| `outbound.set` | admin | `targets` (the whole list, at most 16) | `targets`, and `secrets` `{<target id>: "<secret>"}` for the new ones: shown in this answer only |
| `outbound.test` | admin | `target` (its `id`) | `delivered` (bool), `status?`, `msg?` (one delivery of kind `test`, now; recorded as its `last`) |

A target: `id` (1 to 32 of `A-Z a-z 0-9 _ -`; left out, the hub gives `t1`, `t2`, ...), `name` (at most 64
characters; the id when left out), `kind` (`webhook`: a JSON POST; `push`: an ntfy-style `text/plain` POST of
`text`), `url` (`http://` only, at most 2048 bytes, no credentials in it; `https://` refused with the reason, section 2
and OD-P2f), `enabled` (default `true`; a disabled target's queue is emptied), `kinds` (any of `alarm.raised`,
`alarm.cleared`, `restart`, `schedule.ran`, `schedule.failed`; every kind when left out), and `secret`:

| `secret` in `outbound.set` | Means |
|---|---|
| `"new"` | the hub makes one (32 random bytes as 64 hex characters) and shows it in `secrets`, once |
| `"set"` or left out | keep this id's secret, if it has one (so `outbound.get`'s list can be sent back as it is) |
| `"not set"` or `null` | no secret: no signature |
| anything else | `bad_request`: secrets are made by the hub, never typed |

A refusal is `bad_request` with `msg` naming the field and `target` the entry's index; nothing is kept then. A target
whose `id` stays keeps its queue, counts and backoff.

Which bus events are sent: an alarm raised (`alarm.raised`; a `device_restarted` one is `restart` first) or cleared
(`alarm.cleared`; an acknowledgement is not sent), and a schedule's run (`schedule.ran`; one with a failed action is
`schedule.failed` first). A target gets each event once, as the first of its kinds it picked.

A webhook's body: `{"v": 1, "hub": "<id>", "kind": "alarm.raised", "serial": "...", "at": "...", "text": "...",
"alarm": {...}}` (`serial` and `alarm` where they apply; `alarm` is section 7's object with ISO times). A schedule's
event carries `"schedule": {"rule", "name", "by"?, "results", "results_total"}` (the first 64 results) instead of
`alarm`. `text` is at most 1024 characters. Headers `X-DSPi-Timestamp: <unix seconds>` (the time of that try) and, for
a target with a secret, `X-DSPi-Signature: sha256=<hex HMAC-SHA256(secret, timestamp + "." + body)>`, keyed with the
secret's text as shown (its 64 characters as bytes). A push target gets the same headers. A receiver checks the
signature and that the timestamp is recent. The hub reads only the status line of the reply (at most 8 KiB; no line
end within them is a failure); a 2xx is delivered. Timeouts: 5 s to connect, 10 s in all.

## 9. Parameters

*State: built (P2PA). `crates/dspi-hub/src/api/` (feature `api` of `dspi-hub`; the binary that serves the
API turns it on), tests `crates/dspi-hub/tests/wp_p2pa_api.rs`.*

Read and write any registry parameter by path ([parameter-api.md](parameter-api.md)). A write runs through the same core
code as a console's (`write_param`, every typed setter's guard), inside the hub, as a child session of the caller: same
role, same client and name (`by`), its own session id from the hub's remote range. The limits, the installer lock, the
device lock and the voltage rule then judge it as they judge any client, and other sessions see it as a foreign write.
A read is the device's own answer (`dspi_session::Session::read`, as that child session).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `param.list` | viewer | `serial` | `serial`, `params`: one row per registry path the device offers |
| `param.get` | viewer | `serial`, `path`, `idx?` | `serial`, `path`, `idx`, `value` (and `choice`, or `value_b64`) |
| `param.get_many` | viewer | `serial`, `items`: `[{path, idx?}]`, at most 64 | `serial`, `values`: `[{path, idx, ok, value?, choice?, code?, msg?}]`, in order |
| `param.set` | control | `serial`, `path`, `idx?`, `value`, `confirm?` | `serial`, `path`, `idx`, `value`: the value read back from the device |

A `param.list` row (the web binding's `describe` row): `path`, `label`, `plain`, `kind`, `unit`, `min`, `max`
(`choices: [{value, label}]` for a choice, `max` for text), `readable`, `writable`, `routed` (`false`: not written by
the API; the row's `use` names the message or call that writes it), `gated` (written only with `"confirm": true` by an
admin), `indices` (how many), `layout` (the index ranges on this device, e.g. `[{"name": "output", "count": 9}]`;
`count` is `null` where only the device knows), `hazard` (`none`, `audible`, `deferred`, `flash`, `reconfig`,
`irreversible`). A path the device's firmware lacks (a feature it did not answer, an RP2350-only path on an RP2040)
is not listed.

- `bad_request`: a path not in the registry, the wrong number of indices or one the device does not have, a value of the
  wrong kind or outside the range (with `min` and `max`), a path the API does not write (`use` names what does, where
  the core has a call for it), or raw bytes. Not written by the API at all: `bytes` and `status` kinds, every
  extension-opcode path, and `meters.clear` (the clip latch is shared by every console: the hub never clears it).
- Gated paths (`preset.save`, `preset.delete`, `dev.save`, `dev.save.io`) are written only by an admin with
  `"confirm": true`: the API's explicit confirmation (the hub's core mints and spends the confirmation token itself).
  Otherwise `confirmation_required` (admin) or `denied`.
- On a locked device, control may set `vol.master`, `vol.user`, `vol.mute`, `out.mute` and load a preset only
  (`installer_lock` otherwise). Past a limit: `limit_exceeded` with `limit`, nothing sent.
- **The child session acts for its caller** in two things only: a device lock (spec 7.7) the caller holds lets its
  API writes through (another session's lock answers `locked`), and a PIN unlock (`limits.unlock`) the caller made
  is the child's too. Both end when the caller's session does; a child never keeps them. The role, the limits and
  the voltage rule judge the child as any session.
- A guarded apply pending on the device (another session's) refuses `param.set`, `preset.load`, `preset.save` and
  `snapshot.recall` with `conflict`; reads go on.
- Rate: one request at a time per connection (each is answered before the next is read), each on the caller's child
  session, which the in-flight limit (`hello.limits.max_inflight`) bounds as any session's.
- The hub keeps one core per (caller, device), opened on first use (a full attach: the probe and an image read) and
  closed after 60 s unused, when the caller's session ends or when the device goes; at most 32 at once, then `busy`.

## 10. Presets

*State: built (P2PA).*

| `t` | Role | Request | Answer |
|---|---|---|---|
| `preset.list` | viewer | `serial` | `serial`, `slots`: `[{slot, name, used}]`, `active` (the loaded slot or `null`), `startup` (`{mode: "slot", slot}` or `{mode: "last_active"}`) |
| `preset.load` | control | `serial`, `slot` or `name` | `serial`, `slot`, `name` |
| `preset.save` | admin | `serial`, `slot`, `name?`, `confirm: true` | `serial`, `slot`, `name` |

Slots are 0 to 9. A name matches exactly (case kept) among the used slots; two slots with the name are `conflict`, none
is `not_found`. Names are read from the device for each used slot. A preset load is held after the fact on a device
with a limits profile, as P1HL does (`limits.held` follows). `preset.save` without `"confirm": true` is
`confirmation_required`; `name` (at most 31 bytes) names the slot after the save. The audio link's scenes stay
`system.scene.apply {scene}` ([audio-link.md](audio-link.md)).

## 11. Snapshots on the hub

*State: draft (P2SS: the store and its messages; P2PA: `snapshot.recall`).*

The hub's shared copy of each device's snapshots, in `snapshots/`, in the client store's own format (one folder per
serial, `index.json`, one `<id>.dspipreset` per snapshot; `crates/dspi-session/src/snapshot_store.rs`). **The hub's copy
wins** (OD-P1b): a client that holds the same id with different content shows the hub's and keeps its own as a local
copy; nothing is overwritten silently.

### 11.1 The store (P2SS)

| `t` | Role | Request | Answer |
|---|---|---|---|
`snapshot` is a snapshot's store id (as `alarm` is an alarm's in `alarm.ack`); `id` is the request id, as on every
message. An entry is the index's own object: `{id, name, note, serial, savedMs, platform, inputChannels,
outputChannels, wireFormat}`.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `snapshot.list` | viewer | `serial` | `serial`, `snapshots`: entries, oldest first (the order of the client store's `index.json`) |
| `snapshot.get` | viewer | `snapshot` | `snapshot`: the entry, `document` (the document's text) |
| `snapshot.put` | control | `serial`, `document` (its text, or the JSON object) | `snapshot`: the entry, `created` (`false` when the hub already held this snapshot with the same content) |
| `snapshot.rename` | control | `snapshot`, `name` | `snapshot`: the entry |
| `snapshot.note` | control | `snapshot`, `note` | `snapshot`: the entry |
| `snapshot.delete` | admin | `snapshot` | `serial`, `snapshot`: the id |

- `document` is filed as a client's store files it: `meta.kind = "snapshot"` and a `snapshot` block naming the same
  serial as the message and the client's id. A document that is not such a snapshot, or names another device's serial,
  is `bad_request`. At most 256 KiB.
- The id is the client's, taken from the document and kept (capture-time ids are unique, so one snapshot keeps one id
  on both sides); the hub never makes one. The name and note are the document's; change them with `snapshot.rename`
  and `snapshot.note`. An id the hub holds with different content, or for another device, is `conflict`: the hub's
  copy stays, never overwritten.
- Names (at most 256 characters) and notes (at most 4096) take any Unicode. A cap reached (200 per device, 64 MiB in
  all) is `conflict`.
- Event to every session: `snapshot.changed {v, hub, serial, snapshot, change}` (`change`: `put`, `rename`, `note`,
  `delete`; `snapshot` the id).

### 11.2 Recall on the hub (P2PA)

*State: built (P2PA). The store's messages (11.1) are registered as the `snapshot` family on a hub with a data folder;
each change is broadcast as `snapshot.changed` right after the answer is made.*

| `t` | Role | Request | Answer |
|---|---|---|---|
| `snapshot.recall` | control | `serial`, `snapshot` or `name`, `guard?` (bool), `timeout_ms?` | `serial`, `snapshot`, `name`, `image` (whether the image was written), `lines` (what changed), `notes` (what was not written, and why), `guard` (its id, with `guard: true`) |

The hub writes the image and the settings outside it, in a recall's order, as the caller's child session. On a device
with a limits profile or a lock, control's recall is `image_refused` (nothing written); an admin's is written and then
held. With `guard: true` a guarded apply ([hub.md](hub.md), Guarded apply) begins first, as the caller (`timeout_ms`
as `apply.begin`'s, 30 s by default); confirm it with `apply.confirm`, or it reverts. A recall that fails after its
guard began is reverted at once. A device that is not attached gets nothing (`offline`). `name` matches exactly among
the device's snapshots (two are `conflict`); a snapshot of another device is `not_found`. Recorded in the history as a
`recall` with its `lines`, the caller's name in `by`.

A document reaches the hub in one DSPi Link text frame (at most 64 KiB, `hello.limits.max_frame`): send it compact,
or as the JSON object. The demo device's snapshot runs to about 80 KB pretty-printed, about 50 KB compact.

## 12. Schedules

*State: draft (P2SC).*

Rules in `schedule.json` (admin). A rule runs as the principal `Schedule: <name>` with the admin role, so the limits judge
its writes (an image held after the fact, as for any admin).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `schedule.list` | viewer | | `rules`, each with `next_run` and `last` (`{at, ok}`) |
| `schedule.set` | admin | `rule` (a new one without `id`) | `rule` |
| `schedule.delete` | admin | `rule` (its `id`) | `rule` |
| `schedule.run_now` | admin | `rule` (its `id`) | `rule`, `name`, `at`, `by`, `results` (the run's, as `schedule.ran`) |

A rule:

```json
{"id": "r1", "name": "Doors open", "enabled": true,
 "when": {"days": ["mon", "tue", "wed", "thu", "fri"], "time": "18:30"},
 "actions": [{"t": "preset.load", "name": "Live"},
             {"t": "param.set", "path": "vol.master", "value": -12.0}],
 "targets": ["E46058388B1A2E2C", "E6614103E7A1B2C3"]}
```

- `when` is `{days, time}` (local `HH:MM` on the given days) or `{once: "2026-12-24T17:00"}` (a local date and time).
- `actions` (at most 16), each a message of this page with its device field left out: `preset.load {slot or name}`,
  `snapshot.recall {snapshot or name}`, `system.scene.apply {scene}` (no target needed), `param.set` for `vol.master`,
  `vol.user`, `vol.mute` and `out.mute` only.
- `targets`: serials; `group:<id>` is reserved (phase 3, `unsupported` until then).

**The clock** (OD-P2h): local time in the hub machine's time zone. After downtime a rule missed by up to 5 minutes runs;
older ones do not. A forward jump of the clock skips what it passed (and records it); a backward jump never runs a rule
twice in a day. A local time that does not exist (spring forward) runs at the next valid minute; one that happens twice
(fall back) runs once. While the clock reads before the build date nothing runs and `clock_unset` is raised.

Each run answers per action and target: `results: [{serial, action, ok, code?, msg?}]`, where `action` is a string
naming the action by its position in `actions` and its `t` (`"0:preset.load"`). An offline target fails at
once and is not tried later; a target with a pending guarded apply is `conflict`. Event to every session:
`schedule.ran {v, hub, rule, name, at, by?, results}` (`rule` the rule's id, since events carry no `id`; `by` the admin
for `run_now`). Each run is recorded in the history;
a failure raises `schedule_failed`.

## 13. MQTT and Home Assistant

*State: draft (P2MQ: the broker and publishing; P2HA: Home Assistant).*

### 13.1 The broker (P2MQ)

MQTT 3.1.1, off by default, settings in `mqtt.json`. Plain MQTT only (section 2).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `mqtt.get` | admin | | `settings` (the password as `set` or `not set`) |
| `mqtt.set` | admin | `settings` (in part; `password` a string sets it, `null` clears it) | `settings` |
| `mqtt.status` | admin | | `state` (`off`, `connecting`, `connected`, `failed`), `msg?` (e.g. a refused password), `since`, `queued`, `dropped` |

Settings: `enabled`, `broker` (address), `port` (1883), `client_id`, `username`, `password`, `base_topic` (default
`dspi/<hub id>`), `ha` (section 13.2).

Topics: `<base>/status` is `online` (retained) and the will `offline`; each alarm and history record the hub
publishes goes to `<base>/<serial>/event` as JSON (an alarm as section 7's object with `"kind": "alarm"`, a record as
`{"kind": "event", "record": {...}}`).

### 13.2 Home Assistant (P2HA)

`ha: {"enabled": bool, "key": <cid>}`: the API key (section 14) whose principal `set` topics act as. Turned on
without `key`, the hub mints one named `Home Assistant` with the control role (revoked or changed like any key). The
limits, the lock and attribution apply to every `set`.

Per device, retained discovery under `homeassistant/<component>/<hub>_<serial>_<entity>/config`, with a device block
(name, serial, firmware, model from the facts):

| Entity | Component | State | Set |
|---|---|---|---|
| `master_volume` | `number` (range from the registry) | `vol.master` | `param.set vol.master` |
| `mute` | `switch` | `vol.mute` | `param.set vol.mute` |
| `preset` | `select` (names from the device) | the loaded slot | `preset.load` |
| `identify` | `button` | | `device.identify` (10 s, no tone) |
| `temperature`, `cpu` | `sensor` | health | |
| `clip` | `binary_sensor` | the clip latch | |

State topics `<base>/<serial>/<entity>/state`, set topics `<base>/<serial>/<entity>/set`. A refused set changes
nothing and the state stays. A forgotten device's entities are removed (empty retained configs).

## 14. REST and API keys

*State: draft (P2RS).*

### 14.1 API keys

An API key is a paired client of kind `api`, minted without a PIN: the same table, the same cap of 64 shared with the
consoles, the same `auth.revoke`. `auth.list` rows gain `kind` (`console` or `api`).

| `t` | Role | Request | Answer |
|---|---|---|---|
| `auth.mint` | admin | `name`, `role` (never above the minting session's) | `cid`, `name`, `role`, `kind: "api"`, `key`: shown in this answer only |

On the hub's machine: `uconsole hub api-key new|list|revoke` (section 16). The key is generated by the hub; it keeps only
its hash.

### 14.2 The REST mirror

Off by default; `uconsole hub --rest` turns it on, on the DSPi Link port (OD-P2p). While off, `/api/v1/` answers `404`.

- `Authorization: Bearer <key>` on every request (an API key, or a console's token).
- **Any request with an `Origin` header is `403`.** The web console uses DSPi Link through its own binding; no browser
  page needs REST, and there is no CORS.
- Bodies: JSON, `Content-Length` only (no chunked), at most 64 KiB, read within the hub's request timeout.
- An answer's body is the WebSocket answer without `t` and `id`, plus `v` and `hub`. An error's body is
  `{"v": 1, "hub": "<id>", "code": "<code>", "msg": "..."}` with the codes of section 1.6.
- Rate-limited per key (`429` with `Retry-After`).

The endpoints (PLAN-P2 I1; more may be added within v1, each onto an existing message):

| Method and path | Message |
|---|---|
| `GET /api/v1/devices` | `inventory.list` |
| `GET /api/v1/devices/{serial}/params/{path}?idx=0,1` | `param.get` |
| `PUT /api/v1/devices/{serial}/params/{path}?idx=0,1` body `{"value": ..., "confirm"?: true}` | `param.set` |
| `POST /api/v1/devices/{serial}/presets/{n}/load` | `preset.load {slot: n}` |
| `POST /api/v1/devices/{serial}/snapshots/{id}/recall` body `{"guard"?: true}` | `snapshot.recall` |
| `GET /api/v1/devices/{serial}/events?after=<id>&limit=<n>` | `event.list` |
| `GET /api/v1/alarms` | `alarm.list` |
| `POST /api/v1/alarms/{id}/ack` | `alarm.ack` |
| `GET /api/v1/health` | `health.get` |

| Code | HTTP status |
|---|---|
| `ok` | 200 |
| `bad_request` | 400 (413 for a body over the cap) |
| `unauthenticated` | 401 |
| `denied`, `limit_exceeded`, `installer_lock`, `image_refused`, `confirmation_required`, an `Origin` header | 403 |
| `not_found`, `no_device`, an unknown path | 404 |
| a method a path does not take | 405 |
| `conflict`, `offline` | 409 |
| `locked` | 423 |
| `rate_limited` | 429 |
| `internal` | 500 |
| `unsupported` | 501 |
| `busy`, `device` | 503 |

## 15. Statistics and the hub's name

*State: built (P2HS).*

Spec 7.9 as written, now answered. Both are Admin only: Control and Viewer get `denied`. The cap is `hub_stats`;
a hub built with the service off (`ServiceConfig::stats`) answers both `unsupported`, as hubs before it did.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `hub.stats` | admin | | `sessions`, `uptime_s`, `devices`: `[{handle, serial, cmds, errors, avg_rtt_ms, notify_dropped}]` (`serial` added, section 17) |
| `hub.rename` | admin | `name` | `name`. The new name is saved, in the next `hello` and `/dspi/v1/info`, and announced again over mDNS |

```json
{"t": "hub.stats", "id": 13}
{"t": "ok", "id": 13, "v": 1, "sessions": 3, "uptime_s": 8812,
 "devices": [{"handle": 0, "serial": "E46058388B1A2E2C", "cmds": 18233, "errors": 2, "avg_rtt_ms": 1.4, "notify_dropped": 0}]}
{"t": "hub.rename", "id": 14, "name": "Studio Mac"}
{"t": "ok", "id": 14, "v": 1, "name": "Studio Mac"}
```

What the fields count:

- `sessions`: network sessions open now, principals (API keys, schedules) included; the embedding app's own is not.
- `uptime_s`: seconds since the hub started.
- One row per **attached** device, by handle; a device offline inside its grace has no row. The counts are since
  the device attached and start again at a re-attach.
  - `cmds`: commands that ran on the device. A command refused before it reached the device (role, lock,
    limits, queue timeout) is not counted.
  - `errors`: those of them whose transfer failed (stall, timeout, disconnect, bus error).
  - `avg_rtt_ms`: their mean time on the transport, in milliseconds, three decimals; 0 before the first. Time in
    the device's queue is not included.
  - `notify_dropped`: this device's notifications that a full session outbox dropped (the session is sent
    `resync` reason 0 for them, spec 8.6), over every session.

`hub.rename`: control characters are dropped and the ends trimmed, as the embedders do with a start name. Empty
after that, not a string, or longer than 64 characters is `bad_request`, and nothing changes. The name is in use at
once (every later `hello` and `/dspi/v1/info`; sessions already open are not told), stored in the hub's store
(`hub-store.json`), where an embedder started without a name picks it up next time, and the mDNS record is
announced again under it (paced like every advertisement change, so a browser can see it a couple of seconds
later). An embedder that passes a name at every start (`uconsole hub --name`) puts its own back at the next start.

## 16. Scripting

*State: draft (P2TM).*

No script engine in the hub (OD-P2g): a script calls the API from outside.

- `uconsole --hub HOST api <t> ['<json>']` sends any message of this page and prints the answer (`--json` for the
  whole object). Exit codes follow `code`.
- `uconsole hub --rest`, and at the running hub's prompt `inventory`, `alarms`, `schedule`, `api-key new|list|revoke`.
- Examples: shell, Python over REST, Home Assistant. To write (P2TM).

## 17. Deviations from the DSPi Link spec

*State: built (P2DOC). What this hub does that DSPi Link 1.2 does not define. Everything else is additive (new message
types under new caps, new fields), which the spec allows (spec 10).*

| Spec | It says | This hub | Why |
|---|---|---|---|
| 4.2 | "Nothing else is defined" on the port; "deliberately no REST API for device control" | `/api/v1/` on the DSPi Link port, with `--rest` only (section 14) | REST is what curl, scripts and Home Assistant speak. It is a thin translator onto the same handlers as the WebSocket messages, so the spec's reason holds: one path for serialisation, locking and attribution. It stays outside the protocol's `/dspi/` namespace |
| 4.2 | `/` and static assets for an optional browser UI (`web=1`) | with `--web`: the hub's page under `/hub/` and the web console under `/console/` | the multi-device view is the web console served by the hub |
| 7.9 | `hub.stats` device rows `{handle, cmds, errors, avg_rtt_ms, notify_dropped}` | each row also carries `serial` | an offline-safe key for a row, as every other family uses |
| 7.1 | error codes `unauthenticated` to `internal` | adds `not_found`, `offline`, `conflict`, `confirmation_required`, `limit_exceeded`, `installer_lock`, `image_refused`, `device` (section 1.6) | additive; a client treats an unknown code as a failure with `msg` |

Unchanged: `device.list` and its 60 s grace (the inventory is a separate family), every `/dspi/` path, every DSPi Link 1.2
message's meaning, `fw.install` (`unsupported`), and the binary frames.

## 18. Every message

*State: built (P2DOC). Every request type `t` and event of phase 2, with its least role and its section.*

| `t` | Kind | Role | Cap | § |
|---|---|---|---|---|
| `inventory.list` | request | viewer | `inventory` | 3 |
| `inventory.set` | request | admin | `inventory` | 3 |
| `inventory.forget` | request | admin | `inventory` | 3 |
| `inventory.changed` | event | | `inventory` | 3 |
| `device.identify` | request | control | `identify` | 4 |
| `health.get` | request | viewer | `health` | 5 |
| `event.list` | request | viewer | `events` | 6 |
| `event.watch` | request | viewer | `events` | 6 |
| `event.export` | request | admin | `events` | 6 |
| `event.added` | event | | `events` | 6 |
| `alarm.list` | request | viewer | `alarms` | 7 |
| `alarm.ack` | request | control | `alarms` | 7 |
| `alarm.rules.get` | request | viewer | `alarms` | 7 |
| `alarm.rules.set` | request | admin | `alarms` | 7 |
| `alarm.changed` | event | | `alarms` | 7 |
| `system.alarms`, `system.alarm.ack`, `system.alarm` | existing | viewer, control | | 7, [audio-link.md](audio-link.md) |
| `outbound.get` | request | admin | `outbound` | 8 |
| `outbound.set` | request | admin | `outbound` | 8 |
| `outbound.test` | request | admin | `outbound` | 8 |
| `param.list` | request | viewer | `param_api` | 9 |
| `param.get` | request | viewer | `param_api` | 9 |
| `param.get_many` | request | viewer | `param_api` | 9 |
| `param.set` | request | control | `param_api` | 9 |
| `preset.list` | request | viewer | `param_api` | 10 |
| `preset.load` | request | control | `param_api` | 10 |
| `preset.save` | request | admin | `param_api` | 10 |
| `system.scene.apply` | existing | control, op by op | | 10, [audio-link.md](audio-link.md) |
| `snapshot.list` | request | viewer | `hub_snapshots` | 11.1 |
| `snapshot.get` | request | viewer | `hub_snapshots` | 11.1 |
| `snapshot.put` | request | control | `hub_snapshots` | 11.1 |
| `snapshot.rename` | request | control | `hub_snapshots` | 11.1 |
| `snapshot.note` | request | control | `hub_snapshots` | 11.1 |
| `snapshot.delete` | request | admin | `hub_snapshots` | 11.1 |
| `snapshot.changed` | event | | `hub_snapshots` | 11.1 |
| `snapshot.recall` | request | control | `hub_snapshots` | 11.2 |
| `schedule.list` | request | viewer | `schedule` | 12 |
| `schedule.set` | request | admin | `schedule` | 12 |
| `schedule.delete` | request | admin | `schedule` | 12 |
| `schedule.run_now` | request | admin | `schedule` | 12 |
| `schedule.ran` | event | | `schedule` | 12 |
| `mqtt.get` | request | admin | `mqtt` | 13.1 |
| `mqtt.set` | request | admin | `mqtt` | 13.1 |
| `mqtt.status` | request | admin | `mqtt` | 13.1 |
| `auth.mint` | request | admin | `api_keys` | 14.1 |
| `auth.list` (gains `kind`) | existing | admin | `api_keys` | 14.1 |
| `hub.stats` | request | admin | `hub_stats` | 15 |
| `hub.rename` | request | admin | `hub_stats` | 15 |
| `fw.install` | existing | | | stays `unsupported` |
