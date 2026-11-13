# The audio link: the hub's system model

For installers and programs that set up a system of DSPi units fed by the audio
link: one sender (the master clock) sends a frame stream over optical or SFP
links, and every unit plays link frame F a configured number of frames after it
arrives. The hub keeps the installation, computes each unit's delay so a room
plays sample-aligned, applies scenes at one frame on every unit, and raises
alarms when a link drops.

Not DSPi Link: "Link" below is the hub's network protocol; "the audio link" is
the stream.

| Source | What |
|---|---|
| `crates/dspi-hub/src/system/` | the model, the alignment engine, scenes, alarms, the I/O |
| `crates/dspi-hub/tests/wp_sw2_system.rs` | the behaviour below, against fake units |
| `crates/dspi-core/src/audio_link.rs` | one unit's link from a client (SW1) |
| DSPi-bofr `Documentation/Features/extension_channel_spec.md` 2.8 | the firmware's link commands (normative) |
| DSPi-bofr `Documentation/Features/link_protocol_spec.md` 7.2 | hops and hop delays (normative) |
| SYS2 spec sections 1, 3 and 7 | the design |

**State:** host side complete against the fake device's link model. Receiver
firmware (LK5) is partial and not yet run on hardware; the phase lock and the
apply queue (LK6) are not in firmware yet, so on real units today
`EXT_LINK_PHASE` and `EXT_APPLY_AT_FRAME` are unlisted and the hub reports
those units as unable to take the alignment.

## What the stream cannot tell the hub

A unit reports its lock state, the frame counter, the sender's id and `hops`,
the number of re-emitters (fan-outs, repeaters) between it and the sender. It
does not report which re-emitters those are, and `hops` counts boxes, not
frames: a fan-out may re-emit 2 or 3 frames late. So the hub's model holds the
topology:

- **senders**, with their USB serial when the hub can reach them;
- **fan-outs** (senders slaved to a link input) and **repeaters** (units with a
  transmit pin), each naming its `upstream`;
- **units**, each naming its `upstream` (a sender, a fan-out or a repeater), its
  room, the 8 link channels it takes (`slots`), its latency after the link
  (configured `latencyUs`, or `measuredLatencyUs`, which wins), and its speakers'
  distances by output.

Each re-emitter's hop delay is read from its `EXT_LINK_STATUS` offset 50
(`hop_delay_frames`); when it cannot be read, the model's `hopDelayFrames` is
used, and failing that 1 frame, with an issue `hop_delay_assumed`.

Rooms, zones (rooms sharing a source), groups (named sets of units) and sources
(programmes on link channels) complete the installation; scenes target them.

## The alignment

In frames at the stream's rate, for unit u behind re-emitters with hop delays
d1..dh, latency L_u after the link and its room's lip-sync offset S_u:

- E_u = D − Σd + (L_max − L_u) + S_u is the unit's effective delay, so frame F
  is heard at t_F + D + L_max + S_u on every unit of a room;
- the hub writes `EXT_LINK_PHASE` with phase lock, `d_local = D − h` and
  `hops_offset = E_u − d_local` (for a unit behind one 3-frame fan-out and
  nothing else, −2, as 2.8.5 says). Past the firmware's −64..64 the remainder
  moves into `d_local`, since the firmware uses only the sum;
- D is the model's `dFrames`, or when that is null the smallest D that puts
  every unit at or above its `d_local_min` (`EXT_LINK_INFO`). A unit outside its
  `d_local_min..d_local_max` gets the issue `delay_out_of_range` and is not
  written;
- each speaker's `out.delay` is (r_max − r) / 343 m/s, r_max being the furthest
  speaker of its room, so every speaker is heard at once at the listening
  position.

Lip sync: `lipSyncUs` on the model, or on a room. It delays that room's audio;
it cannot advance it.

A write that changes a unit's phase re-anchors it: 2 to 6 ms of silence. The
hub reads each value first and writes only what differs, so running the
alignment again changes nothing. The hub's own re-anchors raise no alarm.

