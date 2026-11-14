# The audio link: the hub's system model

For installers and programs that set up a system of DSPi units fed by the audio
link: one sender (the master clock) sends a frame stream over optical or SFP
links, and every unit plays audio link frame F a configured number of frames after it
arrives. The hub keeps the installation, computes each unit's delay so a room
plays sample-aligned, applies scenes at one frame on every unit, and raises
alarms when a link drops.

Not DSPi Link: "Link" below is the hub's network protocol; "the audio link" is
the stream.

| Source | What |
|---|---|
| `crates/dspi-hub/src/system/` | the model, the alignment engine, scenes, alarms, the I/O |
| `crates/dspi-hub/tests/wp_sw2_system.rs` | the behaviour below, against fake units |
| `crates/dspi-core/src/alink.rs` | one unit's link from a client (SW1) |
| DSPi-bofr `Documentation/Features/extension_channel_spec.md` 2.8 | the firmware's link commands (normative) |
| DSPi-bofr `Documentation/Features/alink_protocol_spec.md` 7.2 | hops and hop delays (normative) |
| SYS2 spec sections 1, 3 and 7 | the design |

**State:** host side complete against the fake device's link model. Receiver
firmware (LK5) is partial and not yet run on hardware; the phase lock and the
apply queue (LK6) are not in firmware yet, so on real units today
`EXT_ALINK_PHASE` and `EXT_APPLY_AT_FRAME` are unlisted and the hub reports
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
  room, the 8 audio link channels it takes (`slots`), its latency after the link
  (configured `latencyUs`, or `measuredLatencyUs`, which wins), and its speakers'
  distances by output.

Each re-emitter's hop delay is read from its `EXT_ALINK_STATUS` offset 50
(`hop_delay_frames`); when it cannot be read, the model's `hopDelayFrames` is
used, and failing that 1 frame, with an issue `hop_delay_assumed`.

Rooms, zones (rooms sharing a source), groups (named sets of units) and sources
(programmes on audio link channels) complete the installation; scenes target them.

## The alignment

In frames at the stream's rate, for unit u behind re-emitters with hop delays
d1..dh, latency L_u after the link and its room's lip-sync offset S_u:

- E_u = D − Σd + (L_max − L_u) + S_u is the unit's effective delay, so frame F
  is heard at t_F + D + L_max + S_u on every unit of a room;
- the hub writes `EXT_ALINK_PHASE` with phase lock, `d_local = D − h` and
  `hops_offset = E_u − d_local` (for a unit behind one 3-frame fan-out and
  nothing else, −2, as 2.8.5 says). Past the firmware's −64..64 the remainder
  moves into `d_local`, since the firmware uses only the sum;
- D is the model's `dFrames`, or when that is null the smallest D that puts
  every unit at or above its `d_local_min` (`EXT_ALINK_INFO`). A unit outside its
  `d_local_min..d_local_max` gets the issue `delay_out_of_range` and is not
  written;
- each speaker's `out.delay` is (r_max − r) / 343 m/s, r_max being the furthest
  speaker of its room, so every speaker is heard at once at the listening
  position. A unit holds at most 2046 samples (42.6 ms at 48 kHz, 21.3 ms at
  96 kHz) and stores the host's milliseconds while capping only the samples, so
  a read-back cannot show the cap. The hub cuts a longer delay to the maximum,
  reports the issue `delay_over_max`, and answers that write `clamped (ok)` or
  `clamped (unchanged)`, never plain `ok`.

Lip sync: `lipSyncUs` on the model, or on a room. It delays that room's audio;
it cannot advance it.

A write that changes a unit's phase re-anchors it: 2 to 6 ms of silence. The
hub reads each value first and writes only what differs, so running the
alignment again changes nothing. The hub's own re-anchors raise no alarm.

The limits come from each unit's `EXT_ALINK_INFO` and follow its output latency
option (LK6 at 48 kHz: 81..431 frames under the 1 ms option, 257..399 under the
default 4 ms). D = 128 is used only when no unit reports limits, and then
nothing is written.

