# PLAN-AT: the alink time-base contract and multi-unit measurement

*WP PLANAT, 2026-11-19, written on `main` at `f825b0fd` (PLAN-PG wave A merged; P5-A, P5-B, P5-C part 1 merged;
P5TUNE, P5ARRIVAL in review P5-C2). Firmware: `bofr-main` `8385455` (FWSIGGEN merged), read only. Inputs: the design
study `work/alink-measure-study/REPORT.md` (its WP outline FWTB0-3, P5SYS1-4, H1-H7 is the starting point and is cited
"study 3.2" and so on), the firmware specs `Documentation/Features/{alink_protocol_spec,extension_channel_spec,
alink_receiver,alink_usb,test_signals_spec,SPDIF_input_spec}.md`, SYS2 (`work/sys2/SPEC.md`), PLAN-P5, PLAN-PG, the hub's
`crates/dspi-hub/src/system/`. Plan plus one compiling seam (section 2, "The seam"). No other code.*

**The owner's decisions of 2026-11-19, binding** (memory `alink-timebase-decision`):
- **AT1. The time-base contract is normative**, in the firmware's `alink_protocol_spec.md`: a shared sample counter with
  a `time_base` id, a fixed play-out D, apply-at-T on exactly that sample, lock states and events, a clock-domain
  identity. The alink protocol must work with any transport that meets it.
- **AT2. Multi-unit measurement = each unit's own generator started at a timeline sample:** apply ops `SIGGEN_START` and
  `SIGGEN_STOP` through `EXT_APPLY_AT_FRAME` or the superframe broadcast; START arms the DAC mute ahead (about 250 ms on
  the K340); a late START is refused; a loss cancels; `EXT_SIGGEN_TIMELINE` reads back the proof.
- **AT3. RC5 generalised to one timeline across units:** segments `{unit, start T, walk schedule}`, one slope per clock
  domain, one intercept per (unit, output), interleaved passes, a gate on lock events, the read-back and one domain, an
  anchor output for runs longer than 120 s.
- **AT4. Transports:** the line (SFP and optical) and USB-direct mode 2 qualify already; **S/PDIF becomes a time-base
  transport now**; ADAT, I2S and PTP later. The sender-generated whole-path check only later, if this does not work out.

**Goal.** Measure and align a system of several DSPi units with one microphone, sample-exactly, on any transport that
offers the contract. Every unit plays its own generator (bounded at the unit, per output, firmware-exact, as P5 already
models it); the starts are scheduled on the installation's shared counter; the core's timing model reads one timeline
across units. The measurement never names a transport: the core and the hub talk to "a unit with a time base".

**Rules this plan builds on:** PLAN.md section 1 (the core owns logic; typed writes; no panic on device, network, file or
caller data); PLAN-CLIENTS section 1 (web rules: time through a seam, device I/O behind `Transport`); PLAN-P5 (the
signal path 1.1, the safety rules S1 to S13 1.2, the gate RC5d, the summed check RC5f; all apply per unit); PLAN-PG (the
dual-protocol requirement: every new extension id exists on both envelopes, `REQ_EXTENSION` 0x00 on the 1.1.6 generation
and the tunnel `0xE010` on the 1.1.7 generation, `extension_channel_spec.md` 1.3); the compat policy (memory
`owner-compat-policy`); the test-audio rule (memory `test-volume-limit`: no host player, the generator only); the K340
rules (1.20 V core, never touched; never stock firmware); the process (memory `agent-process`: parallel Opus WPs in
waves of disjoint files, one Fable review per batch, toolchains in containers, no tmpfs, no devices in a WP). **Naming:**
"alink" is the audio link, "DSPi Link" the network protocol (memory `alink-dlink-naming`); a **time base** is a shared
sample counter T; a **domain** is the set of units that share one T numbering; a **segment** is one unit's generator
run started at one T; a **follower** receives a time base, a **source** emits one.

**What exists today (checked on `f825b0fd` and `bofr-main` `8385455`):**

| Need | State | Where |
|---|---|---|
| A shared counter | line: 64-bit F in the superframe, low byte in every frame head; USB-direct: `S = spf * N + k`, N = SOF count with `N mod 2048` = bus frame | `alink_protocol_spec.md` 6.1, 6.6; `extension_channel_spec.md` 2.9.1 |
| Fixed play-out | line `EXT_ALINK_PHASE` (d_local + hops_offset); USB `EXT_ALINK_USB.d`; one portable servo `alink/alink_phase.{c,h}` drives both | ext spec 2.8.5, 2.9.2; `alink_phase.h:1-37` |
| Apply at T | `EXT_APPLY_AT_FRAME` 0x8006 (byte 12 = time base 0, 1, 2), 4-deep queue, ops 1-8, event 0xE2; superframe bytes 16-27 on the line | ext spec 2.8.7, 2.9.4; `alink_input.c:777` `apply_exec`, `:833-848` `deliver_span`; `alink_usb_core.c:392-410` |
| Lock states and events | line `EXT_ALINK_STATUS.state` 0-3 + `EXT_ALINK_PHASE.state` 0-3, event 0xE1; USB `EXT_ALINK_USB_STATUS.state` 0-6, event 0xE3 | ext spec 2.8.2, 2.8.12, 2.9.3, 2.9.5 |
| Clock-domain identity | line: superframe byte 7 (sender low byte) + the hub's model; USB: the hub's knowledge of the controller | `crates/dspi-hub/src/system/model.rs:199-345` |
| Invariant across an interval | `EXT_FRAME_COUNTER.frame - output_frames` constant while phase-locked | ext spec 2.8.6, 2.9.4 |
| Generator on the time base | rendered into the alink's own blocks when a time-base source owns the rings (`pipeline_source_owns_rings()`); **START acts when the request arrives** | `test_signals_spec.md` 9; `siggen.c:1267-1292` |
| DAC mute ownership | FWSIGGEN: the generator opens an idle hold (override), waits `max(hold_ms, release_ms)` (100 ms on the K340), tail 15 ms + delay | `test_signals_spec.md` 9.3 |
| S/PDIF input | rate lock only: `input_servo` feed-forward + fill term; latency = prefill + batch, new after every relock; mute until lock | `SPDIF_input_spec.md`; study 1.1 T3 |
| S/PDIF user bits | RX: the raw subframe words keep VUCP (U = bit 29) in the live FIFO the input reads word by word (`spdif_rx_read_fifo_live`, `spdif_input.c:268-372`); only C bits are collected. TX: the stamping loop sets C per subframe from `spdif_channel_status`, block phase `n % 192` carried as a counter (`pico_audio_spdif_multi/audio_spdif.c:97-132`) | `pico-extras` is in-tree and patched by the fork |
| Measurement maths | one generator timetable: `arrival = intercept(output) + ratio * c_i`, `c_i` = cycle start after the START; START pinned by its acknowledgement | `crates/dspi-measure/src/ir.rs:216-232`, `stimulus.rs:424-560`; `crates/dspi-core/src/measure/session.rs:711-745` |
| The session's device | `measure::Device` (start, stop_now, status, meters, limiter_gr, renew_lock, drain_tap) over one `DspiCore` | `crates/dspi-core/src/measure.rs:583-640` |
| The hub's system | model (senders, fan-outs, units with `UnitTransport::{Alink, UsbDirect}`, speakers), alignment engine (`d_local`, `hops_offset`, `out.delay`), scenes at a frame (lead, superframe vs per-unit path, USB bus-frame form), alarms; `system.scene.apply` is Control, `system.set`/`system.align` Admin | `crates/dspi-hub/src/system/{model,align,scene,run,alarms}.rs`; `server.rs:3433-3436` |
| Ext codecs | transcribed (the firmware has no header for 0x8000+): `dspi_proto::ext::{alink, alink_usb}`; I/O in `dspi_session::ext` | `crates/dspi-proto/src/ext/alink.rs:1-60` |
| Fakes | fake alink and alink_usb: one counter per fake on the clock seam, "the superframe does not reach other fakes" | `crates/dspi-transport/src/fake/alink.rs:1-60` |
| Clients and several devices | the web runs one core Worker per device session (`apps/web/src/core/worker.ts:1`); a `DspiCore` is one device connection | so a multi-unit session cannot hold several cores in the web (OD-ATh) |

**Corrections to the study found while checking:**
- **"S/PDIF cannot be made exact by any generator scheme" (study 0, 1.1 T3)** holds for S/PDIF *as a foreign gear
  carries it*. Between DSPi units the user bits are ours to fill: 384 per 192-frame block, 48 bytes, enough for the
  line's 32-byte superframe (F, sender, apply op) once per block. The receiver already reads the raw words that carry
  them and the transmitter already stamps per-subframe status bits. So S/PDIF between DSPi units *is* a time-base
  transport (section 1.2). Foreign S/PDIF stays rate-lock only.