Issues the engine reports (`alignment.issues`): `offline`, `limits_unknown`,
`delay_out_of_range`, `d_too_small`, `hop_delay_assumed`, `hops_mismatch` (the
stream's `hops` disagrees with the model), `sender_mismatch` (a locked unit
hears another sender id than the model's).

## Scenes at a frame

A scene is a list of actions, each an op on a target: `all`, `room:<id>`,
`zone:<id>`, `group:<id>` or `unit:<id>`. Ops are the firmware's: `mute`,
`unmute`, `preset_load` (arg = slot 0-9), `master_volume` (value = dB × 100,
−12800..0), `out_delay` (arg = output, value = samples), `click` (arg =
output), `sync_pulse`. `rate` is not a scene op: only the sender schedules it.
A scene may give one unit at most 4 ops (its queue).

The hub reads the frame counters, adds `applyLeadFrames` (default 2400, 50 ms
at 48 kHz) and rounds up to a superframe boundary. Then:

- **through the sender's superframe** (`EXT_LINK_CTRL_MSG`) when the scene is
  one op for every unit, every unit is behind that sender, the hub reaches the
  sender, and the op is not `unmute`: one write, no control-path latency;
- **to each unit** (`EXT_APPLY_AT_FRAME`) otherwise.

`unmute` always goes to each unit: the BULKFIX guard sits on each unit's own
connection and re-sends the gains of outputs a failed bulk write left suspect
before it lets an unmute through (or refuses it). A broadcast through the
sender would pass no guard. The guard treats an apply-at-frame `unmute` as an
unmute of every suspect output, from any client.

Each unit reports the outcome as event `0xE2` (applied, late, cancelled,
refused); the hub's answer lists each write's own result.

## Alarms

| Code | Severity | Raised | Cleared |
|---|---|---|---|
| `link_lost` | critical | a unit not locked at a read, or an `0xE1` with a loss reason | locked again |
| `unit_offline` | critical | a unit of the model not attached | attached |
| `link_errors` | warning | 16 or more bank errors, or a new loss, since the last read | a read without |
| `reanchor` | warning | an `0xE1` `REANCHOR` the hub did not cause | acknowledged |
| `delay_below_min` | critical | the unit's `D_BELOW_MIN` flag | the flag gone |

Events are decoded only from units that announce them (`EXT_NOTIFY_FEATURES`
bit 2) and that a `system.status` has read; without them alarms follow the
reads. Alarms live for the hub's run; the last 64 cleared ones are kept.

## On Link

JSON messages on the hub's `/dspi/v1` WebSocket, next to the Link 1.2 messages.
They are the parameter API's first messages (PRO1 I1, [parameter-api.md](parameter-api.md)):
named like I1's, judged by the hub's roles, with no REST mirror yet.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `system.get` | any | | `model`, `revision` |
| `system.set` | admin | `model` | `revision`; `bad_request` with the reason |
| `system.status` | any | | `revision`, `observed` (by serial), `alignment`, `alarms` |
| `system.align` | admin | `apply` (false: compute only) | `alignment`, `applied`, `writes` |
| `system.scene.apply` | control | `scene` | `plan` (`frame`, `path`, `ops`), `writes` |
| `system.alarms` | any | | `alarms` |
| `system.alarm.ack` | control | `alarm` (id) | `alarm` |

Events to every session: `system.changed` (`revision`) after a `system.set`,
`system.alarm` (`alarm`) when one is raised, acknowledged or cleared.

Each write result is `{serial, element, what, result}` with `result` one of
`ok`, `unchanged`, `skipped` (not reachable or out of range), `refused` (the
device stalled it), `locked` (a client holds the device's lock), `denied`,
`timeout`, `no_device`.

`system.status`, `system.align` and `system.scene.apply` read and write the
devices: they take a few transfers per unit and block that connection, not the
hub. The hub's writes go through each device's queue as the hub's own session,
so a client's lock and the policy apply.

Example model (one sender, a fan-out, two units in one room):

```json
{"t": "system.set", "id": 1, "model": {
  "rooms": [{"id": "cinema", "name": "Cinema"}],
  "senders": [{"id": "tx", "serial": "E66138...", "senderId": 66}],
  "fanOuts": [{"id": "fan", "upstream": "tx", "output": 1}],
  "units": [
    {"id": "front", "serial": "E66...A", "room": "cinema", "upstream": "tx",
     "slots": [0, 1, 2, 255, 255, 255, 255, 255],
     "speakers": [{"output": 0, "distanceM": 3.2}, {"output": 1, "distanceM": 3.0}]},
    {"id": "rear", "serial": "E66...B", "room": "cinema", "upstream": "fan",
     "slots": [4, 5, 255, 255, 255, 255, 255, 255], "measuredLatencyUs": 120}
  ],
  "scenes": [{"id": "quiet", "actions": [{"target": "all", "op": "master_volume", "value": -2000}]}]
}}
```

Field names are camelCase; an unknown field is refused. Ids are unique across
the whole model and may not contain `:`.

## Where it is kept

In the hub's store, `hub-store.json`, under `system`, beside the paired clients
and device names. A hub without a store (the app's in-process hub) keeps the
model for its run only. A model in the file that this hub cannot read is
ignored with a log line and left in the file until a new one is set.

## Not done yet

- Measuring latencies (SYS2 3.5's click and scope or cross-correlation): enter
  `measuredLatencyUs` by hand for now.
- Several senders in one model work for alignment and per-unit scenes; the
  superframe path is taken only when every unit is behind the one sender.
- A periodic poll: alarms follow events and `system.status` reads. A client that
  shows the system calls `system.status` every few seconds.
- The clients' "Clock and links" panel and slot matrix (WP SW3).