Issues the engine reports (`alignment.issues`): `offline`, `limits_unknown`, `delay_over_max`,
`delay_out_of_range`, `d_too_small`, `hop_delay_assumed`, `hops_mismatch` (the
stream's `hops` disagrees with the model), `sender_mismatch` (a locked unit
hears another sender id than the model's).

## Scenes at a frame

A scene is a list of actions, each an op on a target: `all`, `room:<id>`,
`zone:<id>`, `group:<id>` or `unit:<id>`. Ops are the firmware's: `mute`,
`unmute`, `preset_load` (arg = slot 0-9), `master_volume` (value = dB × 100,
−12800..0), `out_delay` (arg = output, value = samples), `click` (arg =
output), `sync_pulse`. `rate` is not a scene op: only the sender schedules it.
`out_delay` takes 0..2047 samples and `out_delay` and `click` an output below 9,
as the sender checks them. A scene may give one unit at most 4 ops (its queue).
A scene may also set or clear the system mute (`"systemMute": true` or
`false`); without it the scene leaves the system mute as it is.

The hub reads the frame counters, timing each read, and adds a lead: the
model's `applyLeadFrames` (default 2400, 50 ms at 48 kHz), or more when the
writes will take longer. It estimates each op as two transfers (the write and a
read of the unit's queue) at that unit's measured round trip (at least 10 ms; a
unit whose round trip could not be measured, such as one behind a Link bridge
that did not answer, is counted at 100 ms), plus 50 ms. It converts the time at
96 kHz so the lead is never short at any rate, then rounds up to a superframe
boundary. Then:

- **through the sender's superframe** (`EXT_ALINK_CTRL_MSG`) when the scene is
  one op for every unit, every unit is behind that sender, the hub reaches the
  sender, and the op is not `unmute`: one write, no control-path latency;
- **to each unit** (`EXT_APPLY_AT_FRAME`) otherwise.

-- WP HWLK1F -- **Units without the audio link.** A unit whose firmware has no
audio link (its `EXT_LIST` names no 0x8000 block, or `EXT_ALINK_INFO` stalls: a
K340 on bofr-main 470a127, stock firmware) takes no frame. Its ops are written at
once with its own commands (`mute`/`unmute` the user mute, `preset_load`,
`master_volume`, `out_delay` as samples at the model's rate), listed in the
plan's `direct`, with the result `applied_now`; `click` and `sync_pulse` have no
such command (`unsupported`). The rest of the scene is planned as above; the
superframe path needs only every unit *with* the link in the target. Ops on
units with the link that no frame could be read for are listed in `unscheduled`
and answered `no_frame`, the path is then `direct`. A scene is refused
(`no_device`, "no unit of the scene reports an audio link frame") only when nothing can
be written at once and nothing can be scheduled. On the superframe path, a unit
of the scene that is not attached is answered `no_device` (reason
`not connected`), and one attached whose own frame could not be read `no_frame`
(reason `no audio link frame read`: not known to have taken it). -- fixup hwlk1f --
A unit goes the direct way only when two reads agree that its firmware has no
link (its answer now and the last status poll's, or a second read when no poll
has told); an unknown unit never does: its ops stay frame-timed or are not
written. `onTime` is true only when every op acted at the frame: never
with an op applied at once or a unit not reached. The clients say it in one
sentence: "Evening recalled: applied now on K (no audio link); at audio link frame N
through the sender on the others."

`unmute` always goes to each unit: the BULKFIX guard sits on each unit's own
connection and re-sends the gains of outputs a failed bulk write left suspect
before it lets an unmute through (or refuses it). A broadcast through the
sender would pass no guard. The guard treats an apply-at-frame `unmute` as an
unmute of every suspect output, from any client.

After each per-unit write the hub reads the unit's queue (`EXT_APPLY_AT_FRAME`
IN) and reports what the unit says: `queued` (waiting for its frame),
`applied`, `late` (its frame had passed, so it was applied at once and not
with the others), `cancelled`, `refused_at_apply`, or `queued_unverified` when
the queue could not be read. After the last write it reads a frame counter
again (`frameAfterWrites`), and `onTime` is true only when every op is known to
act at the plan's frame. The units also send `0xE2` per op. Writes are made in
order with no rollback, and nothing is re-sent to a unit that was offline; each
write's result is in the answer.

**The system mute.** The sender's superframe carries a system-mute bit; while
it is set every unit on the link plays silence. The sender takes the bit from
`system_flags` on every `EXT_ALINK_CTRL_MSG` write, whatever the op. So the
hub reads the bit before a superframe write and writes it back unchanged
(`plan.systemFlags`). A sender whose bit cannot be read is not used for the
superframe path. Only `system.mute` or a scene with `systemMute` changes it.
A scene that mutes does so before its unit ops; one that unmutes does so after.

**Roles.** A scene runs only if the caller's role may send each of its ops
directly, under the hub's policy: `mute` and `unmute` as `SET_OUTPUT_MUTE`,
`preset_load` as `PRESET_LOAD`, `master_volume` and `out_delay` as their
setters (all control), and `click` and `sync_pulse` as the link's extension
write (admin). The system mute counts as muting outputs (control). Otherwise
nothing is written and the answer is `denied`, naming the op.

## Alarms

| Code | Severity | Raised | Cleared |
|---|---|---|---|
| `alink_lost` | critical | a unit not locked at a read, or an `0xE1` with a loss reason. One loss is one alarm: its count is the unit's losses since it was raised. Another loss before it clears raises it again, acknowledged or not, with the new count | locked again |
| `unit_offline` | critical | a unit of the model not attached | attached |
| `alink_errors` | warning | 16 or more CRC-16 errors within any 60 s of the hub's time. The growth between two reads is spread evenly over the time between them, so the rate decides, not how often clients read: 16 errors over four hours never alarm. A loss is `alink_lost`'s | 60 s with no new error |
| `reanchor` | warning | an `0xE1` `REANCHOR` the hub did not cause | acknowledged |
| `delay_below_min` | critical | the unit's `D_BELOW_MIN` flag | the flag gone |
| `system_muted` | warning | a sender's system mute seen set, at a read or a write | seen clear |
| `usb_realigned` | warning | the hub wrote the USB-direct units' speaker delays again by itself ("Keep USB-direct units aligned", below): the record of it, naming the units | acknowledged, or the next re-align (kept with the cleared ones) |

Events are decoded only from devices that announce them (`EXT_NOTIFY_FEATURES`
bit 2). The hub asks each device of the model when it attaches or the model
changes, and again at every `system.status`. Events are matched to the model
by the device's serial, so a unit that comes back on another handle is
followed at once, and a device that took a freed handle is never mistaken for
it. A fan-out's `0xE1` alarms under the fan-out's own id. Units behind it play
silence while it is lost; after it locks again, run `system.align`, since its
hop delay may have changed. Alarms live for the hub's run; the last 64 cleared ones are kept.

## On Link

JSON messages on the hub's `/dspi/v1` WebSocket, next to the Link 1.2 messages.
They are the parameter API's first messages (PRO1 I1, [parameter-api.md](parameter-api.md)):
named like I1's, judged by the hub's roles, with no REST mirror yet.

| `t` | Role | Request | Answer |
|---|---|---|---|
| `system.get` | any | | `model`, `revision`, `role` (this session's, now) |
| `system.set` | admin | `model`, optional `revision` (the one it was edited from) | `revision`, `warnings` (WP HWLK1F: units without the audio link; the model is kept); `bad_request` with the reason; `conflict` when `revision` is given and the hub has a newer one |
| `system.status` | any | | `revision`, `observed` (by serial), `alignment`, `alarms`, `role` |
| `system.align` | admin | `apply` (false: compute only) | `alignment`, `applied`, `writes` |
| `system.scene.apply` | control, op by op | `scene` | `plan` (`frame`, `path`, `ops`, `leadFrames`, `systemMute`, `systemFlags`), `writes`, `frameAfterWrites`, `onTime` |
| `system.mute` | control | `muted` (bool) | `writes`, one per reachable sender |
| `system.alarms` | any | | `alarms` |
| `system.alarm.ack` | control | `alarm` (id) | `alarm` |

Events to every session: `system.changed` (`revision`) after a `system.set`,
`system.alarm` (`alarm`) when one is raised, acknowledged or cleared,
`system.mute` (`sender`, `muted`) when the hub sets or clears the system mute.

`system.status` is answered at most once a second per session. A second
request within that time gets `rate_limited`, because each read costs every
unit's worker five or more transfers.

-- WP SW3 -- `observed` also carries each unit's `stats` (the rest of
`EXT_ALINK_STATUS`: `lineClass`, `channels`, `lockCount`, `crc8Errors`,
`bitSlips`, `superframeErrors`, `vcxoPpm`, `fillFrames`) and a receiver's
`trimScan` from `EXT_ALINK_DIAG` (`state`, `trimLo`, `trimHi`, `trimUsed`,
`cleanMask`), read only when the DIAG layout is version 1. A sender's carries
`pendingApplyFrame`, the frame of a superframe apply it still holds: it takes
no other until that frame has passed, and refuses one meanwhile (the write's
result is `refused`).

-- fixup sw3x -- `role` in `system.get` and `system.status` is the session's role
as the hub judges it now: a client sees a role changed live (`uconsole hub role`)
at its next read, without reconnecting.

Each write result is `{serial, element, what, result}` with `result` one of
`ok`, `unchanged`, `skipped` (not reachable or out of range), `refused` (the
device stalled it), `locked` (a client holds the device's lock), `denied`,
`timeout`, `no_device`; for a scene also `applied_now`, `unsupported` and
`no_frame` (above). -- WP HWLK1F -- A `skipped` alignment write carries `reason`:
`not connected`, `no audio link (firmware)`, `limits unknown` or `out of range`;
a scene's op applied at once carries `no audio link (firmware)`. The clients say
what an alignment did: "Nothing was written: skipped on K (no audio link
(firmware))." when nothing was (also when every write failed: "Nothing was
written: 2 writes failed; …"), else the units written and those skipped.

-- WP HWLK1F -- `observed` carries `alink`: `present` (`EXT_ALINK_INFO`
answers), `none` (the firmware has no audio link, as above) or null (not told).
-- fixup hwlk1f -- `none` comes only from the firmware's own answer: the device
stalls `EXT_ALINK_INFO` (a stall, not a bus fault, a short or malformed reply or
a timeout, which share the Link status `ERROR`), or `EXT_LIST` answers without
it. A read that tells nothing keeps the last value seen. A unit not attached
keeps everything it last reported (with `online: false`), so it is as last
seen; it is never written by an alignment. Such a unit gets the issue `no_alink` instead of
`limits_unknown`, no link alarm, and every client shows it as "No audio link
(firmware)", never "Unknown" and never with a role from the model; it is not
counted in "N of M locked". `system.set` keeps a model that names one and
answers `warnings`, one sentence per such unit (empty otherwise).

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

## USB-direct units (WP AUSB1)

A home-theatre install can feed a few speakers per DSPi straight from a central
computer over (active) USB cables, with no audio link cable: the firmware's
USB-direct block (DSPi-bofr `extension_channel_spec.md` 2.9, bofr-main ebdf538:
`EXT_ALINK_USB` 0x8010, `_STATUS` 0x8011, `_DIAG` 0x8012, the USB time base of
0x8005/0x8006, event 0xE3). A unit locks its clock to the host's Start of Frame
(mode 1, rate lock) and, in mode 2 (phase lock), plays every USB sample D samples
after it arrived, so every unit of one host plays sample-aligned.

- **The model.** A unit with `"transport": "usb_direct"` has no `upstream`,
  `slots`, `hopDelayFrames` or `repeater`; `"usbMode"` (`off`, `rate_lock`,
  `phase_lock`) is the mode the alignment writes, absent: the unit's own is kept.
  The Installer's "Clock and audio links" panel sets it (web), saved with
  `system.set` by revision like every other edit. A unit's transport is the
  model's; whether its firmware has the block is read (`observed[serial].usbDirect`
  `present`/`none`, issue `no_usb_direct`, a warning on `system.set`).
- **What the hub reads** for such a unit: `EXT_LIST`, `EXT_ALINK_USB`, `_STATUS` and
  `_DIAG` (not the audio link's commands), in `observed[serial].usb`: the mode and
  D, the state (`off`, `no_sof`, `pull_in`, `rate_locked`, `anchoring`,
  `phase_locked`, `rejected`), host and VCXO ppm, the phase error in samples, the
  bus frame, `dMin..dMax`, the stream's start bus frame and shift, gaps, missed and
  late samples, anchors and re-anchors, and DIAG's timeline ends (with the latest
  cause), holdovers and rides, suspends, bus resets and unplugs.
- **One system delay.** A USB-direct unit's `E_u = D + (L_max − L_u) + S_u`, in
  samples at the stream's rate (the same number as audio link frames), is the D
  written to it, inside its `d_min..d_max` (170..399 at 48 kHz); an automatic D is
  the smallest every unit of both kinds takes. A mixed installation gets the issue
  `mixed_time_bases`: an audio link unit plays D frames after the sender sends a
  frame, a USB-direct unit D samples after a sample reaches it, and the sender's
  own USB latency is not counted, so check a mixed room with a test click and
  enter the measured latency.
- **Start offsets (2.9.3, U6).** A host opens its streams one after the other, so
  the same content can sit in different bus frames. From each phase-locked unit's
  stream start bus frame and `stream_shift` the hub works out how many samples
  each unit's content plays earlier than the latest one's
  (`alignment.units[].startOffsetSamples`, issue `usb_start_offsets`) and adds that
  to its speakers' output delays (a unit with an offset and no speakers in the
  model gets `start_offset_no_outputs`). Only units at one stream rate are
  equalised: one at another rate (96 kHz next to 48 kHz) gets `usb_rate_differs`
  and keeps its offset.
