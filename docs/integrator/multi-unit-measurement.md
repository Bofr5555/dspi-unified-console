# Measuring several units together

*WP ATDOC, 2026-11-19: the integrator's page for PLAN-AT ([`docs/core-merge/PLAN-AT.md`](../core-merge/PLAN-AT.md)),
"the alink time-base contract and multi-unit measurement". State: the firmware side is on the fork's `bofr-main`
(`4f7114c`: waves FW-A to FW-C, reviewed); the maths, the simulated world, the codecs, the fakes, the hub family, the
hub's model and the client's session are on `main` (waves AT-A to AT-D part 1, reviewed: `11362bb3`, `915d1a65`,
`1c5ac8ec`). **The web and Qt screens are in progress** (WP ATWEB, WP ATQT). **Nothing of it has run on real boards
yet** (ATHIL, [section 11](#11-what-the-hardware-must-still-confirm)). The single-unit measurement this builds on is
[measurement.md](measurement.md); the user's page is [docs/manual/measurement.md section 10](../manual/measurement.md#10-measuring-several-units-together).*

Where this page and the code disagree, the code is right and the page is the bug.

## Contents

| § | Covers |
|---|---|
| 1 | [What it does](#1-what-it-does) |
| 2 | [Topologies that align, and those that do not](#2-topologies-that-align-and-those-that-do-not) |
| 3 | [The hub requirement and the roles](#3-the-hub-requirement-and-the-roles) |
| 4 | [Before a run: fresh checks and the settle wait](#4-before-a-run-fresh-checks-and-the-settle-wait) |
| 5 | [Leads, passes and levels](#5-leads-passes-and-levels) |
| 6 | [Runs and the anchor](#6-runs-and-the-anchor) |
| 7 | [What the timing can deliver](#7-what-the-timing-can-deliver) |
| 8 | [The gate and the aborts](#8-the-gate-and-the-aborts) |
| 9 | [The summed check, apply, verify, Keep and Revert](#9-the-summed-check-apply-verify-keep-and-revert) |
| 10 | [Notices and the clients](#10-notices-and-the-clients) |
| 11 | [What the hardware must still confirm](#11-what-the-hardware-must-still-confirm) |
| 12 | [Not done yet](#12-not-done-yet) |

---

## 1. What it does

One measurement microphone, several DSPi units, one alignment. Every unit plays **its own generator** (bounded at the
unit, per output, exactly as a single-unit measurement), and every start is scheduled on the installation's **shared
sample counter T**: the hub writes each unit an apply-at-T op 9 `SIGGEN_START`, the firmware arms the DAC ahead of it
and starts the generator's sample 0 on exactly the sample carried as T, or refuses it (a late START never plays). The
client's core reads one timeline across the units: one clock slope for the group, one intercept per (unit, output).

Two promises, kept apart (OD-ATr):

- **The timeline is sample-exact.** Each unit proves its start afterwards (`EXT_SIGGEN_TIMELINE`: run id, the T of
  sample 0, no discontinuity, the samples played).
- **The measured delay between units is not.** Two units never play in one sweep, so the delay between them inherits
  the capture clock's wander between their sweeps. Every cross-unit delay carries an uncertainty and is **offered only
  when that uncertainty is at most 10 µs**; otherwise the result says "timing unreliable" with the wander
  ([section 7](#7-what-the-timing-can-deliver)).

What is offered is written into **the hub's system model**, never directly to the units: each measured speaker gets
`measuredArrivalUs` and the hub's own alignment engine (`system.align`) turns it into output delays
([audio-link.md, measured arrivals](audio-link.md#spdif-units-and-measured-arrivals-wp-athubm)). Applying needs Admin.

The measurement never names a transport: the core and the hub talk to "a unit with a time base". The contract a
transport has to meet is the firmware's `alink_protocol_spec.md` "Time bases" section (C1 to C6: the counter, fixed
play-out, apply at T, lock states with a disturbance counter, a domain identity, discovery).

## 2. Topologies that align, and those that do not

The transports, as PLAN-AT section 1.1 states them (the last column is this page's):

| Transport | T | How T reaches a follower | Broadcast op path | C1-C6 | Several units measured together |
|---|---|---|---|---|---|
| line L1-L3, fan-out, repeater | F | superframe + head byte; re-emitted with `hops` | superframe (sender `EXT_ALINK_CTRL_MSG`) | exist; C4's counter and 0xE4, C6 added (FWTB1) | **yes**: the units behind one sender |
| USB-direct mode 2, 48/96 kHz | S | SOF count + host rate | none; time base 2 gives the same bytes per unit | exist; C4, C6 added (FWTB1) | **yes**: the units on one USB controller (one bus) |
| **S/PDIF between DSPi units** | F | the S/PDIF superframe in the user bits, one per block | the same superframe (source `EXT_ALINK_SPDIF_CTRL`) | **new** (FWSP1, FWSP2) | **yes**: one S/PDIF source and its followers |
| USB 44.1 kHz, USB mode 0/1, foreign S/PDIF, ADAT, I2S slave | - | - | - | `NONE`: single-unit measurement only | **no**: each unit on its own |

**One group is one domain** (OD-ATg, gate G1): the units of one measurement must share one T numbering. That is

- **the units behind one audio link sender**, fan-outs and repeaters included (a unit behind a fan-out lands on the same
  sample after its `hops_offset`);
- **the USB-direct units on one USB controller**, in mode 2 (phase lock) at 48 or 96 kHz. Their counters start from
  different origins, multiples of `2048 * spf` apart, which the hub resolves at `open`. The hub must run on the machine
  they are plugged into, since only it sees their bus;
- **one S/PDIF source and its followers**, DSPi units only. The source is a member of its own domain.

**What does not align as one group:** units behind two senders, USB-direct units on two controllers, a line group plus a
USB group, two S/PDIF sources, any mix of those (two clocks, two numberings, no common T: the hub answers
`not_one_domain`, the notice "These units do not share a clock"). Also any unit whose transport has no time base: foreign
S/PDIF (a TV, a receiver, a DSP that rewrites the user bits), USB at 44.1 kHz or in mode 0/1, ADAT, I2S. Those are
measured one unit at a time (single-unit measurement, unchanged). Several domains in one group are in PLAN-AT's
"Later".

Notes on S/PDIF:

- A follower in S/PDIF mode 1 that finds no DSPi time base on its input (a foreign source, or something that re-clocks
  or rewrites the user bits) reports lock error and, by default, mutes. A direct wire, coax with a transformer and
  TOSLINK with passive splitters carry the time base.
- **A source's own input.** With USB mode 0, every stream start restarts the source's outputs: a new epoch, and every
  follower re-anchors at every "play". A source should run USB-direct mode 2 or a non-USB input (the hub's issue
  `spdif_source_epochs`).
- **The source's time-base slots are never measured outputs** (S-AT6): the stimulus would reach every follower as
  programme. The hub's model refuses a speaker on them and `stage` refuses a generator mask that includes them.
- A unit that is a USB-direct follower and an S/PDIF source at once lists two time bases; it takes part by the entry the
  rest of the group shares (R7b).

## 3. The hub requirement and the roles

**Multi-unit measurement runs through our hub** (OD-ATh). The client's core owns the microphone, the plan, the analysis,
the per-unit safety judgement and the documents; the hub owns the units' side: it reads their time bases, stages their
configs, schedules every start, watches every unit and aborts on its own (the dead-man). The messages are
[hub-api.md section 23](hub-api.md#23-measuring-several-units) (`system.measure.open`, `stage`, `run`, `status`, `abort`,
`close`, the event `system.measure.aborted`), served to DSPi Link 1.3 clients under the cap `system_measure`.

A client on a hub without the cap (a Weeb Labs hub, an older hub of ours) or with no hub at all offers single-unit
measurement only, with the notice `measure.system.needs_hub`. The web cannot hold several device cores, so there is no
hubless path; several USB-direct units on one machine run our hub on that machine.

| Role | May |
|---|---|
| Viewer | nothing of the family (refused) |
| Control | measure (open, stage, run, status), see the result and the proposal, save the documents |
| Admin | all of Control, and **apply** and **Revert** (they write the hub's model: `system.set`, `system.align`) |
| any Control or Admin principal | abort a session (stopping test audio is never a privilege) |

**Applying is Admin-only.** A Control user who presses Apply is refused before anything is sent
(`measure.system.admin`).

## 4. Before a run: fresh checks and the settle wait

**Fresh unit checks.** The per-unit safety rules of the single-unit measurement (S1 to S13: volume at -12 dB or lower,
tweeter protection, silent inputs, no unmodelled processing, the run's bounds) are judged by **each unit's own core** on
its own connection: `measure_system_unit_check` on that unit's `DspiCore`. The core on the hub session takes those
checks and refuses with the unit named when one has a blocker (`measure.system.unit_check`). A client gets a fresh check
from every unit **right before each of these calls**, never reuses an old one:

- `measure_system_start` (the measurement);
- `measure_system_sum_check` (the summed check, [section 9](#9-the-summed-check-apply-verify-keep-and-revert));
- `measure_system_verify` (the verify after an apply).

Between those sessions no lock is held and the hub watches nothing, so a unit turned up or a high-pass removed in between
is caught only by the fresh check (review AT-C, F2). The summed check and the verify also ramp each output again.

**The settle wait.** The phase servos keep settling after the first lock (about 40 s to their settled skew; the FW-B/FW-C
review). The hub therefore places a run's first start **at least 40 s after it first saw each unit locked** with its
current disturbance count. A hub that has watched its units for longer (its system poll feeds the same clock) adds
nothing; the first run after a hub start, a re-anchor or any other disturbance waits **up to 40 s**. The client keeps
polling `status` meanwhile and says it in words: `MeasureSystemState.words` is "waiting for the units' clocks to
settle".

**At `open`** (gate G1): every unit lists the group's time base exactly once, `LOCKED`; the units' T agree within the
reads' round trips; USB-direct units share one bus where the platform says so; one rate (44.1, 48 or 96 kHz) read from
two reads per unit at least 0.1 s apart. When a unit's reads took too long for the hub to time them, it reads again over
a longer span (up to 1.6 s) and then refuses with `device` and the unit named ("a busy hub; try again"), never with a
guessed rate.

## 5. Leads, passes and levels

**The lead** (OD-ATn, S-AT3) is the time between the newest T the hub read and the first start:
`max(lead_ms, max(arm_lead) + 100 ms + the hub's measured write time)`. A unit's arm lead is its
`EXT_SIGGEN_TIMELINE.arm_lead_ms` (`max(hold_ms, release_ms) + 50 ms`: 150 ms on the K340), so **250 ms with a K340** in
the group. The firmware refuses an op 9 with a shorter lead at the write.

**The schedule.** At most one op 9 pending per unit (R6a): the next segment's START is written once the previous one has
started, never more than 110 s ahead (the firmware refuses beyond 120 s), never less than the arm lead plus 25 ms (then
the run is aborted: a late START never plays). A unit's segments are at least the lead apart (else `bad_schedule`). If
the hub dies, each unit plays at most one more finite segment.

**Passes** (OD-ATj): pass 1 is unit A's walk over its outputs, then unit B's, and so on; pass 2 again. **5 passes by
default when the run fits one capture, else 3**; 2 to 8 when set. One config per unit per run, staged once (a config
write restarts a running generator). Fewer than 3 passes are always timing unreliable.

**Levels.** Each unit plays at the minimum of its walked outputs' ramp results (one config per unit). When units play
together (a summed run):

- **the client** plays each output 6 dB under the lower of the two fresh ramp results, never above the measurement's
  (S-AT4);
- **the hub** holds what no ramp can pass: units that play together are each staged **at most 6 dB per doubling of their
  number under the hard cap** (-6 dBFS for speakers), else `bad_schedule`:

| Units playing together | Highest staged level |
|---|---|
| 2 | -12 dBFS |
| 3 or 4 | -18 dBFS |
| 5 to 8 | -24 dBFS |
| 9 to 16 | -30 dBFS |

It counts units, not outputs. Every config is also judged alone at `stage`: a finite log sweep of 1 to 15 s, 1 to 8
repeats, never `RAW`, never pink noise (continuous to op 9), at most the hard cap, only the unit's own outputs, never an
S/PDIF source's time-base slot.

## 6. Runs and the anchor

**One run** holds at most **16 outputs** across the group and lasts at most **120 s** and 8 Mi capture frames (S7). A
group of up to 16 units is opened at once.

**A larger group is split into runs** that each repeat one **anchor output** (OD-ATk). Intercepts are kept relative to
the anchor in each run. The anchor is user-selectable (`MeasureSystemSetup.anchor_serial`, `anchor_output`); by default
it is the first measured output of the group's first unit. Choose a full-range output with a short latency when there is
one.

**Chaining is checked.** The anchor's responses must agree across the runs: magnitude spread at most **1 dB** over
200 Hz to 5 kHz (or over the anchor's own passband when it is not full range), and with 3 or more runs its arrival within
**20 µs** of the runs' drift fit. Otherwise "the microphone or the room changed between runs" and nothing is proposed
(`measure.system.chain`). The microphone stays open across the chained runs (one capture clock; a gap in the capture
aborts), so the arrival check can run. Two outputs offered against the anchor are offered as a pair only when their
combined uncertainty (in quadrature) is at most 10 µs.

**Keep the microphone still** for every run of one alignment.

## 7. What the timing can deliver

**The uncertainty.** The analyser gives each (unit, output) intercept an uncertainty (the standard error of its per-pass
intercepts about the fit, never below the run's pooled scatter) and each cross-unit difference its own. A cross-unit
delay is offered only when that is at most 10 µs **and** the run passed the single-unit timing gate (every cycle within
10 µs of the clock fit, the repeats' phases agreeing). Otherwise: "timing unreliable: the capture clock wandered N µs
between A and B" (`measure.system.timing_unreliable`), and nothing is proposed for that pair. The documents never promise
a sample-exact measured delay.

**With a USB microphone whose clock wanders like the one measured in P5SPIKE** (bursts of 55-100 ppm for 0.25 to
1.5 s, within ±50 µs), **every run reads "timing unreliable"**: the per-cycle gate already refuses it, so nothing is
offered. That is safe, and it is what to expect. The simulated table (review AT-D part 1, re-running ATE2E's study: two
units, one output each, 1 s sweeps, a room, a 31 ppm microphone, 30 seeds per row; P5SPIKE's bursts scaled by k, their
bound with them):

| k | 3 passes: gate pass, offered error | 5 passes |
|---|---|---|
| 1, 0.5, 0.3 | 0/30 | 0/30 |
| 0.2 (11-20 ppm, ±10 µs) | 1/30 (its error 1.46 µs) | 0/30 |
| 0.1 (5.5-10 ppm, ±5 µs) | 22/30, RMS 2.08 µs, worst 4.85 µs | 11/30, RMS 2.00, worst 2.99 |
| 0.05 | 30/30, RMS 1.03, worst 2.37 | 30/30, RMS 0.82, worst 1.67 |
| 0.02 | 30/30, RMS 0.40, worst 0.90 | 30/30, RMS 0.33, worst 0.63 |
| none | 30/30, RMS 0.07 | 30/30, RMS 0.07 |

So runs pass only when the wander is about a tenth of P5SPIKE's or less; more passes mean more chances for a burst.
Everything offered stayed within three times its uncertainty and within 10 µs of the truth. A capture path with a steadier
clock (an analogue measurement microphone on an audio interface) is the remedy today; "overlapped starts" (two units'
sweeps in one deconvolution, so the wander is common to both) is in PLAN-AT's "Later", to be pulled forward if the
owner's microphone trips the gate on hardware.

**The usable timeline spread is about 85 ms.** The analysis keeps each response from 100 ms before the run's first
arrival to 100 ms after (its search span, the same whatever gap is set), less the response's own window. An output heard
more than about 85 ms after the run's first output (its play-out plus the output delay in effect) is cut off: it is "heard
too long after the run's first output for the gap between sweeps", and **nothing of that group is offered**, since the
cut output may be the reference. 80 ms apart is still exact. This is reachable on USB-direct, where the engine puts the
streams' start offsets (about 21 ms per unit on the fakes) into the output delays: five or more units, or a large output
delay. The hub's own bound for the parts of a summed run (`MAX_SUM_SPREAD_MS`, 1 s) is far wider; the 85 ms limit is the
analysis'. The real start offsets on hardware are not known yet (ATHIL notes them).

## 8. The gate and the aborts

A run is discarded, with the unit named and nothing proposed, when any of these fails (G1 to G4; the counters and the
read-back are the proof, events only make it prompt, OD-ATi):

- **G1 one domain**, at `open` and at the end (section 4).
- **G2 nothing moved**: per unit, `LOCKED` at open and at the end, its `disturbances` counter unchanged, `t -
  output_frames` unchanged within a sample.
- **G3 the proof**: per segment, `EXT_SIGGEN_TIMELINE` shows the expected run id, `start` on the scheduled T, started,
  no discontinuity, the segment's length. A START one sample off discards the run.
- **G4 no event**: no lock transition (0xE1, 0xE3, 0xE4), abort or foreign stop on any unit.
- **G5** the single-unit timing gate and the 10 µs rule of section 7 (these mark the result timing unreliable rather than
  discard it).

**The hub aborts by itself** and tells the session's owner (`system.measure.aborted`): no `status` within the lease
(500 to 5000 ms, default 2000), a unit's time base disturbed, a clip event or an output peak at -0.5 dBFS, a generator
stopped by someone else, a preset load, a parameter or volume change not the session's own, a START refused or late, a
unit offline, a device lock lost, the session's connection closing. Abort is `STOP_NOW` and `CANCEL_ALL` on every unit,
plus a broadcast stop where there is a path and every unit has the ops; after it nothing staged can start.

**The client aborts** too, naming the unit: a clip flag, an output peak, a limiter acting, a disturbance, a lock
transition, a refused or cancelled START, a frame gap or clip in the capture, the run's overrun, **Stop**. If the hub
cannot be reached it sends `STOP_NOW` through any unit connection it has.

## 9. The summed check, apply, verify, Keep and Revert

**The summed check** (RC5f across units). Before a cross-unit delay can be applied, every **inter-unit transition** (two
outputs on different units whose responses overlap by at least an octave within 10 dB of their passbands) is played
together once and compared with the sum the walked run predicts: within **1.5 dB RMS** over the transition's band (capped
above at `1 / (8 u)` for the delay's uncertainty u), else **every** cross-unit delay is withdrawn
(`measure.system.sum_check_failed`). Until it has passed, the proposal says `measure.system.sum_check_needed` and apply is
refused.

- It needs **fresh unit checks** (section 4), re-ramps each output and plays 6 dB under the lower result (section 5).
- **The parts start apart** (ATSUMOFF): the part predicted to arrive earlier starts later by the difference, in whole
  samples, so both are heard together. T is commanded and proven, so the offset is exact. A wrong delay of 30 µs or more
  either way fails at every spread the walked run can measure; within about a sample (±10 µs) the magnitude cannot tell.
- **A pair that would cancel is not played**: outputs whose predicted sum lies 6 dB or more under the louder part over
  more than half of the band are refused before anything plays, naming both outputs and the polarity.
- An older hub without the `summed` run refuses the staggered starts (`measure.system.hub_refused`): no verdict, apply
  stays refused. Fail closed.

**Apply (Admin only).** The core files the hub model's revision and every measured unit's output delays, writes the
offered arrivals into the model (`system.set`, guarded by the revision: a model changed meanwhile is `conflict`), then
`system.align` writes the units. There is **no countdown**: the alignment stays until Keep or Revert.

**Verify.** The summed runs again, with the applied delays predicted, judged up to a quarter of the rate. It needs fresh
unit checks again. A wrong delay of more than a few tens of microseconds does not survive it. A failed verify says
`measure.system.verify_failed`.

**Keep takes two steps after a failed verify.** The first `measure_system_keep` is refused with the reason in
`applied.message` ("the verify did not confirm the alignment; ..."); the client shows it and asks again, and only the
person's **second press, at least 1 s later**, keeps it. Nothing presses it for them. A new verify asks again. After a
verify that passed, or before any verify, one Keep keeps.

**Revert (Admin)** sets the filed model again at the revision the apply left, aligns, and reads every unit's output delays
back against the filed ones.

Not proposed across units in this plan: polarity, crossover corners, EQ (each unit's own auto-tune does those within the
unit).

## 10. Notices and the clients

Every notice key the system measurement uses. Rows are in `assets/notices/notices.json`; the core's list is
`NOTICE_KEYS` in `crates/dspi-core/src/measure.rs` (WP ATCORE).

| Key | Title (the person reads) |
|---|---|
| `measure.system.needs_hub` | Measuring several units needs the DSPi hub |
| `measure.system.unit_check` | A unit was not checked |
| `measure.system.rates` | The units run at different rates |
| `measure.system.not_one_domain` | These units do not share a clock |
| `measure.system.not_locked` | A unit's time base is not locked |
| `measure.system.no_timeline` | A unit cannot start on the time base |
| `measure.system.hub_refused` | The hub did not schedule the run |
| `measure.system.plan` | The group does not fit a measurement |
| `measure.system.abort.clip` | A unit clipped |
| `measure.system.abort.output_peak` | An output reached full scale |
| `measure.system.abort.limiter` | A limiter acted |
| `measure.system.abort.disturbance` | Unit lost lock (its time base was disturbed) |
| `measure.system.abort.time_base` | Unit lost lock (its time base changed) |
| `measure.system.abort.generator` | A unit's generator did not play |
| `measure.system.abort.hub` | The hub stopped the measurement |
| `measure.system.abort.disconnected` | The hub does not answer |
| `measure.system.abort.overrun` | The run took too long |
| `measure.system.gate` | The run was discarded |
| `measure.system.timing_unreliable` | Timing unreliable |
| `measure.system.chain` | The runs do not chain |
| `measure.system.sum_check_failed` | The summed check failed |
| `measure.system.sum_check_needed` | Run the summed check |
| `measure.system.admin` | Applying needs Admin |
| `measure.system.nothing_offered` | Nothing to apply |
| `measure.system.verify_failed` | The verify did not confirm the alignment |

**In progress** (WP ATWEB, on its branch, not on `main` yet; ATQT will use the same rows; the coordinator reconciles
this table at their merge):

| Key | Title |
|---|---|
| `measure.system.runs` | The group needs several runs |
| `measure.system.placement` | Choose where the microphone is |
| `dlink.not_one_domain` | These units do not share a clock (the hub's refusal) |
| `dlink.not_locked` | A unit's time base is not locked (the hub's refusal) |
| `dlink.no_timeline` | A unit cannot start on the time base (the hub's refusal) |
| `system.spdif.no_time_base` | This S/PDIF source does not carry a DSPi time base |

Two refusals come as plain errors with words and no key of their own: the summed check's "would cancel" refusal
(section 9) and `open`'s `device` refusal for a unit the hub could not time (section 4).

**The core's calls** (`dspi_core`, UniFFI, the C ABI `dspi_measure_system_*`, wasm): `measure_system_unit_check`,
`_preflight`, `_start`, `_capture_push`, `_tick`, `_state`, `_stop`, `_result`, `_proposal`, `_sum_check`, `_apply`,
`_verify`, `_applied`, `_keep`, `_revert`. One measurement document per unit, with an additive `system` block (group,
time base, domain, anchor, run ids, per output the arrival relative to the anchor, its uncertainty, whether offered).

| Client | State |
|---|---|
| Web | **in progress** (WP ATWEB): a "group across units" step in the Measure screen, the S/PDIF transport in the system editor |
| Qt | **in progress** (WP ATQT, after ATWEB): the same beside the measurement pages |
| Terminal, Android, Swift | later (PLAN-AT "Later": web and Qt first) |

## 11. What the hardware must still confirm

Everything above is proven on fakes, simulations and the firmware's unit tests only. ATHIL (the coordinator and the owner, PLAN-AT
"HIL") must still show on real boards:

- **`EXT_TIME_BASE.d` is what the unit plays by**, on the line (`d_local + hops_offset`), on USB-direct and in both S/PDIF
  roles. The core takes each unit's play-out delay from it (review AT-C, F1); a wrong D moves the proposal and the
  prediction alike, and no fake can show it.
- **Opening real units.** `open`'s rate reads were fixed for reads of a millisecond (review AT-D part 1, F1): before the
  fix two real units would have been refused `not_one_domain`. ATHIL Part A opens two units as its first step.
- **S/PDIF.** The S/PDIF time base is new firmware with five review fixes that only hardware can confirm (review FW-B/FW-C:
  F3, F5 and the firmware-port halves of F1 and F4). First checks: an idle source's op 9 starts on T, a follower in lock
  error stays silent, a queued op is cancelled across a source restart; then HS1 (the latency stays `D + constant` across
  power cycles, source restarts and cable pulls, at 48 and 96 kHz) and HS2 (two followers).
- The two-unit skew (H1, the number the scheme rests on), the generator at one T on two units (H2), the K340's arm lead
  (H7: 250 ms clean, a short lead refused), the phase error under load (H6), and the real USB-direct start offsets against
  the 85 ms spread of section 7.
- Part B: one system measurement with the web or Qt client on two units, a cable pulled mid-run, an Admin apply with the
  summed check, Revert; or, with a wandering microphone, "timing unreliable" with the wander shown.

## 12. Not done yet

- The web and Qt screens (in progress); Terminal, Android and Swift screens (later).
- Overlapped starts (section 7), several domains in one group, an S/PDIF relay (mode 3), ADAT, I2S/TDM and PTP time bases,
  cross-unit polarity, crossover and EQ proposals, the ops on RP2040 (PLAN-AT "Later").
- Scenes at F on the S/PDIF time base (the hub applies S/PDIF units' scenes at once).
- The planner does not yet tell the analysis each output's expected lateness, which would lift the 85 ms spread (review
  AT-D part 1, for the coordinator).

## Reference

| What | Where |
|---|---|
| The plan, decisions, WPs | [`docs/core-merge/PLAN-AT.md`](../core-merge/PLAN-AT.md) |
| The hub's messages | [hub-api.md section 23](hub-api.md#23-measuring-several-units) |
| The hub's model: S/PDIF units, measured arrivals | [audio-link.md](audio-link.md#spdif-units-and-measured-arrivals-wp-athubm) |
| The single-unit measurement | [measurement.md](measurement.md) |
| The wire (time base, timeline, S/PDIF block, ops 9/10) | the fork's `Documentation/Features/alink_protocol_spec.md` and `extension_channel_spec.md` 2.10, 2.11; codecs `crates/dspi-proto/src/ext/timebase.rs` |
| The analyser, the planner, the chain | `crates/dspi-measure/src/system/` |
| The session | `crates/dspi-core/src/measure/system/` |
| The hub family | `crates/dspi-hub/src/system/measure/` |