- **"Already in the queue" (owner, S/PDIF).** No backlog line names an S/PDIF time base. The queued items are SYS2 3.1's
  mode-flag note ("S/PDIF and ADAT keep mode 0 unless a later WP adds a counter for them ... S/PDIF has the 192-frame
  block phase"), followups line 13 (ADAT input phase lock, MAYBE LATER, owner 2026-11-14) and HANDOVER's "ADAT phase
  lock = maybe later". This plan is that later WP for S/PDIF; ADAT stays in "Later". Upstream 1.1.7's S/PDIF
  off-nominal acquire (`bcd5104`, soft VCXO +-1000 ppm seed) is rate acquisition, arrives with the port (PLAN-PG FWPG5,
  `input_servo.c`) and helps followers lock faster; it is not a time base.
- **The study puts the hub call "beside `system::scene`" and a hubless path in the core** (study 3.4). The web cannot
  hold several device cores (one Worker per device), so a hubless path would exist on native clients only. **Multi-unit
  measurement runs through our hub** (OD-ATh); several USB-direct units on one PC run our hub on that PC.
- **`SET_CONFIG` while running restarts the generator** (`test_signals_spec.md` 4.1). The study's "queue one pass
  ahead" therefore cannot restage a config between passes: **one config per unit per run, staged once**; every pass of
  that unit is another op 9 on the same staged config (OD-ATj).
- **The study sizes the work at 20 units** (firmware 6, console 11). The owner's S/PDIF transport adds two firmware WPs
  (6.5 units); the study's P5SYS3 (4 units for protocol, fake, hub call and session) is split into the parts it really
  is, plus a contract read and event that make the core transport-agnostic. **This plan is 13.5 units of firmware and 26
  of app WPs, plus 3.5 of reviews and 2.25 of HIL** (section 2, "Waves").

---

## 1. The design

### 1.1 The time-base contract (normative; FWTB0 writes it into `alink_protocol_spec.md`)

A new section "Time bases" in `alink_protocol_spec.md`, with `extension_channel_spec.md` 2.8.6, 2.8.7, 2.9.4 and the new
2.10 and 2.11 as its control plane. Nothing in it is line-specific. **A transport is an alink time-base transport when a
unit on it offers C1 to C6:**

- **C1. A sample counter T**, 64 bits at the host interface, shared by every unit of one domain, with a `time_base` id.
  Units may count from different origins only where one read resolves it (USB: multiples of `2048 * spf`).
- **C2. Fixed play-out.** A follower plays the sample carried as T exactly D samples after it arrived; D is configured
  per unit; the measured error is reported (`phase_error`). A source plays the sample carried as T at the instant it
  emits T (D = 0 by definition).
- **C3. Apply at T.** A queue of ops that act on exactly the sample carried as T as it enters the pipeline, on audio and
  on silence, each with a result event carrying T; a late op is reported late (op 9: refused, C3 has no late START).
- **C4. Lock states and events.** The contract's states (below), an event for every transition (0xE4), and a
  **disturbance counter** that every loss, holdover, resume, re-anchor, whole-sample correction and epoch change
  increments, readable with T and `output_frames` in one read (`EXT_TIME_BASE`), so a host can prove that nothing
  happened in an interval without relying on events (which are rate-limited and collapse).
- **C5. A domain identity** a host can compare: `EXT_TIME_BASE.domain` (time base in byte 3; line: sender id in byte 0;
  S/PDIF: the source's serial bytes 0-1 and the epoch in byte 2; USB: 0, the host's bus decides, OD-ATg).
- **C6. Discovery.** `EXT_LIST` names `EXT_TIME_BASE` (0x8030) on every build that has a time base; the per-transport
  block names its own ids.

**The contract's states** (`EXT_TIME_BASE` entry `state`, event 0xE4 `state`), mapped from what exists:

| Contract state | Meaning | Line (`ALINK_STATUS.state` / `ALINK_PHASE.state`) | USB-direct (`ALINK_USB_STATUS.state`) | S/PDIF (new) |
|---|---|---|---|---|
| 0 `NONE` | no usable time base | 0; or mode 0 (fill servo) | 0, 1, 3 (rate lock) | mode 0; or a stream with no superframe and mode 1 off |
| 1 `ACQUIRING` | search, pull-in, waiting for a superframe | 1 | 2 | locking, waiting for a valid superframe |
| 2 `ANCHORING` | drain, prefill, start, verify (muted) | 2 / 1 | 4 | anchoring |
| 3 `LOCKED` | T valid, play-out fixed, apply-at-T works | 2 / 2 | 5 | locked (followers); a source with its outputs running |
| 4 `HOLDOVER` | T extrapolated, muted, timeline kept | 3 | 5 with `HOLDOVER` flag | holdover |
| 5 `LOCK_ERROR` | configured to lock but cannot | 2 / 3 (pending) | 6; flag `LOCK_ERROR` | no superframe in mode 1, D out of range, CRC failures |

**Time base ids** (OD-ATb): 0 line F, 1 USB S, 2 USB bus-frame form (apply only), **3 S/PDIF F**, 0xFF none;
reserved for later: 4 ADAT, 5 I2S/TDM, 6 PTP media clock.

**The transports against the contract:**

| Transport | T | How T reaches a follower | Broadcast op path | C1-C6 |
|---|---|---|---|---|
| line L1-L3, fan-out, repeater | F | superframe + head byte; re-emitted with `hops` | superframe (sender `EXT_ALINK_CTRL_MSG`) | exist; C4's counter and 0xE4, C6 added (FWTB1) |
| USB-direct mode 2, 48/96 kHz | S | SOF count + host rate | none; time base 2 gives the same bytes per unit | exist; C4, C6 added (FWTB1) |
| **S/PDIF between DSPi units** | F | the S/PDIF superframe in the user bits, one per block | the same superframe (source `EXT_ALINK_SPDIF_CTRL`) | **new** (FWSP1, FWSP2) |
| USB 44.1 kHz, USB mode 0/1, foreign S/PDIF, ADAT, I2S slave | - | - | - | `NONE`: single-unit measurement only |

A new transport (ADAT, I2S/TDM, PTP) implements C1-C6 and the conformance checks FWTB0 lists (a host-test checklist:
counter continuity across a relock, apply-at-T on a split block, holdover, the disturbance counter, the domain id) and
gets measurement, scenes and CLICK with no host change.

### 1.2 S/PDIF as a time-base transport

- **Roles** (`EXT_ALINK_SPDIF.mode`): 0 off (today's S/PDIF, the default); 1 **follower** (the active input is S/PDIF and
  carries a time base); 2 **source** (the unit stamps its own counter on its S/PDIF outputs). One role per unit in this
  plan; mode 3 (relay: a follower or a USB-direct unit re-emitting its upstream T) is reserved (Later, OD-ATd).
- **The source's counter.** F counts the frames the source's outputs play, from an arbitrary origin set when its
  outputs start in sync: F of a frame is the output-frame index at which the source's DAC outputs play it and its S/PDIF
  outputs emit it (all slots start together, `start_output_slots_in_sync`). Any output restart (preset load, rate change,
  flash write, pipeline reset) starts a new **epoch**: a new origin, the epoch byte incremented, and every follower
  re-anchors (reason `EPOCH`). The source is a member of its own domain: its `EXT_TIME_BASE` lists a source entry, its
  apply queue (`EXT_APPLY_AT_FRAME` time base 3) acts on the sample written at output position F, exactly as the USB
  engine acts on S as it enters the pipeline.
- **The carriage** (FWTB0 specifies it as a section "Carriage on S/PDIF" in `alink_protocol_spec.md`; OD-ATc). The
  user bits of one IEC 60958 block (192 frames, both subframes, 384 bits, starting at the Z/B preamble, user bit i in
  subframe order = byte i/8, bit i%8, LSB first like the C bits) carry one **S/PDIF superframe** of 48 bytes: bytes 0-31
  are the line's superframe layout (6.1) with byte 4 `channels` = 2, byte 5 `class` = **0x10** (S/PDIF carriage, never a
  line class), byte 6 `hops` = 0, byte 7 the source's serial low byte, bytes 8-15 F of the block's first frame (**F mod
  192 = 0** replaces the line's mod-32 rule), bytes 16-27 the apply field (ops 1, 2, 4, 7, 9, 10; others refused), **byte
  28 the epoch, byte 29 the serial's second byte**, bytes 30-31 the CRC-16 over 0-29 as on the line; bytes 32-47 are 0
  and ignored. A superframe carries a pending apply when it completes before the target (`frame + 192 <= N`). The
  reference codec is portable C with a Python twin and its own vector file (FWSP1).
- **The follower.** It reads the user bits from the raw words it already consumes (no RAM-resident change where
  avoidable), accepts a superframe as the line receiver does (CRC, magic, version, class 0x10, rate equal to the detected
  rate, counter continuity: the next block's F is F + 192), and runs the **same `alink_phase` servo** as the line and USB:
  `e = F_arr - (next_w - fill) - D`, with `F_arr` from the RX DMA position against the block start (subframe
  resolution, half a frame, smoothed by the servo's IIR and PI as the line's quarter-frame output quantisation is).
  Anchor, verify, whole-frame corrections, PI, holdover on loss (VCXO trim held, outputs on silence, T extrapolated),
  re-anchor beyond 2 frames, relock placed by F. `d_min`/`d_max` are computed as on the line (2.8.1) plus the
  receiver's in-flight margin and reported; the hub chooses D for the domain as `align.rs` does for the line.
- **Lock error** (OD-ATp): a follower in mode 1 that sees no valid superframe (a foreign source, a DSP or AVR in
  between that rewrites the user bits), D out of range or a rate it cannot lock at is `LOCK_ERROR`; its action is the
  per-unit `on_lock_error` as on USB-direct (owner 2026-11-14): 0 mute (default), 1 play unaligned (today's rate lock).
  Mode 0 stays the default so a TV on S/PDIF is never muted by a forgotten setting.
- **What carries it:** a direct wire (3.3 V logic between boards), coax with a transformer, TOSLINK with passive optical
  splitters. Anything that re-clocks or rewrites the user bits does not; the follower then reports `LOCK_ERROR` and the
  UI says "this S/PDIF source does not carry a DSPi time base".
- **Builds.** RP2350 builds, the K340 included (`DSPI_ALINK_SPDIF`, default on there); RP2040 later (its RAM is spent;
  OD-ATm). The source needs an S/PDIF-type output slot; the follower an S/PDIF input (the K340's RX1 default is GP5).
- **The source's S/PDIF slots are never in a generator mask** (the hub refuses it, S-AT6): followers would receive the
  stimulus as programme.

### 1.3 The generator on the time base: ops 9 and 10, the proof, the contract read

**Apply ops** (`alink_codec.h:70-79`, spec 6.4, ext spec 2.8.7, 2.9.4, 2.10; OD-ATe):

| Op | Name | arg | value | Acts on the sample carried as T |
|---|---|---|---|---|
| 9 | `SIGGEN_START` | 0 | 0 | the generator's sample 0 is that sample; starts the staged `SiggenConfig` |
| 10 | `SIGGEN_STOP` | 0 fade, 1 now | 0 | as `SIGGEN_CTL_STOP` / `STOP_NOW` at that sample |

The rules that differ from ops 1-8:
- **Arming.** The firmware arms op 9 at `T - arm_lead`, where `arm_lead = max(hold_ms, release_ms) + 50 ms` (150 ms on
  the K340), reported in `EXT_SIGGEN_TIMELINE.arm_lead_ms`: from then on the generator "wants output", opens an idle DAC
  hold through the FWSIGGEN override and waits the DAC's un-mute ramp, so its first sample at T is not attenuated. A
  write whose T is less than `arm_lead` ahead is **refused at the write** (STALL / `ERROR`; `VC_ERR_STATE` on the tunnel).
- **Never late.** If T has passed when the op comes due, or the output is not ready at T (DAC still held, the unit muted
  by `REQ_SET_USER_MUTE` or a system-mute flag, time base not `LOCKED`, no valid staged config), the generator does not
  start and the op completes `refused` (3), with `EXT_SIGGEN_TIMELINE.last_result` naming why. `late` (1) is never
  reported for op 9.
- **A loss cancels.** A pending op 9 is cancelled when the time base leaves `LOCKED` (0xE2 `cancelled`,
  `CANCELLED_LOSS`). A running generator keeps today's behaviour (muted with the unit) and the timeline counts a
  discontinuity.
- **A `SIGGEN_SET_CONFIG` while op 9 is pending cancels it** (`CANCELLED_CONFIG`): what starts at T is what was staged
  when the op was written. `STOP`, `STOP_NOW` and `CANCEL_ALL` cancel it too (`CANCELLED_HOST`).
- **Broadcast.** Ops 9 and 10 may travel in the line's superframe and in the S/PDIF superframe. The validity rule
  becomes "op 0..10" (protocol version stays 1: nothing is released; a v1 receiver without the ops rejects such a
  superframe and keeps playing, spec 6.3). A unit with no staged config refuses a broadcast op 9 and stays silent. The
  hub uses the broadcast only for `SIGGEN_STOP` now (the abort without control latency, OD-ATf).
- **A plain `SIGGEN_CTL_START` is stamped too:** while a time base is `LOCKED`, the timeline records the T of its sample
  0, so a single unit's run is on its time base as well.
- **Discovery.** `EXT_LIST` names `EXT_SIGGEN_TIMELINE` on every build with the ops; line builds set
  `EXT_ALINK_INFO.features` bit 6 `SIGGEN_SYNC`. RP2040 builds get the ops on the USB time base (flash only).

**`EXT_SIGGEN_TIMELINE` (0x8031)**, IN, 40 bytes:

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `version` 1 |
| 1 | 1 | `time_base` of `start` (0xFF none) |
| 2 | 1 | `state`: 0 idle, 1 pending, 2 armed, 3 running |
| 3 | 1 | `last_result`: 0 none, 1 started, 2 refused late, 3 refused not ready, 4 cancelled by a loss, 5 cancelled by a config write, 6 cancelled by the host |
| 4 | 2 | `discontinuities` of the current or last run: an output underrun's re-anchor, a pipeline reset, a re-anchor or whole-sample correction, a holdover entered while running, an epoch change |
| 6 | 2 | `arm_lead_ms` |
| 8 | 8 | `start`: T of generator sample 0 of the current or last run (0 none) |
| 16 | 8 | `pending`: T of the pending op 9 (0 none) |
| 24 | 4 | `samples`: generator samples rendered in the current or last run |
| 28 | 4 | `held`: frames of `siggen_hold()` in it |
| 32 | 4 | `run_id`: increments at every start (timed or plain) |
| 36 | 4 | reserved, 0 |

The upstream `SiggenStatus` (0xA7) is not touched.

**`EXT_TIME_BASE` (0x8030)**, IN, 64 bytes: the contract in one read (C4, C5).

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `version` 1 |
| 1 | 1 | `count`: entries, 0..2 |
| 2 | 2 | reserved, 0 |
| 4 | 4 | `time_us`: the unit's `time_us_32()` at the read |
| 8 | 4 | `output_frames`: output frames played since the outputs started in sync |
| 12 + 24 i | 1 | `time_base` |
| +1 | 1 | `role`: 0 follower, 1 source |
| +2 | 1 | `state` (contract) |
| +3 | 1 | `last_reason` (0xE4 reasons) |
| +4 | 4 | `domain` (C5) |
| +8 | 8 | `t`: T at `time_us` (S for USB, F otherwise) |
| +16 | 2 | `disturbances` (wraps) |
| +18 | 2 | `d` (follower; 0 for a source) |
| +20 | 4 | `phase_error`, f32 samples (0 unless locked) |

Entries are ordered follower first. `t - output_frames` is constant while the entry stays `LOCKED` and `disturbances`
does not change. `EXT_FRAME_COUNTER` (0x8005) is unchanged and keeps answering the input's time base.

**`NOTIFY_EVT_TIME_BASE` (0xE4)**, announced by `EXT_NOTIFY_FEATURES` bit 5, 12 bytes: `2, 0xE4, 0, seq`, `time_base`,
`state`, `reason` (0 none, 1 locked, 2 lost, 3 holdover, 4 resumed, 5 re-anchor, 6 correction, 7 disabled, 8 config,
9 lock error, 10 lock ok, 11 epoch), reserved, T low 32 bits. Sent on every contract transition by every transport, at
most one per 250 ms with the clip event's merge rules; the line's 0xE1 and USB's 0xE3 stay as they are.

**The S/PDIF block** (`0x8020`-`0x802F`; ext spec 2.10):
- **`EXT_ALINK_SPDIF` (0x8020)**, IN/OUT 8 bytes: 0 `mode` (0-2), 1 `flags` (bit 0 `AUTO_REANCHOR`, default set), 2-3 `d`
  (u16 samples, follower), 4 `on_lock_error` (0 mute, 1 play unaligned), 5 reserved, 6 `tx_mask` (source: which S/PDIF
  slots carry the superframe, 0 = every S/PDIF-type slot), 7 `state` (IN only, contract state). Refused: mode above 2,
  undefined flags, a nonzero reserved byte, mode 1 with `d` outside `d_min..d_max`, mode 2 on a build without an S/PDIF
  output slot. Persisted with `REQ_SAVE_OUTPUT_CONFIG` like the other alink blocks (not in presets or bulk params).
- **`EXT_ALINK_SPDIF_STATUS` (0x8021)**, IN 48 bytes: state, phase state, rate code, flags (superframe seen, time base,
  holdover, lock error, source, foreign stream), source id (serial bytes 0-1), epoch, F now (u64), phase error (f32),
  VCXO ppm (f32), `d_min`, `d_max`, fill, `lock_count`, `loss_count`, `sf_errors`, `reanchors` (FWTB0 fixes the offsets).
- **`EXT_ALINK_SPDIF_CTRL` (0x8022)**, on a source: OUT 16 bytes in the `EXT_APPLY_AT_FRAME` layout (byte 12 = 3), one
  op carried in the superframe to every follower and applied by the source at the same F; IN 16 bytes, its queue as
  `EXT_APPLY_AT_FRAME` IN.

**One table of new wire, both envelopes.** Every id is an ordinary extension sub-command (ext spec 1.3: "every ID
assigned later"): `REQ_EXTENSION` 0x00 on G1b, the tunnel `0xE010` with the same `wValue` and bytes on G2b; refusals
STALL/`ERROR` on page 0 and `VC_ERR_VALUE`/`VC_ERR_STATE` on the tunnel (1.3.4's rows). The hub's raw forwarding rule
for clients is unchanged (extension requests admin-only); the hub's own `system.measure.*` requests are judged by that
call's role (section 1.5), as `system.scene.apply` already is.

| Id / op | Name | Dir | Len | WP (firmware / console) |
|---|---|---|---|---|
| 0x8006 ops 9, 10 | `SIGGEN_START`, `SIGGEN_STOP` | OUT | 16 | FWTB1 / ATPROTO, ATFAKE |
| superframe op field | ops 9, 10 valid | - | - | FWTB1 (line), FWSP2 (S/PDIF) |
| 0x8020 | `EXT_ALINK_SPDIF` | IN/OUT | 8 | FWSP2 / ATPROTO, ATFAKE |
| 0x8021 | `EXT_ALINK_SPDIF_STATUS` | IN | 48 | FWSP2 / ATPROTO, ATFAKE |
| 0x8022 | `EXT_ALINK_SPDIF_CTRL` | IN/OUT | 16 | FWSP2 / ATPROTO, ATFAKE |
| 0x8030 | `EXT_TIME_BASE` | IN | 64 | FWTB1 (line, USB), FWSP2 (S/PDIF) / ATPROTO, ATFAKE |
| 0x8031 | `EXT_SIGGEN_TIMELINE` | IN | 40 | FWTB1 / ATPROTO, ATFAKE |
| 0xE4, feature bit 5 | `NOTIFY_EVT_TIME_BASE` | event | 12 | FWTB1, FWSP2 / ATPROTO, ATFAKE |
| `EXT_ALINK_INFO.features` bit 6 | `SIGGEN_SYNC` | - | - | FWTB1 / ATPROTO |

### 1.4 RC5 generalised: one timeline across units (AT3)

**The schedule** (`dspi_measure::system`, the seam). A run is a `SystemSchedule`: one domain, one rate, and segments
`{unit, start T, WalkSchedule}` in time order, none overlapping. Cycle c of segment s plays on the domain at
`T_s + start_c` (`start_c` from the segment's walk). A single unit is the one-segment case, so the existing model, its
goldens and its gate do not change (ATSYS proves it: every existing walked golden gives the same result through the
system analyser).

**The fit.** Arrival of cycle c of segment s in capture frames:
`a(c) = I(u_s, o_c) + r_dom * (T_s + start_c - T_ref)`, with one slope `r_dom` (capture frames per domain sample) per
clock domain and one intercept `I` per (unit, output). In this plan a run has exactly one domain (OD-ATg), so one
slope; the analyser carries the domain per segment and refuses a schedule spanning two. The ratio is removed per cycle
in the frequency domain and the repeats averaged coherently as P5 does (PLAN-P5 1.1 step 6). Intercept differences
across units are the acoustic delay plus each unit's play-out delay and post-alink latency: exactly what alignment
removes.

**Cycle identity.** T is commanded, so the stimulus side is exact. The capture side keeps a coarse pin: every
`system.measure.status` reply carries the domain T the hub read in that call, and the client notes its capture frame
when the reply arrives (`TimePin`); the pin with the shortest round trip places T on the capture to a few ms (the
first arrival's window is P5's: 200 ms early to 500 ms late), and the schedule then predicts every later cycle. The
per-unit generator status (4 Hz, through the hub) stays the cross-check (`active_channel`, `cycles_done`).

**The walk** (OD-ATj). Passes interleave across units: pass 1 is unit A's walk over its group outputs, then unit B's,
...; pass 2 again; each a segment with `repeat` = 1 on the unit's one staged config. Segments sit back to back on the
timeline with the config's gap between them. A unit's queue holds 4 ops; the hub keeps at most two op 9 pending per
unit and writes the next as segments complete. So the slope's baseline is the whole run and a burst of microphone
wander cannot bias one unit against another. Passes: 3 by default (P5's `min_passes_reliable`), 2 to 8.

**The gate** (RC5d additions; G1-G4 discard the run, G5 marks it timing unreliable as today):
- **G1 one domain:** every unit's `EXT_TIME_BASE` entry has the same `time_base` and `domain`; USB units also the same
  host bus (`nusb`'s `bus_id()` from the hub's enumeration; where a platform gives none, the bus-frame test alone) and
  bus frames agreeing within one frame in one read tick. Checked at open (the
  plan step refuses: "these units do not share a clock") and at the end.
- **G2 nothing moved:** per unit, the entry is `LOCKED` at open and at the end, `disturbances` unchanged, `t -
  output_frames` unchanged.
- **G3 the proof:** per segment, `EXT_SIGGEN_TIMELINE` read after it shows the expected `run_id`, `start` equal to the
  scheduled T (in the unit's own numbering), `last_result` started, `discontinuities` 0, `samples` the segment's
  length.
- **G4 no event:** the hub saw no 0xE1/0xE3/0xE4 transition, abort or foreign stop on any unit during the run.
- **G5** the existing bounds: every cycle's residual at most 10 us, phase agreement as P5ARRIVAL left it.

**Across runs: the anchor output** (OD-ATk). A run is at most 120 s and 8 Mi capture frames (S7, RC8). A group that
does not fit is split into runs that all repeat one **anchor output** (default: the first unit's lowest-latency full-range
output, user-selectable); intercepts are kept relative to the anchor's in each run. Chaining requires the anchor's
responses to agree across runs (magnitude spread at most 1 dB in 200 Hz to 5 kHz, arrival within 20 us after the drift
fit), else "the microphone or the room changed between runs" and nothing is proposed. The microphone does not move
between the runs of one alignment.

**What a run yields.** Per (unit, output): the impulse response and response (P5's), saved as that unit's measurement
document (P5MDOC's kind, one per unit, with an additive `system` block: group, domain, anchor, run ids, intercept
relative to the anchor). Per group: the arrivals relative to the latest output, and the relative polarity at each
inter-unit transition (reported).

### 1.5 Where it runs: the hub family `system.measure.*` and the client session

**The split** (OD-ATh). The client's core owns the microphone, the plan, the analysis, the per-unit safety judgement and
the documents. **Our hub owns the units' side:** it reads the time bases, stages the configs, schedules the starts,
watches every unit's events and meters, and aborts on its own (dead-man). The client reaches it with one DSPi Link
session. A client on a hub without the cap `system_measure` (an upstream hub, an older hub of ours, or USB only) offers
single-unit measurement only, with the notice "measuring several units together needs the DSPi hub".

**Messages** (hub-api section 23, ATHUB; JSON as the other families; errors typed as section 1.6 of hub-api):

| Message | Role | What |
|---|---|---|
| `system.measure.open {units:[serial], lease_ms?}` | Control | Takes each unit's device lock ("Measuring") for the session; reads every unit's `EXT_LIST`, `EXT_TIME_BASE`, `EXT_SIGGEN_TIMELINE` (arm lead), platform and output count; checks G1. Answers `{session, domain, units:[{serial, time_base, role, state, d, arm_lead_ms, outputs, t, t_minus_output}]}` or `not_one_domain`, `not_locked {serial}`, `no_timeline {serial}`, `offline`, `locked`, `installer_lock` |
| `system.measure.stage {session, serial, config}` | Control | Validates a 36-byte `SiggenConfig` (base64) with P5's static rules (finite, never `RAW`, sweep bounds, level at most the hard cap, the unit's S/PDIF source slots not in the mask; the pure validator in `dspi-measure`, never a copy) and writes 0xA4 |
| `system.measure.run {session, segments:[{serial, offset}], lead_ms}` | Control | `T0` = the newest T read now + `max(lead_ms, max arm_lead + 100 ms + measured write time)`; each segment's op 9 at `T0 + offset` in the domain numbering, mapped to each unit's own (USB: per-unit S origin resolved from the open's reads, written as time base 2 within 1.98 s or as S beyond); writes the first two per unit and the rest as earlier ones complete. Answers `{t0, segments:[{serial, t, t_unit}]}` |
| `system.measure.status {session}` | Control | Renews the lease. Per unit: `SiggenStatus`, `EXT_SIGGEN_TIMELINE`, the `EXT_TIME_BASE` entry (state, disturbances, `t - output_frames`), output peaks and clip flags, limiter gain reduction, events since open; plus `{t, hub_us}` read in this call (the time pin) |
| `system.measure.abort {session, reason}` | the session's owner, or Admin | `SIGGEN_CTL_STOP_NOW` on every unit, `CANCEL_ALL` on every time-base queue, a broadcast `SIGGEN_STOP` now through the sender or S/PDIF source where there is one; answers per unit |
| `system.measure.close {session}` | owner | `CANCEL_ALL`, releases the locks |
| event `system.measure.aborted {session, serial?, reason}` | to the owner | the hub aborted on its own |

**The hub aborts by itself** (dead-man, fail closed): no `status` for 2 s (the lease); any unit's contract transition or
disturbance, a clip event (0xE0) or an output peak at -0.5 dBFS or above, a foreign generator stop, a preset load, a
foreign write on a measured unit, a unit offline; the session's Link connection closing. The client's own aborts (P5
S4, S5) call `abort`; if that fails it sends `STOP_NOW` through any unit connection it has.

**The client session** (`dspi_core::measure::system`, ATCORE): the P5 session's stages per unit through a hub-backed
`measure::Device` per unit (`start` = stage + a one-segment run at the next T, `stop_now` = abort of that unit, `status`,
`meters`, `limiter_gr` from `system.measure.status`), so the preflight (S2, S6, S8, S13), the ramp (S3) and the clip and
foreign-change aborts run per unit unchanged; then the timed system run (section 1.4), the analysis in slices, the gate,
the documents.

**The proposal** (OD-ATl). The alignment across units is written into the hub's system model, never as direct
`out.delay` writes from the client: each measured speaker gets `measured_arrival_us` (relative to the group's latest)
with the run ids, and `system.align` (the hub's engine, the single writer of `d_local`, `hops_offset` and `out.delay` for
system units) uses measured arrivals where present instead of distance delays (lip-sync and post-alink latency terms
unchanged). Applying needs Admin (`system.set` and `system.align` are Admin today); a Control user can measure, see and
save the proposal. **RC5f applies across units:** before a delay is offered, one summed run per inter-unit transition
with the current chains (the units of the transition started by op 9 at one T, `invert_mask` at their current net
polarities) must agree with the sum predicted from the walked run within P5's 1.5 dB RMS, else every cross-unit delay is
withdrawn with the reason. Before applying, the hub's system model revision and every unit's snapshot are filed; after
applying, the summed runs are repeated with the new delays (P5's verify step); Keep or Revert, and Revert restores the
model and the units. Polarity changes, crossover
corners and EQ across units are not proposed in this plan (each unit's P5TUNE does them within the unit; Later).

### 1.6 Safety

PLAN-P5's S1 to S13 apply **per unit** unchanged (the generator only, volume, ramp, clip, foreign changes, the tweeter
rule, finite runs, silence first, bounded proposals, guarded and reversible, locks and roles, privacy, no unmodelled
processing). Added:
- **S-AT1. Abort reaches every unit:** hub abort (per-unit `STOP_NOW` + `CANCEL_ALL` + broadcast `SIGGEN_STOP` now), the
  client's fallback, the hub's dead-man (2 s), and each unit's own finite run.
- **S-AT2. A late START never starts** (op 9 refused, never `late`); a loss cancels a pending START; the generator stays
  muted with its unit in holdover.
- **S-AT3. Lead and arming:** the lead is at least `max(arm_lead) + 100 ms` (250 ms on the K340, H7 checks it); a shorter
  T is refused at the write.
- **S-AT4. One generator at a time per group** (segments do not overlap), except a summed run. A summed run across
  units plays at the minimum of its outputs' ramp results less 6 dB for every doubling of the outputs it starts
  together (correlated outputs add up to 6 dB per doubling at the transition; a rule of this plan, never above P5's
  ceilings).
- **S-AT5. Locks:** every unit's device lock is held for the session; a lock lost on one unit aborts all.
- **S-AT6. The S/PDIF source's time-base slots are never measured outputs** (they would feed the stimulus to every
  follower as programme).
- **S-AT7. The K340's core voltage is never written** by any WP or HIL step; every firmware WP builds the K340 image with
  `-DDSPI_VREG_MV=1200` and leaves `core_vreg` untouched.

---

## 2. Work packages

**Shared rules for every AT WP.** `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief (`work/wave1-brief.md`:
containers only, no tmpfs, no devices, scoped checks, never push, the commit trailer), PLAN.md section 1, and:
- **Firmware WPs** branch from `bofr-main` in their own worktree of `~/prog/DSPi`, build in the firmware container
  (K340 `-DDSPI_USB_MIDI=ON -DDSPI_VREG_MV=1200`, pico2, pico), run `bash firmware/tests/run.sh`, and run
  `scripts/check_ram_placement.py`: **the K340 build's `.data` grows by at most 64 bytes per WP** (the hot paths stay in
  flash; the coordinator updates the RAM baseline at the batch merge, a WP only reports its delta). No `core_vreg` code
  is touched (S-AT7). Cold code goes to flash; nothing new runs in an ISR except where the entry says.
- **Both envelopes (PLAN-PG):** every new id is served by the one extension dispatcher, so it reaches both envelopes.
  Firmware: on today's 1.1.6 line through `REQ_EXTENSION`; when FWPG2 (the tunnel) lands, its golden test
  `firmware/tests/test_ext_page.c` covers the ids present: whichever of FWPG2 and an AT firmware WP merges later adds the
  rows for these ids under a marker. Console: codecs and fakes are tested on the G1b fake and on PGFAKE1's `Bofr117`
  (tunnel) fake; the session and hub tests run on both.
- **Independent oracles** (as PLAN-PG): the codecs (ATPROTO) and the fake's handlers (ATFAKE) are written separately from
  FWTB0's spec and the seam's constants; the firmware's Python twins and vector files are the third oracle; the
  simulated world (ATSIM) is separate code from the analyser (ATSYS).
- **Typed errors, no panics** on device, hub or capture data; every decoder and every numeric routine has a `proptest_*`.
- **Bindings** (C, UniFFI, wasm) are shared and additive under markers: `ci/drift.sh write`, the Android contract, new
  methods classified in `WriteGuardedCore`. **Notices:** one block per WP in `assets/notices/notices.json`.
- **Tests carry the WP id** (`wp_atsys_*`, `test_fwtb1_*` and so on). **Sizes** are this project's units (about one hour
  of agent time with review).

### The seam (committed with this plan)

- `crates/dspi-proto/src/ext/timebase.rs`: the wire constants of section 1.3 (ids 0x8020-0x8022, 0x8030, 0x8031,
  lengths, time base ids, roles, contract states, reasons, ops 9 and 10, `APPLY_OP_MAX`, feature bits, timeline states
  and results, S/PDIF modes and carriage constants), transcribed from this plan; a test that the ids are in the fork's
  block range and unassigned; compile-time layout checks. One `pub mod timebase;` line in `ext.rs`. **FWTB0 must match
  it;** a difference found there is decided by the coordinator before wave AT-B and the seam re-cut in one commit.
- `crates/dspi-measure/src/system.rs`: `UnitIndex`, `DomainId`, `Segment` (`end`), `TimePin`, `AnchorOutput`,
  `SystemSchedule` (`new` validates: not empty, at most `MAX_SEGMENTS` 128, units below `MAX_UNITS` 16, one rate, in
  order and not overlapping; `single`, `segments`, `start`, `end`, `units`), tests. One `pub mod system;` line in
  `lib.rs`. ATSYS and ATSIM build on these types; neither edits them except additively under its marker.

Checked: `ci/run.sh 'cargo test -p dspi-measure --lib wp_planat && cargo test -p dspi-proto --lib wp_planat && cargo
clippy -p dspi-measure -p dspi-proto --all-targets -- -D warnings && cargo fmt -p dspi-measure -p dspi-proto -- --check'`:
3 tests pass, clippy and fmt clean.

### Wave FW-A: the contract on paper (firmware repo, docs only)

**FWTB0: The time-base contract, ops 9/10, the S/PDIF carriage and block, the time-base block (docs).** [TODO] Size
1.25. Wave FW-A. **Owns:** in `Documentation/Features/`: `alink_protocol_spec.md` (new section "Time bases" with C1-C6,
the state table, the transport table and the conformance checklist; new section "Carriage on S/PDIF"; 6.3 validity
"0..10"; 6.4 op table rows 9, 10), `extension_channel_spec.md` (1.2 range rows `0x8020-0x802F` S/PDIF time base,
`0x8030-0x803F` time base and measurement; 2.4 bit 5; 2.8.1 features bit 6; 2.8.7 and 2.9.4 ops 9 and 10 with section
1.3's rules; new 2.10 S/PDIF block with byte offsets of 0x8021; new 2.11 `EXT_TIME_BASE`, `EXT_SIGGEN_TIMELINE`, event
0xE4; 4 implementation map rows; 5 history), `test_signals_spec.md` (new section "Start on the time base": arming,
never late, loss and config cancel, the timeline, plain START stamped), `SPDIF_input_spec.md` (a pointer section to the
follower mode). **Done when:** every number in section 1.3 of this plan and in the seam appears once in the specs with
the same value (a checklist in the report: id, length, offset, code, against `timebase.rs`); each new structure has a
byte example; the carriage section has one complete 48-byte superframe example with its CRC computed by the line's
reference Python (`tools/alink/`, run in the firmware container); the spec says plainly that foreign S/PDIF, ADAT, I2S
and USB 44.1 kHz have no time base. **Deps:** this plan accepted. Short Fable review (0.25) before FW-B and AT-B start.

### Wave FW-B: the ops on the line and USB; the S/PDIF carriage

**FWTB1: Ops 9 and 10, `EXT_SIGGEN_TIMELINE`, `EXT_TIME_BASE` and 0xE4 on the line and on USB.** [TODO] Size 4.5. Wave
FW-B. **Owns:** `firmware/DSPi/alink/alink_codec.{c,h}` (op constants, validity), `tools/alink/` (the Python twin's op
table) and `firmware/tests/alink_vectors.json` (regenerated), `alink_input.c` (`apply_exec` ops 9/10, the line's
disturbance counter and 0xE4), `alink_usb/alink_usb_core.c` (the same on USB), `siggen.c`/`siggen.h` (start at a sample:
`siggen_arm_at()`, the never-late check, the timeline record, the discontinuity counter, plain START stamped),
`siggen_dac.{c,h}` (arming through the override at `T - arm_lead`), new `firmware/DSPi/timebase.{c,h}` (the
`EXT_TIME_BASE` aggregator: a provider table each transport registers into, with a slot reserved for S/PDIF; the 0xE4
emitter with the 250 ms merge), the extension dispatcher's rows for 0x8030/0x8031 under a marker, `notify` feature bit 5,
`EXT_ALINK_INFO.features` bit 6, `firmware/DSPi-Sender` (`EXT_ALINK_CTRL_MSG` accepts ops 9 and 10, superframe validity),
`tools/midi_ctrl/fake_device.py` and `tests/test_midi_ctrl.py` (ops, ids, examples), host tests: `test_alink_phase` (queue
cases with op 9), `test_siggen_timing` (sample 0 on T exactly with a split block, on silence and on audio), `test_alink_usb`
(op 9 by S and by bus-frame form), `test_siggen_dac` (arm at `T - arm_lead`, refused when the lead is short, DAC open at T),
new `test_timebase.c`. **Done when:** host tests show generator sample 0 on the sample carried as T on both transports;
a late op 9 is refused with `REFUSED_LATE` and never reported late; a pending op 9 is cancelled by a holdover, by a 0xA4
write and by `CANCEL_ALL` with the matching `last_result`; `discontinuities` counts an underrun re-anchor, a re-anchor and
a holdover entered while running; `EXT_TIME_BASE` maps every line and USB state as section 1.1's table says and
`disturbances` counts every listed event; 0xE4 is sent once per transition with the merge rule; the sender relays op 9 in
the superframe and a v1 decoder vector with op 9 is still rejected as invalid; RP2040 builds have the ops on USB with
`.data` unchanged; RAM check PASS. **Deps:** FWTB0.

**FWSP1: The S/PDIF carriage: codec, vectors, user bits in and out.** [TODO] Size 2. Wave FW-B. **Owns:** new
`firmware/DSPi/alink/alink_spdif_codec.{c,h}` (portable C11 like `alink_codec`: pack and unpack the 48-byte superframe,
validity in the line's order plus class 0x10 and F mod 192, the user-bit order), new `tools/alink/alink_spdif.py` (the
Python twin), new `firmware/tests/alink_spdif_vectors.json`, new `firmware/tests/test_alink_spdif_codec.c`,
`firmware/pico-extras/src/rp2_common/pico_audio_spdif_multi/` (a per-instance user block, double-buffered at the block
boundary, OR'd into each subframe's U bit by the stamping loop; silence prestamps carry it too), `spdif_input.c` (only a
user-bit collector on the words it consumes, block-aligned on the B preamble, handing complete 48-byte blocks and their
first word's DMA position to a callback; no behaviour change), new `firmware/tests/test_spdif_ubits.c`, the CMake lines
under a marker. **Done when:** C and Python agree on every vector (typical, every validity failure, apply carriage edge
`frame + 192 = N`, epoch change); a host test stamps a user block through the TX library's stamping code and reads it back
bit-exactly through the collector, across ring wraps and at 44.1/48/96 kHz; an existing S/PDIF output with no user block
set emits all-zero user bits (byte-identical to today); the collector costs nothing when no callback is set; K340 `.data`
delta at most 64 bytes; RAM check PASS. **Deps:** FWTB0. (Disjoint from FWTB1: different files; CMake lines additive.)

### Wave FW-C: the S/PDIF roles; the generator simulations

**FWSP2: S/PDIF source and follower on the time base.** [TODO] Size 4.5. Wave FW-C. **Owns:** new
`firmware/DSPi/alink_spdif.{c,h}` (the roles: the source's counter, epoch and apply queue on time base 3, the superframe
it hands to FWSP1's user block, `EXT_ALINK_SPDIF_CTRL`; the follower's superframe acceptance, `F_arr` from the DMA
position, the `alink_phase` driving: anchor, verify, corrections, PI, holdover, re-anchor, lock error and
`on_lock_error`; op 9/10 on time base 3 through FWTB1's `siggen_arm_at()`), its rows in FWTB1's `timebase.c` provider
table and the dispatcher's rows for 0x8020-0x8022 (markers), `spdif_input.c` (the follower hooks: phase mode replaces
the fill term, mute rules), `main.c` (one service call and `pipeline_source_owns_rings()` for a locked follower, markers),
the persisted block next to the other alink blocks in `flash_storage.c` (marker), new `tools/alink_spdif_sim/` (source +
followers through the portable code, with the RX quantisation and jitter modelled), new
`firmware/tests/test_alink_spdif.c`. **Done when:** host tests: a follower anchors, locks and plays the sample carried as
F at `F + D`; a missing superframe in mode 1 is `LOCK_ERROR` and mutes (or plays unaligned with `on_lock_error` 1); a cut
of 50 ms is a holdover that resumes on the same timeline; a source epoch change re-anchors every follower with reason
`EPOCH`; CRC errors are counted and never move the timeline; op 9 on time base 3 starts the generator on its sample on a
follower and on the source; `EXT_TIME_BASE` lists the source and follower entries with the domain of C5; the simulation:
a source and two followers, skew between the followers under 0.1 frame at 48 and 96 kHz after 10 s, and the
source-to-follower latency identical (to within 0.1 frame) after a follower relock, a source epoch change and a
re-anchor; mode 0 is byte-identical to today's S/PDIF input (a regression run of the existing S/PDIF tests); K340
`.data` delta at most 64 bytes; RAM check PASS. **Deps:** FWTB1, FWSP1.

**FWTB3: The generator on the time base, simulated (line, fan-out, USB).** [TODO] Size 1.25. Wave FW-C. **Owns:**
`tools/alink_phase_sim/`, `tools/alink_usb_sim/` (two and four units with generators started at one T; the simulation
renders each unit's output samples) and new `tools/alink/siggen_sync_bench.py` (the bench driver ATHIL uses: schedules op 9
on several units over USB vendor requests, reads the timelines; runs against `midi_ctrl`'s fake in its tests).
**Done when:** the simulations show two units' generator outputs coinciding sample for sample plus the lock error
(under 0.05 frame on the line, under 0.1 sample on USB); staggered starts land on their samples; a loss mid-run cancels
pending starts and the timeline counts the discontinuity; a late op is refused; a unit behind a fan-out lands on the same
sample after `hops_offset`; the bench driver's tests pass against the fake. **Deps:** FWTB1.

One Fable review for FW-B and FW-C together (1.0).

### Wave AT-A: the maths and the simulated world (pure; console)

**ATSYS: The analyser over segments.** [TODO] Size 3. Wave AT-A. **Owns:** `crates/dspi-measure/src/system.rs` (additive
below the seam's types) and new `crates/dspi-measure/src/system/` (analysis, gate inputs, anchor chaining, tests),
additive `pub(crate)` visibility in `ir.rs` under a marker where the system analyser reuses P5's per-cycle arrival,
deconvolution and averaging, new `stimulus::check_config_bounds` (the static S-rules on a `SiggenConfig`'s fields, pure,
for the hub and the core) in `stimulus.rs` under a marker. **Implements:** `SystemCapture` (samples, first frame, rate,
the schedule, time pins, per-unit status observations), `analyse_system` in slices like `WalkedAnalyser`, the fit of
section 1.4 (slope per domain, intercept per (unit, output)), cycle identity from the best pin, the gate inputs G1-G4 as
plain facts the session fills (`UnitFacts`, `SegmentProof`) and G5, the anchor-output chaining with its consistency check,
interleaved-pass helpers (`interleave(units, passes, gap)` building the segments). **Done when:** every existing walked
golden and P5ARRIVAL replay gives the same per-output result through `analyse_system` with one segment as through
`analyse_walked` (bit-equal intercepts and gate); synthetic captures built from the existing single-unit renderer with
per-unit offsets recover inter-unit delays within 2 us; a segment whose proof says `start` differs by one sample is
discarded by G3; a schedule with two domains is refused; three runs chained through an anchor recover the delays
within the sum of their residuals, and a moved microphone (anchor spread 3 dB) refuses the chain; `proptest_*` for every
public function; `cargo test -p dspi-measure`. **Deps:** this plan's seam; P5-C2 (P5TUNE, P5ARRIVAL) merged.

**ATSIM: The multi-unit simulated world.** [TODO] Size 2. Wave AT-A. **Owns:** new `crates/dspi-measure/src/sim/system.rs`
(one `mod` line in `sim.rs` under a marker). **Implements:** a `SystemWorld`: N units, each with its outputs, drivers and
paths (P5SIM's types), a play-out offset (D and post-alink latency), a lock jitter (white plus a slow wander), a per-unit
start error in samples, faults (a unit that never starts, a holdover or re-anchor at a given T, a whole-sample correction
without an event, an epoch change), a domain per unit (two domains possible), a fan-out hop error of one frame, an S/PDIF
source with followers, and the microphone clock of P5SIM with P5SPIKE's wander bursts (55-100 ppm for 0.25-1.5 s, within
+-50 us); it renders the capture of a `SystemSchedule` and returns the ground truth (each output's true arrival, the
faults' times). **Done when:** rendered arrivals match closed-form arrivals within 0.1 us with jitter off; every fault
appears in the ground truth at its T; memory bounded by P5SIM's constants; `cargo test -p dspi-measure --features sim`.
**Deps:** this plan's seam. (Disjoint from ATSYS; ATSIM does not call the analyser, ATE2E joins them.)

### Wave AT-B: the wire in the console, the fakes

**ATPROTO: Codecs and session I/O for the new wire.** [TODO] Size 2. Wave AT-B. **Owns:**
`crates/dspi-proto/src/ext/timebase.rs` (codecs below the seam's constants), `ext/alink.rs` (`ApplyOp::SiggenStart`,
`SiggenStop`, the superframe validity to 10, marker), `ext.rs` (`name()` rows for 0x8020-0x8031, marker), new
`crates/dspi-session/src/ext_timebase.rs` (read/write helpers over `dspi_session::ext`: `read_time_base`,
`read_siggen_timeline`, `read_alink_spdif{,_status,_ctrl}`, `write_alink_spdif`, `write_spdif_ctrl`, `apply_at`
with ops 9/10; event decoding behind feature bit 5), one `mod` line in `dspi-session/src/lib.rs`. **Done when:** every
example of FWTB0's specs decodes and encodes byte-exactly; short replies are `None`, extra bytes ignored; the helpers
run on the G1b fake (`REQ_EXTENSION`) and on PGFAKE1's `Bofr117` (tunnel) with the same results, and on G1s/G2s refuse
locally (not listed) with nothing sent; `proptest_*` for every decoder; `cargo test -p dspi-proto -p dspi-session`.
**Deps:** FWTB0 merged (and its review), PG-B2 merged (PGFAKE1, PGADAPT).

**ATFAKE: Fake units on one time base.** [TODO] Size 2.5. Wave AT-B. **Owns:** new
`crates/dspi-transport/src/fake/timebase.rs` (and `fake/timebase/`), additive lines under markers in `fake.rs` (one `mod`
line, one `FakeOptions` field `time_base: Option<FakeTimeBaseHandle>`), `fake/alink.rs`, `fake/alink_usb.rs` (their
counters read the shared time base when one is set; the superframe broadcast reaches every fake on it),
`fake/analysis.rs` (op 9/10 on the fake generator: arming lead, never late, cancels, the timeline record),
`fake/measure_hooks.rs` (read each unit's recorded starts for rendering), `fake/diag.rs` (dispatch rows, both envelopes).
**Implements:** `FakeTimeBase` shared by N fakes (domain, rate, per-unit origin offsets for USB, per-unit D, the S/PDIF
source role), fault injection (holdover, resume, re-anchor, correction with or without an event, epoch change, lock
error), `EXT_TIME_BASE`, `EXT_SIGGEN_TIMELINE`, the S/PDIF block, event 0xE4, written from FWTB0's spec, not from
ATPROTO's codecs. **Done when:** three fakes on one time base agree on T within the fake clock's tick; op 9 on each starts
its fake generator on its T and `EXT_SIGGEN_TIMELINE` reports it; every refusal and cancel of section 1.3 is reproduced;
each injected fault moves `disturbances` and sends 0xE4; the same tests pass through `REQ_EXTENSION` and through the
tunnel; `cargo test -p dspi-transport`. **Deps:** FWTB0 merged, PG-B2 merged.

One Fable review for AT-A and AT-B together (0.75).

### Wave AT-C: the hub, the hub's model, the client session

**ATHUB: The hub family `system.measure.*`.** [TODO] Size 4. Wave AT-C. **Owns:** new
`crates/dspi-hub/src/system/measure.rs` (and `system/measure/`: sessions, scheduler tick, dead-man, aborts, the USB
domain), one `mod` line in `system/mod.rs`, `crates/dspi-hub/src/usb.rs` (keep each device's
`nusb` `DeviceInfo::bus_id()` from the enumeration, marker), `server.rs` (dispatch and role lines for the
family, the cap `system_measure` in `hello.caps`, markers), `docs/integrator/hub-api.md` (new section 23 and the rows of
section 18). **Implements:** section 1.5's messages, the gate's G1 at open, the per-unit mapping of domain T (USB
origins), the two-ahead op queue, the hub's own aborts, judging each config with `stimulus::check_config_bounds` and the
limits profile and installer lock per unit, attribution of every write to the session's principal. **Done when:**
in-process hub tests with ATFAKE's units (line, USB-direct, S/PDIF worlds): open refuses two domains, an unlocked unit, a
unit without the timeline, a locked unit and a Viewer; `run` writes op 9 at `T0 + offset` in each unit's numbering
(checked on the fakes' recorded starts, USB through time base 2 and S); the queue never holds more than two op 9 per
unit and fills as segments end; abort stops every unit and cancels every queue, by the owner, by an Admin and by the
dead-man after 2 s without `status`; a disturbance, a clip event, a foreign preset load and a closed connection each abort
and send `system.measure.aborted`; an upstream-shaped client without the cap gets `unsupported`; tests on G1b and G2b
fakes; `cargo test -p dspi-hub`. **Deps:** AT-A, AT-B.

**ATHUBM: The hub's system model: the S/PDIF transport and measured arrivals.** [TODO] Size 2. Wave AT-C. **Owns:**
`crates/dspi-hub/src/system/{model,align,alarms,run}.rs` (additive under markers), `docs/integrator/audio-link.md`
(S/PDIF units, measured arrivals). **Implements:** `UnitTransport::Spdif` with `upstream` = the source unit's id and a
`spdif_source` flag on a unit; the align engine writes `EXT_ALINK_SPDIF` (mode, D, `on_lock_error`) on followers and the
source as it writes `EXT_ALINK_USB` today, chooses D for an S/PDIF domain from the followers' `d_min`/`d_max`;
`Speaker.measured_arrival_us` (with run ids and a timestamp) used instead of the distance delay where present;
`system.status` and alarms read `EXT_TIME_BASE` and 0xE4 for every transport (S/PDIF lock error and loss alarms like
USB-direct's). **Done when:** model validation accepts an S/PDIF source with followers and refuses a follower whose
upstream is not a source, a source used as a measured output's slot, a cycle; align tests: the S/PDIF writes, D chosen
within every follower's limits, measured arrivals giving the expected `out.delay` (lip-sync and latency terms
unchanged), distance delays where no measurement exists; an existing model JSON loads unchanged; `cargo test -p
dspi-hub`. **Deps:** AT-B.

**ATCORE: The client's system measurement session.** [TODO] Size 5. Wave AT-C. **Owns:** new
`crates/dspi-core/src/measure/system.rs` (and `measure/system/`), one `mod` line in `measure.rs` (marker), new
`crates/dspi-core/src/measure/tests_wp_atcore.rs`, bindings (`dspi-ffi`, `dspi-capi`, `dspi-wasm`, markers), one notices
block, the additive `system` block in `crates/dspi-measure/src/doc.rs` (marker). **Implements:** `measure_system_preflight`,
`measure_system_start` (group: units by serial and outputs, passes, anchor), `measure_system_state`, `_tick`, `_stop`,
`_result`, `measure_system_proposal`, `measure_system_apply` (Admin: model + `system.align`, filing the model revision and
unit snapshots; Revert) and the summed check (RC5f); the hub-backed `measure::Device` per unit; the planner (interleaved
segments, run split with the anchor, the lead from the units' arm leads); the gate's facts from `status`; documents per
unit. The hub messages are section 1.5's; the tests drive a scripted hub stub (the real hub joins in ATE2E). **Done
when:** the stub tests cover a two-unit and a four-unit plan (segments, offsets, lead 250 ms with a K340-like arm lead);
each per-unit preflight refusal (S2, S6, S8, S13) names its unit; a clip, a foreign change, a disturbance, a failed proof
and an abort event each end the run with nothing proposed and the unit named; a Control role can measure but not apply;
a summed run that disagrees with the prediction withdraws every cross-unit delay; apply files the revision and
snapshots, repeats the summed runs, and Revert restores them; a hub without the cap gives the notice; `cargo test -p
dspi-core`, `ci/drift.sh`, the Android contract regenerated. **Deps:** AT-A, AT-B; P5-C2 merged.

One Fable review for AT-C (0.75); the full gate after AT-C.

### Wave AT-D: end to end, the clients, the docs

**ATE2E: Simulated multi-unit worlds end to end.** [TODO] Size 1.5. Wave AT-D. **Owns:** new
`crates/dspi-core/tests/wp_ate2e_system.rs`, new `crates/dspi-measure/src/tests_wp_ate2e.rs` (one `mod` line, marker).
**Implements** the study's 4.3 list against the real hub in-process, ATFAKE's units and ATSIM's world rendering the
capture from the starts the fakes recorded: two and four units with different D, one behind a fan-out: inter-unit delays
within 2 us of the world's; a START one sample late: recovered as 20.8 us at 48 kHz (or discarded by G3 when the proof
shows it); holdover on one unit in pass 2: aborted, the unit named, nothing proposed; a whole-sample correction without
an event: caught by G2; two domains: refused at open; P5SPIKE-like microphone wander: interleaved passes keep the
inter-unit bias under the gate or the gate trips, never a confident wrong delay (100 seeds); three chained runs through
an anchor; an S/PDIF source with two followers; every case on G1b and G2b fakes. **Done when:** all of those pass;
`cargo test -p dspi-core --test wp_ate2e_system` and `-p dspi-measure --features sim`. **Deps:** AT-C.

**ATWEB: Web: measuring several units.** [TODO] Size 1.5. Wave AT-D. **Owns:** new
`apps/web/src/screens/measure/system/` (an "alignment group across units" step in P5WMS's measure screen through its
tab slot), the S/PDIF transport in the web's system editor (unit fed by S/PDIF from a source), web typings in
`apps/web/src/core/protocol.ts` (marker). **Done when:** web e2e on the demo hub with fake units: plan, run, abort,
result, proposal shown, apply refused for Control and done for Admin; notices for "needs the DSPi hub", "these units do
not share a clock", "unit lost lock", "needs N runs", "this S/PDIF source does not carry a DSPi time base". **Deps:**
AT-C; P5WMS merged.

**ATQT: Qt: measuring several units.** [TODO] Size 1.5. Wave AT-D. **Owns:** a new Qt controller and view for the group
step beside P5QUI's measurement pages (registrar line, marker), the S/PDIF transport in Qt's system editor. **Done when:**
Qt tests on the demo hub as ATWEB's; the same notices. **Deps:** AT-C; P5QUI merged.

**ATDOC: Docs.** [TODO] Size 1. Wave AT-D. **Owns:** new `docs/integrator/multi-unit-measurement.md` (topologies that
align: line, USB-direct mode 2 on one controller, S/PDIF between DSPi units; what does not; the hub requirement; leads;
runs and the anchor), the user manual's measurement chapter section, `docs/PARITY.md` rows, this plan's `[DONE]` marks.
**Done when:** links resolve; every notice key of ATWEB and ATQT appears; the topology table matches section 1.1.
**Deps:** AT-C (writes alongside ATWEB and ATQT; disjoint files).

One Fable review for AT-D (0.75).

### HIL

**ATHIL: Real boards (coordinator and owner).** [TODO] Size 2.25. Not an agent WP. **Part A** (1.25, after FW-C is
reviewed and merged on `bofr-main`; owner's go to flash these images, OD-ATq): electrical only, no speakers, generator
level -20 dBFS, capture into a stereo line input, no host playback (test-audio rule), K340 at 1.20 V checked before and
after (read back, never written); tools: `tools/alink/skew_xcorr.py`, FWTB3's `siggen_sync_bench.py`.
- **H1** USB-direct two-unit skew, CLICK at one T (K340 + a Pico 2 on one controller): mean within +-2 us, spread under
  0.1 sample, the same after three power cycles of each. The number the whole scheme rests on; never measured yet.
- **HS1** S/PDIF durability (a Pico 2 source, the K340 or a Pico 2 follower, a direct wire or TOSLINK): CLICK at one F on
  the source and the follower; the offset equals `D + constant` and does not change (within 0.1 sample) across five
  follower power cycles, three source restarts (new epoch), three cable pulls (holdover and relock), at 48 and 96 kHz.
  The property S/PDIF lacked.
- **HS2** two S/PDIF followers of one source (needs a third board): skew mean within +-2 us, spread under 0.1 sample.
- **H2** generator at one T on two units (op 9): the two sweeps' offset equals H1's (or HS1's) CLICK skew.
- **H3** generator against programme on one unit: a programme sweep and the generator's sweep started at the matching
  T differ by zero samples at the output.
- **H4** loss during a run (USB cable at a hub, S/PDIF cable pull): events arrive, the generator is silent, the timeline
  shows it, pending starts are cancelled.
- **H6** phase error under load on the K340, 120 s with the pipeline busy, against the 10 us gate.
- **H7** the arm lead on the K340: op 9 with leads of 100, 150 and 250 ms: no first-cycle attenuation at 250 ms, a clean
  refusal when too short.
- **H5** (line, fan-out) when the owner has wired the alink Picos: zero whole-sample offset behind a fan-out after
  `hops_offset`; a deliberately wrong offset reads as that many samples.

**Part B** (1.0, after AT-D and P5HIL): one system measurement with the web or Qt client through our hub on two units
(USB-direct, then S/PDIF), the two outputs summed passively into one microphone-interface input: the recovered
inter-unit delay equals Part A's skew plus the configured D difference within 5 us; a cable pulled mid-run aborts and
names the unit; the proposal is shown; an Admin applies it and the summed check passes; Revert restores the model and
the units. Results to `work/athil/RESULTS.md`; then `bofr-main` is pushed to the Bofr5555 fork (memory `push-forks`).

### Waves

No two WPs in one wave own the same file, and no WP depends on another WP of its own wave. CMake lines, dispatcher rows,
`mod` lines, bindings, `notices.json` blocks and the RAM baseline are additive (the RAM baseline is the coordinator's at
the merge). Firmware and console waves run in parallel; the console needs firmware only through FWTB0's spec.

| Wave | WPs | Size | Needs |
|---|---|---|---|
| FW-A | FWTB0 1.25 (+ review 0.25) | 1.5 | this plan |
| FW-B | FWTB1 4.5, FWSP1 2 | 6.5 | FW-A |
| FW-C | FWSP2 4.5, FWTB3 1.25 (+ review of FW-B and FW-C 1.0) | 6.75 | FW-B |
| AT-A | ATSYS 3, ATSIM 2 | 5 | the seam; P5-C2 merged (ATSYS) |
| AT-B | ATPROTO 2, ATFAKE 2.5 (+ review of AT-A and AT-B 0.75) | 5.25 | FW-A reviewed; PG-B2 merged |
| AT-C | ATHUB 4, ATHUBM 2, ATCORE 5 (+ review 0.75; full gate) | 11.75 | AT-A, AT-B; P5-C2 |
| AT-D | ATE2E 1.5, ATWEB 1.5, ATQT 1.5, ATDOC 1 (+ review 0.75) | 6.25 | AT-C; P5-D (P5WMS, P5QUI) for the clients |
| — | **ATHIL** Part A (1.25) after FW-C; Part B (1.0) after AT-D and P5HIL | (2.25) | owner's go and wiring |
| | **Firmware: 5 WPs** | **13.5** | contract and ops 5.75, S/PDIF 6.5, simulation 1.25 |
| | **App: 11 WPs** | **26** | maths 5, wire and fakes 4.5, hub 6, core 5, end to end 1.5, clients 3, docs 1 |
| | **Reviews 3.5, HIL 2.25** | **45.25 in all** | |

If fewer agents run, FWTB1 and ATSYS are the long poles; AT-C's ATCORE is the longest console WP. The single-unit P5
waves are not blocked by this plan; AT-D's clients follow P5-D, and Part B follows P5HIL so a single unit is proven
before several are.

---

## 3. Testable without hardware

| What | Where shown | WP |
|---|---|---|
| op 9 lands on its sample (split block, silence, audio), never late, cancels | firmware host tests | FWTB1 |
| the contract's states and disturbance counter per transport | `test_timebase.c`, `test_alink_spdif.c` | FWTB1, FWSP2 |
| the S/PDIF carriage bit-exact, C against Python | `alink_spdif_vectors.json` | FWSP1 |
| S/PDIF follower skew and durability across relock, epoch, re-anchor | `tools/alink_spdif_sim` | FWSP2 |
| two/four units' generators coincide on line, fan-out and USB | `tools/alink_phase_sim`, `alink_usb_sim` | FWTB3 |
| single-unit results unchanged through the system analyser | goldens, P5ARRIVAL replays | ATSYS |
| inter-unit delays within 2 us; one-sample START error; two domains refused; anchor chains; moved microphone | synthetic captures | ATSYS |
| a world with every fault and its ground truth | `sim::system` | ATSIM |
| both envelopes for every new id; refusals local on stock | G1b, `Bofr117`, G1s, G2s fakes | ATPROTO, ATFAKE |
| hub scheduling, USB origin mapping, queue depth, dead-man and every abort | in-process hub + fakes | ATHUB |
| S/PDIF in the model and the align writes; measured arrivals | hub unit tests | ATHUBM |
| per-unit safety, gate facts, Control vs Admin, apply and Revert | scripted hub stub | ATCORE |
| the study's 4.3 list end to end, on both generations | real hub + fakes + sim | ATE2E |

What no fake proves: the real two-unit skew (H1, HS2), S/PDIF latency durability on real receivers (HS1), the K340's
arm lead (H7), the phase error under load (H6), and the line on hardware (H5). ATHIL does.

---

## 4. Decisions

Every decision carries a firm recommendation; following the owner's rule of 2026-11-15 they are taken as recommended
unless the owner objects.

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-ATa** | **The contract's control plane** | (1) the existing per-transport reads plus a generic `EXT_TIME_BASE` (C4, C5) and one event 0xE4 for every transport; (2) per-transport reads only | **(1)** | The core and the hub talk to "a unit with a time base"; a new transport adds a provider, not host code; the disturbance counter is the only proof that survives event merging. 0xE1/0xE3 stay, so nothing breaks |
| **OD-ATb** | **Time base ids** | (1) 3 = S/PDIF now; reserve 4 ADAT, 5 I2S/TDM, 6 PTP; (2) the study's 3 ADAT, 4 S/PDIF | **(1)** | S/PDIF is built first; ids are cheap to reserve and never reused |
| **OD-ATc** | **How T crosses S/PDIF** | (1) the line's superframe in the user bits, one 48-byte block per 192 frames, class 0x10, F mod 192 = 0; (2) channel-status bits; (3) a counter in the audio LSBs | **(1)** | User bits are free between DSPi units (consumer gear uses channel status, LG Sound Sync reads it); one codec layout shared with the line; the 192-frame block is the channel-status period both ends already track. (2) collides with real channel status; (3) corrupts audio |
| **OD-ATd** | **What an S/PDIF source counts** | (1) its own output frames, a new epoch at every output restart; (2) relay its upstream T (line or USB) | **(1) now, (2) later** | (1) is self-contained and testable with two Picos; (2) joins domains and needs the source's `t - output_frames` mapping and re-anchor rules: worth it once (1) holds on hardware (Later) |
| **OD-ATe** | **Op 9's timing rules** | (1) arm at `T - arm_lead`, refuse a short lead at the write, never late, a loss/config write/host cancels, plain START stamped; (2) arm at accept; (3) late START runs at once | **(1)** | A queued op does not hold the DAC open for nothing; a wrong timetable is worse than none; the cancels keep "what starts at T" equal to what the host scheduled |
| **OD-ATf** | **Delivery of starts and stops** | (1) starts per unit (`EXT_APPLY_AT_FRAME`), broadcast only for `SIGGEN_STOP` now; (2) broadcast starts too | **(1)** | Segments start at different T per unit and the superframe carries one op at a time; the broadcast stop is the abort without control latency |
| **OD-ATg** | **What one group may span** | (1) one time-base numbering (one domain): one sender's line units, one controller's USB-direct units, or one S/PDIF source with its followers; USB origins mapped by the hub; (2) several domains with a slope each | **(1)** | Two domains have two clocks and two numberings: no common T to schedule on. The analyser carries a domain per segment so (2) can come later; the plan step refuses it now |
| **OD-ATh** | **Where the units' side runs** | (1) our hub, a `system.measure.*` family; native and web alike; (2) a hubless path in the core for native clients | **(1)** | The web cannot hold several device cores; the hub already has the model, the scenes, the alarms and the per-unit judges; the hub's dead-man keeps a run finite if a client vanishes. Several USB-direct units on one PC run our hub there |
| **OD-ATi** | **What the gate trusts** | (1) counters and read-back (`disturbances`, `t - output_frames`, the timeline), events for promptness only; (2) events | **(1)** | Events are rate-limited and collapse (250 ms); a counter read before and after cannot miss one |
| **OD-ATj** | **Passes and configs** | (1) interleaved passes, one config per unit staged once, each segment `repeat` 1; (2) one long walk per unit | **(1)** | `SET_CONFIG` while running restarts the generator, so configs cannot be restaged between passes; interleaving spreads microphone wander over every unit |
| **OD-ATk** | **Runs longer than one capture** | (1) an anchor output in every run, intercepts relative to it, a consistency check; (2) one run only | **(1)** | S7 (120 s) and 8 Mi frames cap a run; a cinema or a large room needs several |
| **OD-ATl** | **Where the alignment lands** | (1) the hub's model (`measured_arrival_us`), written by `system.align`; Admin applies; delays only across units; (2) direct `out.delay` writes from the client | **(1)** | One writer of `d_local`, `hops_offset` and `out.delay` for system units; lip-sync and latency terms stay consistent; polarity and crossovers across units need a cross-unit tune (Later) |
| **OD-ATm** | **S/PDIF time base on RP2040** | (1) RP2350 only now; (2) both | **(1)** | RP2040's RAM is spent; ops 9/10 on its USB time base still come (flash only) |
| **OD-ATn** | **The default lead** | (1) `max(arm_lead) + 100 ms` plus the measured write time, 250 ms with a K340; (2) the scenes' 2400 frames | **(1)** | 50 ms is shorter than the K340's DAC un-mute; H7 confirms 250 ms |
| **OD-ATo** | **Which firmware line gets it** | (1) `bofr-main` (1.1.6 line) now; the port (PLAN-PG FWPG1-6) carries it; the tunnel's golden rows added by whichever lands later; (2) wait for the port | **(1)** | The port waits for upstream's tag; the ids are envelope-independent by the spec (1.3) |
| **OD-ATp** | **S/PDIF defaults** | (1) mode 0 by default; in mode 1 a missing superframe is `LOCK_ERROR` with `on_lock_error` mute by default; (2) mode 1 by default | **(1)** | A TV on S/PDIF must never be muted by a setting it cannot satisfy; inside a system, mode 1 failing must be visible (owner 2026-11-14 for USB-direct) |
| **OD-ATq** | **Flashing the HIL images** | (1) the owner's go for these images (S/PDIF roles and ops are new code, outside the standing go for alink-usb test images); (2) the standing go | **(1)** | The standing go covers reviewed alink-usb test images within agreed parameters only |

---

## 5. What not to do

- Start a generator late, or let a START that missed its sample run anyway; arm the DAC for an op that is not due.
- Restage a `SiggenConfig` between passes (it restarts a running generator).
- Measure across two domains, or across units whose T cannot be compared in one read.
- Claim a time base on foreign S/PDIF, ADAT, I2S, USB 44.1 kHz or USB mode 0/1; measure across them (single unit only).
- Put a source's S/PDIF time-base slot in a generator mask.
- Gate on events alone; propose anything from a run that failed G1-G4.
- Write `out.delay`, `d_local` or `hops_offset` of a system unit from the client; apply a system alignment without
  Admin, a filed revision and snapshots.
- Play test audio from a host; raise any volume; measure without the per-unit S1-S13.
- Touch the K340's core voltage; flash any HIL image without the owner's go; flash stock firmware on a K340.
- Put the U-bit collector, the stamping or the servo's new code in RAM without a measured need (64-byte budget).
- Send a new id to a unit whose `EXT_LIST` does not name it; send the tunnel to a 1.1.6-generation unit.
- Push anything from a WP; open a pull request to Weeb Labs.

---

## 6. Owner questions and actions

**No blocking question.** Every decision in section 4 has a firm recommendation.

**Owner actions:**
1. **A go to flash** the FW-B/FW-C images on the K340 and the Pico 2 boards for ATHIL Part A (OD-ATq).
2. **Boards and wiring:** a Pico 2 as S/PDIF source (the spare Pico 2 PLAN-PG asks for can serve both), a third board for
   HS2 if available; an S/PDIF connection between them (a direct GPIO wire with common ground is enough; TOSLINK modules
   if preferred); access to the K340's S/PDIF input pin (GP5, the default RX1) if the K340 is to be a follower, else two
   Pico 2 boards; the stereo line input (e.g. the Xonar U7) for the electrical captures.
3. **When the alink Picos are wired:** H5 (line and fan-out).

---

## 7. Later (not in this plan's waves)

| Item | Size | Why later |
|---|---|---|
| S/PDIF relay (mode 3): a follower or USB-direct unit re-emits its upstream T on S/PDIF, joining domains (OD-ATd) | 2 | after HS1 holds |
| ADAT time base (4): a counter in the user bits over a multi-frame block, source and follower | about 5 | owner: maybe later |
| I2S/TDM time base (5): a counter word on a spare slot or a start pulse | about 5 | not planned (SYS1) |
| PTP / AES67 media clock (6) on another MCU | long-term | SYS1 step 6 |
| Several domains in one group (a slope each, a bridge measurement between them) | 3 | needs a bridge (relay or a reference) |
| Cross-unit polarity, crossover corner and EQ proposals (a system tune) | 4 | after the delay alignment is proven (Part B) |
| SNDGEN: the sender generates a sweep on a slot mask, a summed whole-path check of the programme path (study 2 (b)) | 2.5 | owner: only if this does not work out (H3 is the test) |
| S/PDIF time base on RP2040 | 1.5 | RAM (OD-ATm) |
| Terminal (`uconsole measure system`, `uconsole timebase`), Android, Swift screens | 1, 2.5, 3 | owner's order: web and Qt first |
| A hubless multi-unit path for native clients | 2 | OD-ATh; only if an installer cannot run our hub |