- **Start offsets move** (WP FIXM1). Every time the host reopens a stream (ALSA or
  PipeWire closing and opening the device, a player starting) and every time a
  unit anchors again (a bus reset, a mode or D change, an automatic re-anchor),
  the stream is placed anew and its start offset changes, while the speaker delays
  the hub wrote stay; a bench K340 anchored eight times in an hour. The hub
  therefore remembers the offsets it last wrote (from the first alignment written
  in its run) and compares them at every `system.status`: when one moved, the
  alignment carries the issue `usb_start_offsets_changed`, "USB-direct start
  offsets changed — Align again (… moved since the last alignment …)", which every
  client shows (the core's view sets `alignNeeded`; the web's Alignment panel has
  an "Align again…" button). Offsets are relative, so streams that all moved
  together raise nothing. An alignment that changes a unit's mode or D re-anchors
  it, so expect the issue once more after such an alignment.
- **Keep USB-direct units aligned** (Installer, the model's `keepUsbAligned`, off
  by default; the core's edits `keepUsbAligned`, the view's `keepUsbAligned`).
  On, the hub does not ask: when an offset moved it writes the moved units'
  speaker delays again itself (nothing else, so nothing re-anchors and the audio
  link units are untouched) and records it as the alarm `usb_realigned`. A unit it
  cannot write (offline, out of range, a refused write) stays in the issue.
