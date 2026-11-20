# Measuring several units together

*WP ATDOC, 2026-11-19: the integrator's page for PLAN-AT ([`docs/core-merge/PLAN-AT.md`](../core-merge/PLAN-AT.md)),
"the alink time-base contract and multi-unit measurement". State: the firmware side is on the fork's `bofr-main`
(`4f7114c`: waves FW-A to FW-C, reviewed); the maths, the simulated world, the codecs, the fakes, the hub family, the
hub's model and the client's session are on `main` (waves AT-A to AT-D part 1, reviewed: `11362bb3`, `915d1a65`,
`1c5ac8ec`). The web and Qt screens (WP ATWEB, WP ATQT), the hub's guard of a system apply (WP ATGUARD) and the demo
hub's simulated microphone (WP ATDEMO) are built too (wave AT-D part 2, review AT-D2). **Nothing of it has run on real
boards yet** (ATHIL, [section 11](#11-what-the-hardware-must-still-confirm)). The single-unit measurement this builds on is
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

**Overlapped starts (WP ATOVL, the default where it is safe).** After that first interleaved pass (which fixes cycle
identity exactly as before: the first sweep is heard alone), every further pass starts every unit's walk together, unit
B **heard a fixed 50 to 200 ms after unit A** (C as much after B, and so on), so each unit's j-th output plays with the
others' j-th outputs: a *group*. Each group is deconvolved as one cut holding all its sweeps whole, and the capture
clock's wander, which every unit of a group meets at the same frequency within the offset, is taken out of the fit as
the group's common delay. The passes given (5 or 3 by default, 2 to 8) are the overlapped ones; the first interleaved
pass comes on top. The starts are planned with each unit's expected lateness, so the units are *heard* the offset
apart, and a unit's own segments keep the lead.

The **offset** is the smallest from 50 ms (1 ms steps) at which the analysis can tell the units apart: each unit's
response is kept from 20 ms before the run's timing origin to the offset less 10 ms after it (never more than the
interleaved plan keeps), every other unit's impulse **and its harmonic images** (orders 2 to 9, which a log sweep puts
`T ln k / ln(f2/f1)` early) stay 5 ms clear of that window allowing 15 ms of unplanned spread between the units, and a
group's cut stays inside the gap between sweeps. Where no offset up to 200 ms does, the plan is **interleaved** and says
why. With the 1 s sweeps of the simulations two units overlap at 50 ms; three do not (the third unit's second harmonic,
100 ms early, would land on the first unit's arrival). With the speakers' 5.5 s sweep and 1 s gaps up to about 13 units
overlap. Only the timed walk overlaps; ramps and summed runs are as before.

**When the run cannot tell its units apart** (a unit heard more than 15 ms from where its terms put it, a cycle on the
edge of its 20 ms search, another unit's sound or harmonic image found in a window), the analysis refuses it as "not
separable" and the session **measures the same run again with interleaved passes**, once. What the result reports
(`MeasureSystemResult.plan`): "overlapped starts, 50 ms apart", "interleaved passes (overlapped starts do not fit: ...)"
or "interleaved passes (overlapped starts could not tell the units apart: ...)"; "run 1: ...; run 2: ..." for chained
runs.

**Levels.** Each unit plays at the minimum of its walked outputs' ramp results (one config per unit). When units play
together (a summed run, or overlapped starts):

- **the client** plays each output of a summed run 6 dB under the lower of the two fresh ramp results, never above the
  measurement's (S-AT4); with overlapped starts each unit 6 dB per doubling of the run's units under its own ramp result
  (and under the hard cap less as much), for the whole run;
- **the hub** holds what no ramp can pass: units that play together are each staged **at most 6 dB per doubling of their
  number under the hard cap** (-6 dBFS for speakers), else `bad_schedule` (with overlapped starts: of the most units
  sounding at once):

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

**A larger group is split into runs** that each repeat one **anchor output** (OD-ATk). The runs are sized as the
session will lay them out (interleaved, the longer plan and the fallback) with the lateness the plan expects: each
unit's smallest `out.delay` among its outputs in the run, from its own check, and 10 ms per segment for the play-out the
plan cannot know yet (WP ATOVL, review AT-E). A group that fits one capture only without its lateness is split here,
before anything plays; a run that still does not fit when the engine's terms are read is refused before it plays, as
before. Intercepts are kept relative to
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
1.5 s, within ±50 µs), **interleaved passes read "timing unreliable"**: the per-cycle gate refuses every run. **Overlapped
starts make that wander common to what plays together** (WP ATOVL, owner 2026-11-19): what a group measures (the
differences between its units) is judged on its own, its common delay taken out. For an overlapped run
`timing_reliable` is that gate: every cycle within 10 µs of the fit after its group's common delay, no ambiguous
arrival, and each repeat's phase, and its delay along the sweep, agreeing with the other repeats **against what the rest
of its group heard at the same frequencies** (where the rest is loud enough to say; elsewhere unchanged). Beside it the
run keeps **P5's absolute gate** (every cycle against the plain fit, the wander left in): a difference between outputs
that never played together (two outputs of one unit, or A's first output against B's second) is offered only on that
gate, as with interleaved passes. So with a wandering microphone the units' j-th outputs are aligned to each other, and
outputs of one unit are not offered.

The simulated table (WP ATOVL, re-running ATE2E's study: two units, one output each, 1 s sweeps, a room, a 31 ppm
microphone, 30 seeds per row; P5SPIKE's bursts scaled by k, their bound with them; "reliable" is the run's gate,
"offered" the cross-unit delay, the error against ATSIM's truth):

| k | overlapped, 3 passes: reliable and offered, RMS error, worst | overlapped, 5 passes | interleaved, 3 passes | interleaved, 5 passes |
|---|---|---|---|---|
| 1 (P5SPIKE) | 29/30, 1.01 µs, 2.03 µs | 28/30, 0.86, 2.03 | 0/30 | 0/30 |
| 0.5 | 30/30, 0.53, 1.11 | 30/30, 0.46, 1.10 | 0/30 | 0/30 |
| 0.3 | 30/30, 0.34, 0.71 | 30/30, 0.29, 0.70 | 0/30 | 0/30 |
| 0.1 | 30/30, 0.15, 0.30 | 30/30, 0.14, 0.30 | 22/30, 2.08, 4.85 | 11/30, 2.00, 2.99 |
| none | 30/30, 0.10, 0.10 | 30/30, 0.10, 0.10 | 30/30, 0.07, 0.08 | 30/30, 0.07, 0.08 |

(`ci/run.sh 'cargo test --release -p dspi-measure --features sim --lib wp_atovl_wander_table -- --ignored
--nocapture'`, 410 s; the interleaved rows are ATE2E's table again, unchanged.)

Everything offered stayed within three times its uncertainty and within 10 µs of the truth (the study asserts it for every
seed). Through the real hub the run lands where the wall clock puts it, so the end-to-end case with P5SPIKE's microphone
asserts "reliable and within its uncertainty, or unreliable and nothing offered".

**What overlapped starts cost.** The responses are kept on the shorter window (20 ms before the origin, the offset less
10 ms after: 40 ms at 50 ms), so below about 300 Hz the fit window is shorter than its 15 cycles; and a later unit's
window holds the earlier units' room decay from the offset on, alike in every overlapped pass. Timing is not affected
(the summed check runs as before and judges the real sum), but a later unit's low-frequency response in the documents
carries some of the earlier unit's room: ATHIL should compare one with an interleaved run. Each unit plays 6 dB per
doubling of the units lower. The analysis keeps every cut's group whole, so its FFTs are as long as the group's span
longer.

**Each unit's expected lateness is planned for (WP ATLATE).** The analysis keeps each response from 100 ms before
where it looks for the run's first arrival to 100 ms after (its search span, the same whatever gap is set), less the
response's own window. The planner reads the engine's terms before the walked run (each unit's play-out of a domain
sample and its outputs' `out.delay`, which on USB-direct hold the streams' start offsets, about 21 ms per unit on the
fakes) and tells the analysis each unit's expected lateness (`interleave_late`, `SystemSchedule::with_lateness`): a
segment after a unit heard later starts that much later again, so no cycle is heard closer than a sweep and its gap to
the next, and the analysis looks for each unit's cycles that much later. Units heard up to 1 s apart
(`MAX_LATENESS_MS`) are measured whole (150 ms apart: within 0.1 us in simulation); more than that is a model or
alignment fault and the run is refused before it plays, the unit named. While `system.status` is rate limited the run
waits up to 5 s for the terms; a unit whose terms cannot be read is expected at 0 and its arrivals are not offered anyway.
What the planner cannot know (a speaker tens of metres away, a wrong term) still meets the old limit: an output heard more
than about 85 ms after where it was expected is cut off, "heard too long after the run's first output for the gap between
sweeps", and **nothing of that group is offered**. The hub's own bound for the parts of a summed run
(`MAX_SUM_SPREAD_MS`, 1 s) is the same size. The analysis looks for a run's first cycle, and judges the units' status
polls, the first unit's lateness later (for a summed run: as much later as its last part starts), so the whole second is
usable whichever unit is the late one (review AT-E; before, a first unit or a sum more than about 0.5 s late was "not
heard in the window the START implies"). The lateness is per unit (the smallest of its measured outputs' delays): two
outputs of one unit whose `out.delay` differ by more than about 85 ms still meet the old limit. The real start offsets on hardware are not known yet (ATHIL notes them).

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
`system.align` writes the units.

**The hub guards the apply** (WP ATGUARD, [hub-api.md section 23.4](hub-api.md#23-measuring-several-units), cap
`system_apply_guard`). Before it files anything the core begins the hub's guard (`system.guard.begin`); the hub files
the model itself and starts a countdown. Its length is the proposal's `guard_s`: the planned verify, 25 % longer, plus
120 s, at most 590 s. Unless Keep confirms it in time, the hub sets the filed model back and aligns the units **by
itself**: at the deadline, when the applying console's session ends (a crash, a closed page, a lost connection), and
when the hub stops. A measuring session of that console (a verify still playing) is stopped first. While the guard is
pending, a `system.set` from another session is refused `conflict`, and a model someone changed through the hub's own
API is not overwritten (the outcome is `not_written`, said in words). A client shows the proposal's `guard_note` before
the apply and counts `applied.guard_s` down after it.

While an applied alignment waits for Keep or Revert, `measure_system_start` is refused (review AT-D2): a new
measurement would replace what was filed, and the alignment could then be neither kept nor reverted from that console.

**A hub without the cap** (an older hub) has no guard: nothing reverts by itself, not at a deadline and not when the
connection is lost. The proposal's `guard_note` and the apply's message say so, `guard_s` is 0, and the alignment stays
until Keep or Revert.

**Verify.** The summed runs again, with the applied delays predicted, judged up to a quarter of the rate. It needs fresh
unit checks again. A wrong delay of more than a few tens of microseconds does not survive it. A failed verify says
`measure.system.verify_failed`.

**Keep takes two steps after a failed verify.** The first `measure_system_keep` is refused with the reason in
`applied.message` ("the verify did not confirm the alignment; ..."); the client shows it and asks again, and only the
person's **second press, at least 1 s later**, keeps it. Nothing presses it for them. A new verify asks again. After a
verify that passed, or before any verify, one Keep keeps. Keep confirms the hub's guard (`system.guard.confirm`). A Keep
that comes after the hub has ended the guard by itself is answered with what the hub did, in its words
(`applied.stage` is then `reverted`; the message says so when the hub could not put everything back). A verify is
refused the same way once the guard has ended.

**Revert (Admin)** sets the filed model again at the revision the apply left, aligns, and reads every unit's output delays
back against the filed ones. Under the hub's guard the hub does the set and the alignment (`system.guard.cancel`), which
ends the guard. The alignment the hub writes back is the filed model's: units that did not hold that model's alignment
before the measurement end up aligned to it, and the read-back then names the outputs that differ from the filed delays.

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

The screens' own rows (WP ATWEB; Qt uses the same) and the demo's (WP ATDEMO):

| Key | Title |
|---|---|
| `measure_system_runs` | The group needs several runs |
| `measure_system_placement` | Choose where the microphone is |
| `dlink.not_one_domain` | These units do not share a clock (the hub's refusal; the screens show it as `measure.system.not_one_domain`) |
| `dlink.not_locked` | A unit's time base is not locked (the hub's refusal; shown as `measure.system.not_locked`) |
| `dlink.no_timeline` | A unit cannot start on the time base (the hub's refusal; shown as `measure.system.no_timeline`) |
| `spdif_no_time_base` | This S/PDIF source does not carry a DSPi time base |
| `measure.system.sim_refused` | The simulated microphone cannot hear these units (the demo hub only) |

**When the hub stops a run itself**, the core says why from the hub's words and names the unit: a time base that was
disturbed or whose timeline moved is `measure.system.abort.disturbance`, one no longer locked
`measure.system.abort.time_base`, anything else `measure.system.abort.hub`. `measure.system.abort.disconnected` is
only a hub that no longer answers.

Two refusals come as plain errors with words and no key of their own: the summed check's "would cancel" refusal
(section 9) and `open`'s `device` refusal for a unit the hub could not time (section 4).

**The core's calls** (`dspi_core`, UniFFI, the C ABI `dspi_measure_system_*`, wasm): `measure_system_unit_check`,
`_preflight`, `_start`, `_capture_push`, `_tick`, `_state`, `_stop`, `_result`, `_proposal`, `_sum_check`, `_apply`,
`_verify`, `_applied`, `_keep`, `_revert`; and, in the C ABI and wasm only, `_sim_set` and `_sim_info` (the demo hub's
simulated microphone, below). One measurement document per unit, with an additive `system` block (group,
time base, domain, anchor, run ids, per output the arrival relative to the anchor, its uncertainty, whether offered).

| Client | State |
|---|---|
| Web | built (WP ATWEB): the Measure screen's "Several units" tab; the S/PDIF transport in the Installer's clock panel |
| Qt | built (WP ATQT): Tools > Measure…, the "Several units" tab; the S/PDIF card in Window > System, "Clock and audio links" |
| Terminal, Android, Swift | later (PLAN-AT "Later": web and Qt first). Android's System screen shows a unit fed by S/PDIF with its S/PDIF state and does not count it as an audio link unit; it has no S/PDIF editor |

**Trying it without hardware** (WP ATDEMO). `uconsole --demo hub --alink --with-measure-units` attaches simulated units
on one time base (DEMO-TB-A, -B, -C, with C re-anchoring every 15 s), one on another (DEMO-TB-X) and an S/PDIF source
with a follower in lock error, and gives the hub a simulated room. That hub alone offers the cap
`system_measure_demo_mic`; with the simulated microphone on (`measure_system_sim_set`), the core reads what the room
heard from the hub on every tick and the client opens no microphone and pushes nothing. Nothing plays and nothing is
captured anywhere. It fails closed: the hub answers only for a session whose every unit is one of the room's own
attachments (the same device object, not just the serial), the core asks before every session and starts nothing on a
refusal, a refusal during a session aborts it (`measure.system.sim_refused`), and the core turns the simulated
microphone off on any hub without the cap. A real DSPi is never measured with it.

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
  (H7: 250 ms clean, a short lead refused), the phase error under load (H6), and the real USB-direct start offsets (the
  lateness the planner plans for, section 7).
- Part B: one system measurement with the web or Qt client on two units, a cable pulled mid-run, an Admin apply with the
  summed check, Revert; or, with a wandering microphone, "timing unreliable" with the wander shown.
- **Overlapped starts** (WP ATOVL, section 5): two units with the owner's microphone overlapped (the result says
  "overlapped starts, 50 ms apart" and the delay is offered), the same pair interleaved for comparison (the delay and,
  below 300 Hz, the later unit's response), and a third unit 20 ms further than its terms (measured again interleaved).

## 12. Not done yet

- Terminal, Android and Swift screens (later); an S/PDIF editor for Android and Swift.
- A verify's curve is not drawn (its result is in words).
- Several domains in one group, an S/PDIF relay (mode 3), ADAT, I2S/TDM and PTP time bases,
  cross-unit polarity, crossover and EQ proposals, the ops on RP2040 (PLAN-AT "Later").
- Scenes at F on the S/PDIF time base (the hub applies S/PDIF units' scenes at once).

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