- **Mixed rates.** One D in samples is one time only at one rate: a USB-direct unit
  at 96 kHz plays its D in 96 kHz samples (half the time of the same number at
  48 kHz, and its limits are 324..351), while the audio link and 48 kHz units count
  D in 48 kHz frames. The hub writes the same number to every unit and does not
  convert it, so a USB-direct unit at 96 kHz next to a 48 kHz audio link (or 48 kHz
  USB-direct units) is not aligned by the system delay: run every unit of a room
  at one rate, or measure that unit with a test click and enter its latency.
  `mixed_time_bases` covers the latency difference between the transports, not
  this unit difference.
- **Writing.** `system.align` with `apply` writes `EXT_ALINK_USB` (`what:
  "usb_direct"`: the mode, automatic re-anchor as the unit has it, and D in phase
  lock) when it differs, then the output delays; skipped with `no USB-direct block
  (firmware)`, `state unknown`, `not connected` or `out of range`.
- **Scenes.** The phase-locked USB-direct units take a scene's ops at one bus frame
  (`EXT_APPLY_AT_FRAME` time base 2, sample 0 of that frame: the same 8 bytes mean
  the same instant on every unit of the host), taken from the most recent read
  (not the largest S: S of different units differ by their origins), plus a lead
  counted from that read that covers the audio link's writes and each USB-direct
  op's write and read-back, at most 1.92 s ahead (`plan.usbFrame`, `plan.usbOps`,
  `plan.usbLeadSamples`);
  path `usb_frame` when nothing else is scheduled. A USB-direct unit that is not
  phase-locked takes its ops at once with its own commands (reason `not
  phase-locked (USB-direct)`). When the audio link's part is refused whole (a
  sender holding a scene), the USB-direct ops are held back too (`skipped`, reason
  `the audio link part was refused`), so no room plays half a scene. The recall
  says each part: "Evening recalled: at USB frame 1234 on Rear L/R, Height; at
  audio link frame 1000032 through the sender on the others; applied now on Plain
  unit (no audio link)."
- **Clients.** The core's view rows carry `transport`, the USB mode (as run and as
  asked), D and its range, host and VCXO ppm, the phase error, gaps, missed and late
  samples, re-anchors, timeline ends and the latest cause, the start offset, and one
  `usbStats` line; the summary counts them apart ("Audio link: 4 of 4 locked,
  USB-direct 2 of 2 locked"). The web shows them in the System view (rooms and the
  topology, beside the audio link's tree), the Installer's tables "USB-direct units"
  and "USB-direct stats", and the alignment's plan; Qt and Android in their System
  screens. `uconsole ext usb-direct` (text or `--json`) reads one unit directly. The
  demo: `uconsole --demo hub --alink` adds "Rear L/R" and "Height" (fake units
  phase-locked at 48 kHz, the host 11.5 ppm slow, Height's stream 3 frames late);
  the core's `open_demo_usb_direct` (wasm `open` `{"kind": "demo", "usbDirect":
  true}`) opens one on its own.
- **Not done.** All USB-direct units of a hub are taken to be on one USB host
  (one SOF): units on two host controllers count different bus frames, and the hub
  does not tell them apart yet. Qt and Android show the units but set no mode
  (the web's Installer does; the core's `alink_system_save` takes `usbModes`).
  No alarm is raised for a USB-direct unit yet (the event log has 0xE3).

## Where it is kept

In the hub's store, `hub-store.json`, under `system`, beside the paired clients
and device names. A hub without a store (the app's in-process hub) keeps the
model for its run only. A model in the file that this hub cannot read is
ignored with a log line and left in the file until a new one is set.

## Not done yet

- Measuring latencies (SYS2 3.5's click and scope or cross-correlation): enter
  `measuredLatencyUs` by hand for now.
- Several senders in one model work for alignment and per-unit scenes; the
  superframe path is taken only when every unit is behind the one sender. The
  engine converts microseconds at the rate most units report, so a minority
  unit on a sender at another rate is converted at the wrong rate.
- `L_max` is taken over the whole model, not per room: a room of fast units
  waits for the slowest unit anywhere (more latency, never misalignment).
- A periodic poll: alarms follow events and `system.status` reads. A client that
  shows the system calls `system.status` every few seconds.
- The native clients' System view and "Clock and audio links" panel: the web console has
  them (WP SW3, `apps/web/README.md`); Qt and Android have them over the core's
  typed calls (WP LKUI1: `alink_system_view`, `alink_system_save` with the revision,
  `alink_system_align`, `alink_system_recall`; C `dspi_alink_system_*`), without the
  slot matrix, link stats, trim scan, system mute toggle or alarm acknowledge
  (`docs/PARITY.md` section 2.I). Swift is pending.
- Resetting a unit's link stats through the hub: `EXT_ALINK_STATS_RESET` is not a
  `system.*` request yet.
