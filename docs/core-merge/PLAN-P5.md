# PRO phase 5: measurement and auto-tune

*WP PLANP5, 2026-11-16, written on `main` at `462fcde1` (phase 3 merged through P3-C and WBCLIP). The spec is PRO1's
report (`work/pro1/REPORT.md` section 2.6 rows T1 to T5, section 4 "Phase 5", section 5, section 6 "What not to do",
question 9) and **the owner's requirements of 2026-11-16, which are binding** (R1 to R5 below). Phases 1 to 3 are merged;
this plan is written against that code, cited `file:line`. Plan plus one compiling seam: the crate skeleton
`crates/dspi-measure` (its shared types in `src/types.rs`, an empty file per module so wave P5-A needs no shared edit),
committed with this plan. No other code.*

**Goal.** Tune a DSPi with a calibrated measurement microphone instead of by ear, in every client from one
implementation: measure what each output really does in the room (or on a headphone coupler), show it against a target
live, adjust directly from the measured curve, and let the system tune a multi-way speaker by itself (time alignment,
polarity, crossover, levels, then a target EQ fit), with every change previewed, bounded, guarded and reversible. **The
core owns the workflow and the maths** (a new pure crate `dspi-measure` for the maths, a sans-IO `measure` module in
`dspi-core` for the workflow); a client contributes only its microphone's samples and its screens. **The stimulus is the
DSPi's own signal generator**, so no host ever plays test audio; the microphone is a host audio input. Results are
documents of the `.dspipreset` schema-1 family, filed locally or on the hub; applying a tune is the guarded apply, the
undo group and the A/B compare that exist. Web and Qt get the screens first, the Terminal gets offline analysis and the
apply path, Android and Swift come in a later batch.

**The owner's requirements (2026-11-16), binding:**
- **R1. Part of the system, not an add-on.** The core owns the measurement workflow and the maths, every client gets the
  same feature; results are documents in the system (snapshot and preset family, undo, A/B, read-only and lock
  respected, hub-side for the web and the fleet).
- **R2. A calibrated measurement microphone.** Import the microphone's calibration file (common formats: miniDSP/UMIK-style
  `.txt` with a sensitivity line, REW-style frequency/dB[/phase]); USB measurement microphones are host audio inputs.
- **R3. The user chooses the microphone's placement:** speakers (listening position, near-field, several positions
  averaged) or headphones (a coupler type: IEC 60318-4 / 711 ear simulator, flat-plate coupler, a generic ear coupler or
  HATS, each with its own compensation and target).
- **R4. Direct responses:** the live measured response (sweep and/or RTA with pink noise), shown with the target, and
  direct adjustments from it.
- **R5. Auto-tune:** e.g. a three-way crossover: measure each way, align timing (delay) and polarity, tune the crossover
  (frequency, slope or type, levels), then a target or room EQ fit; every change reversible (a snapshot before, A/B, the
  guarded apply with confirm and revert) and bounded (never past a limit, gain and boost caps, speaker-safe levels: start
  quiet, ramp, abort on clip; **max 80 % host volume and a guarded player only** for test audio, since test audio once
  fell back to the owner's speakers).

**Other rules this phase builds on:**
- **PLAN.md section 1:** the core owns logic; typed writes only through `Writable` (`Inner::writable_conn`); the client
  read-only mode and a read-only connection refuse every write before it is sent; listeners are called with the `Inner`
  lock released; no panic on device, file, network or caller data (section 1.7); property tests for every parser and
  every numeric routine (section 1.9). Web rule (PLAN-CLIENTS section 1): time through a seam, no ungated OS calls, no
  needless threads.
- **The test-audio rule** (memory `test-volume-limit`): at most 80 % input volume, and test audio must never be able to
  reach another output. This plan meets it by construction: **phase 5 has no host player at all** (OD-P5a).
- **Compatibility policy (owner, 2026-11-15):** no shims between our components; our clients must keep working against an
  upstream Weeb Labs hub and over USB; our hub against upstream clients is best effort. New documents are new
  `meta.kind`s, not new formats (PRO1 section 6).
- **Process:** parallel Opus WPs in waves of disjoint owned files, one Fable review per wave, nits folded into later WPs;
  toolchains in containers, no tmpfs, no devices in a WP; the K340 and the microphone stay with the coordinator and the
  owner (P5HIL). Swift work on the Mac VM is slow, so Apple WPs are few and separate: this phase has none.
- **Naming:** "measurement" is a run and its document; "generator" is the DSPi's test signal generator (`siggen`);
  "RTA" alone is ambiguous from now on: the **device analyser** is the firmware's (`rta.rs`), the **microphone RTA** is
  the host's (P5DSP). "alink"/"dlink" as before; "Snapshot" is the host whole-device state.

**What exists today (checked in the code on `462fcde1` and `bofr-main` at `8ba8b72`):**

| Phase-5 need | State | Builds on |
|---|---|---|
| A stimulus | **The firmware has a test signal generator**, upstream since v1.1.5 (`~/prog/DSPi` `dfc9fb3`, 2026-07-05) and on `bofr-main`, RP2040 and RP2350 alike (`Documentation/Features/test_signals_spec.md`). 15 types incl. log sweep (4) and pink noise (3); `level_db` is a **peak dBFS, -120..0**; finite or infinite `repeat`; `SIGGEN_FLAG_WALK` plays the masked outputs one per cycle; **per-cycle synth state is reset, so repeated sweeps are bit-identical** (spec section 6.2, 7); 5 ms fades; injected after the matrix and **before** each output's crossover, PEQ, delay, trim, master and user volume and mute (`audio_pipeline.c:536`, `:351`); `SIGGEN_FLAG_RAW` bypasses crossover and PEQ; a preset load stops it (`STOP_PRESET`); `NOTIFY_EVT_SIGGEN_STATE` 0x07 on every start and stop; it runs with no host stream (the pump) | the whole signal path (section 1.1) |
| Generator in the core | `siggen_fetch_caps` (`crates/dspi-core/src/stats.rs:255`), `siggen_get_config` (`:320`), `siggen_set_config` on the `Writable`, journalled history-only (`:344`), `siggen_control` (`:370`), `siggen_get_status` (`:397`); the read-only mode refuses it (`read_only.rs`); the demo device implements it (`crates/dspi-transport/src/fake/analysis.rs:17-20`) | P5SES drives it; nothing new on the wire |
| Generator over the hub | `SIGGEN_SET_CONFIG` 0xA4 and `SIGGEN_CONTROL` 0xA6 are class `control` (`tests/fixtures/link/policy/commands.json` rows 156, 158), so Control and Admin may play it, Viewer may not | the web measures through the hub as the caller |
| The device analyser | `rta.rs` (C9), `dspi_session::rta`, `rta_sharing` (CG19): an FFT on the device at the input or output tap, band and bin frames, one configuration per device shared between consoles; web `screens/analysis/SpectrumStrip.tsx`, Qt `RtaController` | unchanged; T5's hold and compare (P5LIVE) also applies to it |
| Generator and analyser screens | web `apps/web/src/screens/analysis/ToolsScreen.tsx`, `siggen.ts`; Qt `SiggenController`, `RtaController` | the measurement screens are new, beside them |
| Filter maths | `dspi_proto::dsp` (`coefficients` `:75`, `response_at` `:260`, `phase_at` `:818`), `dspi_proto::xover` (`sections` `:197`, families as `crossover.c` builds them); the core's cascade with the Linkwitz Transform (`crates/dspi-core/src/curves.rs:83`, `:97`), `curve_sections` (`:433`), the setters' quantizers (`:388-415`) | the fit and the crossover search evaluate candidates with the core's own cascade through `SectionModel` (OD-P5c) |
| Typed writes | `set_filter` (`params.rs:88`), `set_crossover` (`:120`), `set_matrix_route` with `invert` (`:660`), `set_output_gain` (`:721`), `set_output_delay` (`:760`), `write_param` (`write_param.rs:171`); delay is 0..`Platform::max_delay_ms` (21 or 42 ms, `registry.rs:906-911`); `vol.user` -60..0 dB (`registry.rs:506-514`) | a tune is a list of these writes (OD-P5h) |
| Undo, guard, A/B, snapshots | undo groups `begin_group`/`end_group` (`journal.rs:1087`, `:1116`); the guarded apply `apply_guard_begin` (`guarded.rs:476`), `apply_confirm` (`:485`), `apply_revert` (`:520`), hub-side guards (P1HA, `crates/dspi-hub/src/apply_guard.rs`); A/B `ab_start` (`ab.rs:396`), `ab_start_documents` (`:410`); `snapshot_capture_document` (`snapshot.rs:433`), the stores (`dspi_session::snapshot_store`, the hub's `snapshots.rs`, 256 KiB a document) | the apply path of every tune and every direct adjustment |
| Documents | `.dspipreset` schema 1, `meta.kind` read by `doc_kind` (`crates/dspi-core/src/files.rs:2027-2031`): `preset`, `snapshot`, `template`, `limits` | three more kinds: `measurement`, `mic`, `target` (OD-P5g) |
| Limits | `OutputLimits` (`crates/dspi-session/src/limits.rs:167`: `max_gain_db`, `highpass_min_hz`, `highpass_min_slope_db`, `delay_min_ms`, `delay_max_ms`, `limiter_ceiling_db`), the profile (`:195`: `max_peq_boost_db`, `master_volume_max_db`, `locked_pages`); every typed write is judged (`crates/dspi-core/src/limits.rs`) | caps of the fit, the crossover search and the delay proposal; the tweeter rule (S6) |
| Device lock | `device_lock_acquire(reason, timeout_ms)` with a lease of at most 60 s (`presets.rs:482`, `:77`), `device_lock_release` (`:507`); over DSPi Link other sessions get `locked`; elsewhere it asks no one | a measurement run holds it (section 1.5) |
| Meters and clip | `fetch_status` (`connect.rs:239`): `peaks`, `clip_flags`, hold and max; the clients poll it already; 0xE0 clip events on `bofr-main` (`EXT_NOTIFY_FEATURES` bit 1) | the silence check and abort-on-clip without a new read |
| Modes and page locks | `assets/modes/modes.json` screens (`front`, `system`, `channels`, `routing`, `dynamics`, `hardware`, `history`, `limits`, `diagnostics`, `firmware`); an installer locks pages by id | a `measure` screen id (P5SES) |
| Templates | `assets/templates/3way-rp2350.dspipreset` (300 Hz / 3 kHz), `2way-*`, `headphones-*`, `k340.dspipreset` | layouts the tune recognises (section 1.4 step 1) |
| Host audio | **None anywhere**: no capture and no playback in the core, the clients or the hub (grep for `getUserMedia`, `AudioRecord`, `cpal`, `QAudioSource`, `AVAudioEngine`, `AudioContext`) | P5WCAP (web), P5QCAP (Qt) are the first |
| FFT | none in `Cargo.lock` | P5DSP adds `realfft`/`rustfft` (OD-P5d) |
| Qt image | no Qt Multimedia in `ci/container/Containerfile.qtport:29`, `:33` | P5QCAP's image bump |

**Corrections to PRO1 found while checking:**
- PRO1 T1 has the host "play a log sweep or pink noise through the DSPi as its USB output, or the device's generator".
  The generator exists on stock firmware and is better in every respect that matters here: it cannot fall back to
  another output, it is sample-exact and repeatable, it needs no per-platform playback code. **Generator only**; no host
  playback is built (OD-P5a).
- PRO1 T1 and section 6 say phase needs "a loopback reference", so "ship magnitude-only first". Not so for what the tune
  needs: the generator's walked, repeated sweeps put every way on **one cycle grid in one capture**, and the grid itself
  is the timing reference, the clock drift measured from it (section 1.1). Relative phase and delay between ways are
  exact; only the absolute latency is unknown, and nothing needs it. Phase is in phase 5 (OD-P5b).
- PRO1 T1 gives the core "a `Capture` seam per platform". The core opens no audio device (the web's wasm core cannot);
  it is **sans-IO**: a client pushes its microphone's samples, the core asks for them (OD-P5e).
- PRO1 T4 fits "the 10 PEQ bands". A fit uses the **free bands** (flat or bypassed) unless the user releases others, so
  a tune never overwrites someone's EQ silently (OD-P5l).
- PRO1 T5 wants "calibrated input level if the firmware adds it". The calibrated level is the microphone's, on the host,
  from its calibration file (R2); no firmware.
- PRO1 question 9 is answered by R2: a USB microphone on the host.
- PRO1 sizes phase 5 at about 20 units. The owner's requirements add the microphone calibration, placements and
  couplers, the live view, the multi-way auto-tune, the hub documents, the safety engine, Qt capture and a simulated
  world for testing without hardware: **this plan is 55.75 units plus 1 for the HIL run** (section 2, "Waves"), and a
  later Android and Swift batch of about 9.5.

**Not in this phase:** host playback of test audio (never, unless the owner asks, OD-P5a); measurement screens in
Android and Swift (a later batch, sized in section 2); a microphone on the hub's machine (OD-P5f); impulse-response
convolution or FIR filters on the device (the firmware has none); more PEQ bands (FW7, upstream); AutoEQ's handling
(PRO1 T6, separate); fleet-wide tuning (a tune is per device and room); a measurement started over REST (OD-P5r).

---

## 1. Goal and scope, item by item

PRO1's T1 to T5 keep their ids; the owner's requirements add T7 to T12 (T6 is PRO1's AutoEQ item, not in this phase).

| Item | What the core does | Where | Clients |
|---|---|---|---|
| **T1 transfer function** | From the generator's walked log sweeps and the microphone's capture: deconvolution to an impulse response per output and cycle, the clock drift from the cycle grid, coherent averaging of repeats, windowing (frequency-dependent by default), magnitude and phase on a 1/96-octave grid from 10 Hz to 22 kHz, fractional-octave smoothing, a noise floor from a pre-roll and a per-point "trusted" mask from SNR and repeat-to-repeat spread; REW text export (frequency, dB, phase; impulse response) | `dspi-measure` `stimulus`, `ir`, `response`, `export` (P5DSP); the run in `dspi-core` `measure` (P5SES) | every client through the core |
| **T2 delay and polarity** | From the ways' impulse responses on the common grid: coarse arrival from the onset, fine delay and polarity that make the summed response flattest through each crossover region; delays normalised so the latest way gets 0, within the platform's range and the limits profile's; polarity as the feeding crosspoints' invert | `align` (P5XO); the tune (P5TUNE) | all |
| **T3 target curves** | Built-in targets defined by parameters in our own code (flat, tilt, house curve with a bass shelf; a default per coupler type), import of any REW or CSV target, coupler compensation import, normalisation at a reference band; shown dashed on every graph, used by the fit | `target` (P5CAL); documents (P5DOCK) | all |
| **T4 room and target EQ fit** | Fit the channel's free PEQ bands toward the target inside a range (speakers: 20 Hz to 300 Hz by default, the Schroeder region, wider by choice; headphones 20 Hz to 10 kHz), bounded boost and cut, no filling of position-dependent or deep dips, headroom kept by an equal output-gain cut; preview with the predicted curve, apply guarded with A/B | `fit` (P5FIT); the tune and the live view (P5TUNE, P5LIVE) | all |
| **T5 RTA improvements** | The **microphone RTA** (Welch, pink-referenced, 1/1 to 1/48 octave, calibrated to dB SPL when the microphone is) with averaging presets (fast, slow, infinite), hold and compare of two captures; hold and compare also for the device analyser | `rta` (P5DSP); `measure/live.rs` (P5LIVE) | web, Qt (bar strip and graph exist) |
| **T7 microphone calibration** | Parse miniDSP/UMIK-1/UMIK-2 `.txt` (a `"Sens Factor =…dB, AGain=…dB, SERNO: …"` line, 0° and 90° files), REW calibration text (frequency, dB, optional phase; `*`, `#` and `"` comment lines), CSV (comma, semicolon, tab) and Dayton-style sensitivity lines; bounded; to the grid; the sensitivity to an absolute dB SPL scale with the vendor's published convention cited in the code, or a calibrator step (94 or 114 dB at 1 kHz), or relative dB when neither exists; a `mic` document per microphone | `cal` (P5CAL); documents (P5DOCK) | all |
| **T8 placements and couplers** | Speakers: listening position; several positions (3 to 9) power-averaged with the spread kept; near-field per driver spliced to a far-field measurement below a splice frequency. Headphones: coupler type (IEC 60318-4 ear simulator, flat-plate, HATS or other ear coupler), reseats (1 to 9) averaged, left and right each; each coupler type with its own default target and an optional compensation file | `types::Placement` (seam), `position` (P5CAL); the run (P5SES) | all |
| **T9 live response and direct adjustments** | Live mode: short sweeps repeated (one result per sweep) or pink noise with the microphone RTA, shown with the target and with the **predicted** curve of a pending change; direct adjustments: "cut this peak" and "fit this range" from a point or a span on the measured curve, each one typed write or one undo group, previewed first | `measure/live.rs` (P5LIVE) | web, Qt |
| **T10 auto-tune** | Section 1.4: layout, per-way measurement, delay and polarity, crossover (frequency, family and order, levels), the summed check, the target fit, one proposal with every write in words, applied as one guarded undo group between a "before" and an "after" snapshot, verified by a measurement inside the guard, kept or reverted, A/B offered | `xover`, `align`, `fit`; `measure/tune.rs` (P5TUNE) | web, Qt, Terminal (apply from a document) |
| **T11 documents and the hub** | Kinds `measurement`, `mic`, `target` in the `.dspipreset` schema-1 family (section 1.6); a local store; the hub's `measure.doc.*` family and its store, merged with the local list as snapshots are (P2CC1's rule: the hub's copy wins); REST reads within v1 | `dspi-measure` `doc` (P5MDOC), `measure_doc.rs`, `measure_store.rs` (P5DOCK), `crates/dspi-hub/src/measurements.rs` (P5HUB) | all; the web reads the hub's |
| **T12 safety** | Section 1.2: a preflight, a closed-loop level ramp from a quiet start, mode ceilings, abort on clip or SPL or anything foreign, only finite generator runs, never `RAW`, never a raised volume, the tweeter rule, the device lock as a dead-man lease | `measure/safety.rs` (P5SES) | all |

### 1.1 The signal path (T1, OD-P5a, OD-P5b)

1. **Stimulus: the DSPi's generator, through the output's own processing.** A run is one generator configuration: type
   log sweep (4), `channel_mask` = the outputs measured, **`SIGGEN_FLAG_WALK`** (one output per cycle, round-robin),
   `repeat` = passes (2 to 8; default 3), `duration_ms` = one sweep (default 2.7 s for live, 5.5 s for a measurement;
   1 to 15 s), `gap_ms` = silence after each sweep (default 500 ms, at least the room's decay), 20 Hz to 20 kHz (narrower
   for a way by choice), `level_db` from the ramp (section 1.2), **flags without `RAW`**. The generator reaches the air
   through the output's crossover, PEQ, delay, trim and volumes, so what is measured is what the device plays.
2. **Capture: the host's microphone, pushed by the client.** The client opens the microphone (web: `getUserMedia` with
   echo cancellation, noise suppression and auto gain **off**, through an AudioWorklet; Qt: `QAudioSource`), and pushes
   mono `f32` blocks with their rate to `measure_capture_push`; the core asks for capture through `MeasureState.wants_capture`.
   Rates 44.1 to 96 kHz; the browser's or OS's resampling is harmless (a common latency).
3. **The core renders its own copy of the sweep.** `dspi-measure::stimulus` models the generator as `siggen.c` renders it:
   the 48-bit phase accumulator with the Q31 per-sample growth of a log sweep (`siggen.c:360-385`, `:926-940`), the
   attack and release windows, the cycle length `fs * duration_ms / 1000` and the gap in samples, the walk order and the
   per-cycle reset, at the device's sample rate (44.1, 48 or 96 kHz rows; other rates use the 48 kHz row, spec 6.4).
4. **The timing reference is the generator's cycle grid.** The capture is deconvolved coarsely (one reference sweep,
   matched filter), giving one arrival per cycle. Cycle `c` starts at `c * P` device samples (`P` = sweep + gap). A
   straight-line fit of arrival against `c` (outliers dropped, at least 3 cycles) gives **the clock ratio** between the
   DSPi and the microphone (independent clocks: the generator runs on the DSPi's clock with no host stream) and an offset
   (the unknown latency). The capture is resampled by that ratio (band-limited, fractional), then each cycle is
   deconvolved exactly and the repeats of an output averaged coherently. An output's arrival **relative to its own
   cycle's start on the grid** is its delay; the common latency cancels between outputs, and so do the host's and the
   browser's buffering. A run of a single output with one repeat cannot estimate drift: such runs are magnitude-only
   (the live view), never used for alignment.
5. **No loopback, no reference speaker, no firmware time stamp** (OD-P5b). What the grid cannot give is the absolute
   latency from USB to air, which nothing in this phase needs.
6. **Noise and trust.** A 1 s pre-roll with the generator idle gives the noise floor; per point, SNR above 20 dB and a
   repeat-to-repeat spread under 1 dB mark it trusted (`Response.valid`); untrusted points are drawn faint and never
   fitted.
7. **Processing in slices.** Deconvolution runs per cycle as its samples arrive, each slice bounded (one FFT of the
   cycle's length, about 2^19 points at 96 kHz), inside `measure_tick`, so a web Worker or a Qt timer is never blocked
   for long (a budget of about 20 ms of work per tick, measured in P5SES's tests). Raw audio is bounded
   (`MAX_CAPTURE_SAMPLES` 4 Mi samples, 16 MiB) and dropped when the run's analysis ends.

### 1.2 Safety (T12)

The owner's rule, R5, item by item, and what enforces it. **S1 to S12 are the tests of P5SES and P5TUNE.**

- **S1. No host player, ever.** The stimulus is the DSPi's generator; no client plays test audio. If the DSPi leaves
  USB, nothing anywhere falls back to another output: the signal never existed on the host (OD-P5a). A client that
  finds no generator (firmware before v1.1.5) offers the microphone RTA of programme audio and offline analysis only.
- **S2. Volume.** The session never writes `vol.user`, `vol.master`, a mute or an output gain while measuring (no hidden
  writes). Preflight **refuses** when `vol.user` is above **-12 dB** (80 % of the registry's -60..0 dB range; stricter
  than what an OS slider at 80 % sends), with the notice to lower it; the master volume is judged by the limits profile
  as today.
- **S3. Start quiet, ramp, cap.** Before a run, a ramp per output: 1.5 s of pink noise from the generator at the start
  level, measured; +6 dB a step until the microphone's SNR is 35 dB in 100 Hz to 10 kHz, or a ceiling is reached. Start
  levels: speakers **-50 dBFS**, headphones **-60 dBFS**. Ceilings (generator peak level): speakers **-12 dBFS**,
  headphones **-30 dBFS** by default; the user may lower them and raise them only to the hard caps, **-6 dBFS** and
  **-20 dBFS**. With a calibrated microphone the ramp also stops at **95 dB SPL** (speakers) or **90 dB SPL** (on a
  coupler), Leq over 125 ms (hard caps 105 and 100). The sweep runs at the ramp's level. Never full scale.
- **S4. Abort on clip.** A capture sample at -1 dBFS or above, a measured output's peak at -0.5 dBFS or above in
  `fetch_status` (or a 0xE0 clip event where the firmware has it), or the SPL past its cap: `SIGGEN_CTL_STOP_NOW` at once,
  the run discarded, the notice says which. A ramp step that clips ends the ramp at 12 dB below that step.
- **S5. Abort on anything foreign.** A `NOTIFY_EVT_SIGGEN_STATE` stop that is not ours (another console, a preset load:
  reason `PRESET` or `HOST`), a foreign write on a measured output's chain or on a volume (the journal's
  `note_foreign`, a `PARAM_CHANGED` or `BULK_INVALIDATED` of another origin), the device lock lost or `locked`, a
  disconnect, the capture stopping or short by 2 % or more, the run taking 5 s longer than planned, or the user's Stop:
  STOP_NOW where the device is reachable, the run discarded, nothing retried.
- **S6. The tweeter rule.** Never `SIGGEN_FLAG_RAW`. Preflight refuses to measure an output whose role is `Mid` or `High`
  unless it has an active high-pass (a crossover band of a high-pass type, or a PEQ high-pass) at or above the limits
  profile's `highpass_min_hz` when there is one, else at or above 200 Hz for `High` and 40 Hz for `Mid`; the notice says
  to add the protective high-pass first. The tune never proposes a high-pass lower than that, nor a slope shallower than
  `highpass_min_slope_db`. Driver data without the filter comes from de-embedding the known chain (section 1.4 step 3),
  never from bypassing it.
- **S7. Only finite runs.** Every generator configuration the session stages has `repeat >= 1` or a `duration_ms` of at
  most 60 s (the live view re-arms noise every 30 s while the client is alive). A client that dies leaves a run that ends
  on its own within its planned length (at most 120 s), and a device lock whose lease (30 s, renewed by `measure_tick`)
  runs out.
- **S8. Silence first.** Preflight refuses while any input's peak in `fetch_status` is above -70 dBFS over 1 s (programme
  audio would reach the outputs not under test and spoil the run): "stop playback to measure". The session never
  switches the input source.
- **S9. Bounded proposals.** Every write of a proposal is judged before it is offered: inside the registry's range, the
  limits profile (gain, high-pass, delay, boost), the platform's delay maximum; PEQ boost at most **+3 dB** (speakers) or
  **+6 dB** (headphones) and never above `max_peq_boost_db`; cuts at most -15 dB; Q 0.3 to 10; crossover and level
  changes **cut only** (no output gain is ever raised); the EQ's net boost offset by an equal output-gain cut so the
  chain's peak gain does not rise. A proposal with a write the core would refuse is not offered.
- **S10. Guarded and reversible.** A tune is applied only inside `apply_guard_begin` (default 300 s, enough for the
  verifying sweep), as one undo group, after a "before" snapshot is filed; Keep, Revert, the timeout, a failed write or
  a lost connection behave as every guarded apply does. A direct adjustment is one undo group, unguarded (it is one
  band, as a manual edit).
- **S11. Locks and roles.** Nothing is played or written without a writable connection, the client read-only mode off,
  the role (Control or Admin through a hub), the `measure` screen not locked by an installer, and no foreign guard or
  A/B compare pending.
- **S12. Privacy.** Raw microphone audio stays in the core's memory for the run only; it is never logged, put in an
  event, a document, the diagnostics bundle or sent to the hub. A WAV of a capture is written only when the user exports
  it. Documents hold derived responses and short impulse-response excerpts.

### 1.3 Placements and couplers (T8)

| Placement | Runs | What is stored | Used for |
|---|---|---|---|
| Listening position | one walked run of the outputs | each output's response with phase, its IR excerpt and arrival | alignment, crossover, fit |
| Several positions (3 to 9) | position 1 is the listening position and the timing reference; one run per position, the user moves the microphone between them (the session waits, `measure_next_position`) | per output: the power average of magnitudes, the spread per point, position 1's phase and arrival | the fit (dips that vary more than 3 dB across positions are not filled); alignment from position 1 only |
| Near-field | one run per driver with the microphone close to it; a far-field run of the same outputs | the near-field response spliced to the far-field below the splice frequency (default from the driver's size, or set), levels matched in a one-octave overlap | driver data for the crossover search; never the fit above the splice |
| Headphones | coupler type; reseats 1 to 9, left and right each (two outputs, or the channel pair the layout names) | per side: the average of reseats with the spread; the coupler type; the compensation applied | the fit per side toward the coupler's target; a left/right match option |

Coupler types and their defaults: **IEC 60318-4 ear simulator** (the target is defined at the drum reference point;
built-in default: a parametric diffuse-field-like curve with a bass shelf, labelled as ours), **flat-plate coupler**
(no standard compensation: the default target is flat-plate-specific and parametric, and the user is told a vendor's
compensation file is better), **HATS or other ear coupler with pinna** (as the ear simulator, with the pinna's own
compensation file if the user has one). No third-party target data is bundled (OD-P5p); any target or compensation is
imported from a file.

### 1.4 Auto-tune, step by step (T10, R5)

Example: a stereo three-way speaker (outputs 0 to 2 left low, mid, high; 3 to 5 right). Every step shows its result
before the next, and the user may stop after any step with nothing written.

1. **Layout.** The core proposes speakers and ways from the device's state (an output with only a low-pass is a `Low`,
   high-pass and low-pass a `Mid`, only a high-pass a `High`, a subwoofer by name or band; the matrix's sources pair left
   and right), the user confirms or edits it. A template's layout (`3way-rp2350`, `2way-*`, `headphones-*`) is recognised.
2. **Preflight** (section 1.2): every blocker in words, nothing played until it is clear.
3. **Measure.** Per speaker, one walked run of its ways at the listening position (S3's ramp per way first); with
   several positions, the further runs. The measured responses hold the chain each way had (its crossover, PEQ, gain,
   delay, polarity, from the mirror), so **each driver's own response is the measurement divided by its known chain**
   (de-embedding through `SectionModel`): the tune can then predict any new chain exactly, and a measurement stays valid
   if the user edits the chain later (OD-P5o).
4. **Delay and polarity** (T2): proposed per way, with the reason ("the tweeter arrives 0.21 ms early: delay it"; "the
   mid's polarity is inverted against the low"). The user may keep the current delays.
5. **Crossover**: per transition (low/mid, mid/high), a search over the corner (1/12-octave steps, within half an octave
   of the current one by default, wider by choice, never below S6's high-pass rule), the family and order among the types
   the firmware builds (Linkwitz-Riley, Butterworth, Bessel; orders the type table has), and the ways' levels (cut
   only), each candidate scored by the predicted acoustic sum against the target through the transition, a power-response
   penalty, and a penalty for corners near a driver's own roll-off or breakup. "Keep the crossover, align only" is an
   option. The result: per output its crossover bands, gain change, delay and polarity, and the predicted sum.
6. **Sum check.** One run with both ways of a transition playing together (no walk, the generator's correlated copy):
   the measured sum against the predicted one. Further than 3 dB RMS through the transition: the proposal is marked
   "measured sum differs" and the user decides.
7. **Target fit** (T4): per speaker side, the free PEQ bands toward the target, on the predicted response with the new
   crossover, within the fit's range and caps (S9).
8. **Proposal.** One list: every write in words ("Output 2 (L high): crossover 2.8 kHz LR4 → 3.1 kHz LR4; delay 0 →
   0.21 ms; PEQ band 6: peak -3.5 dB at 1.9 kHz Q 2.1"), the curves before and predicted, the limits it was held to.
   Nothing is written yet.
9. **Apply.** `snapshot_capture_document` "Before auto-tune <date>" filed (the hub's store through the hub, else the
   local one); `apply_guard_begin("Auto-tune", 300)`; `begin_group("Auto-tune")`; the writes through `write_param`
   (each judged by read-only, limits and the lock as any write); `end_group`. A refused write stops the apply and the
   guard reverts.
10. **Verify** inside the guard: one run of the tuned outputs; measured against predicted; the guard's countdown shown.
11. **Keep or revert.** Keep: `apply_confirm`, "After auto-tune <date>" filed with a `tune` block (the measurement's id,
    the algorithm version, the options), and the A/B compare of the two snapshots offered (`ab_start`). Revert or the
    timeout: the guard's revert; nothing else changes.

### 1.5 Locks, read-only, other clients, the hub

- **The device lock.** A run holds `device_lock_acquire("Measuring", 30 s)`, renewed by `measure_tick` while the session
  is active, released at the end, on abort and on drop. Over DSPi Link every other session's writes get `locked`, and
  its lock indicator shows the reason, as for a preset save today. Over USB the lock asks no one; a foreign write is
  then caught by S5. The user's own edits in the live view are the lock holder's and pass.
- **Read-only.** A client in read-only mode, a read-only connection or a Viewer through the hub cannot play the
  generator: it gets the **passive** microphone RTA (programme audio, no stimulus), document viewing and offline
  analysis; preflight names the reason.
- **Other clients** see the generator's state from its notification (existing), the lock's reason, and saved
  documents through the hub's `measure.doc.changed`. A measurement in progress is the measuring client's; it is not
  mirrored to others.
- **The hub** stores and serves documents and arbitrates through the lock it already has; **the measurement runs where
  the microphone is**: in the browser's core for the web (the wasm core is the core), in the Qt app, never on the hub's
  machine (OD-P5f). Every device write is the measuring client's, judged as the caller's by role, limits and lock; the
  hub adds no write of its own.
- **Several devices.** One session per `DspiCore` (per device); a second `measure_start` on the same core is `busy`.

### 1.6 Documents (T11, OD-P5g)

All three are `.dspipreset` schema-1 documents (`{"schemaVersion": 1, "meta": {"kind": …, "name": …, "savedUtc": …,
"appVersion": …}, …}`), parsed with bounds, refused by `config_import` (they configure no device: a clear
`InvalidArgument`), at most 256 KiB compact (the hub snapshot store's bound).

| Kind | Holds | Scope |
|---|---|---|
| `measurement` | `measurement {serial, platform, savedMs, placement, mic (id, name, serial, sensitivity, the calibration's hash), stimulus (type, f1, f2, duration, repeat, gap, level_db, device rate), capture (rate, clock ratio, latency estimate), outputs: [{output, role, speaker, chain (the measured chain), arrival_ms, response (grid, mag, phase?, valid, spl), spread?, ir {rate, start_ms, samples base64 f32 LE, at most 16384}}], positions: n, notes}` and, after a tune, `tunes: [{at, snapshotBefore, snapshotAfter, kept}]` | per device serial |
| `mic` | `mic {name, serial?, vendor?, format (umik, rew, csv), angle (0, 90), sensitivity_db?, gain_db?, splOffset_db?, response (grid, mag, phase?), sourceName, sourceHash}` | the user's (hub-wide on a hub) |
| `target` | `target {name, use (speakers, headphones), coupler?, definition (builtin params, or imported points), compensation?, reference band}` | the user's (hub-wide on a hub) |

The tune's result is not a new kind: it is two **snapshots** (`meta.kind = "snapshot"`), "Before auto-tune" and "After
auto-tune", the latter with an additive `tune` block. REW export is text the user saves, not a document.

---

## 2. Work packages

**Shared rules for every P5 WP.** `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief, PLAN.md section 1, and:
- **`dspi-measure` is pure:** no I/O, no threads, no clock, no `dspi-core`, no `dspi-transport`; every public function
  has a `proptest_*` that it neither panics nor returns non-finite values; it builds for `wasm32-unknown-unknown`
  (`DSPI_PROFILE=cross ci/run.sh 'cargo build -p dspi-measure --target wasm32-unknown-unknown'`). Its lint set is the
  hub's (`unwrap_used`, `expect_used`, `indexing_slicing`, `arithmetic_side_effects` warned; `-D warnings` in CI).
- **Shared types are `crates/dspi-measure/src/types.rs`** (the seam committed with this plan): a WP that needs another
  shared type adds it there under its marker and lists it; nobody changes a field's meaning. The `mod` lines in
  `dspi-measure`'s `lib.rs` exist already.
- **The core's measurement code is sans-IO:** no thread, no sleep, no audio API; time from the core's clock seam
  (`dspi_transport::clock::Instant`, as `rta.rs`); listeners called with the `Inner` lock released; every device write
  through an existing typed setter on the `Writable`. Never `SIGGEN_FLAG_RAW`, never a write to a volume while measuring,
  never an infinite generator run (section 1.2).
- **Determinism:** the same inputs give the same proposal bit for bit (no hash-map iteration order, no time, seeded
  noise in the simulator); golden files under `crates/dspi-measure/testdata/p5/` and `crates/dspi-core/testdata/p5/`.
- **Testing without hardware** (section 4): the simulator (P5SIM) and the demo device; **no microphone, no audio
  device, no K340 in any WP**; the one real run is P5HIL's.
- **Bindings** (C, UniFFI, wasm) are shared and additive, under markers: run `ci/drift.sh write`, regenerate the Android
  contract and classify new methods in `WriteGuardedCore` (writes: `measure_start`, `measure_tune_apply`, `measure_live_*`
  that write; reads: the rest). Apple bindings are regenerated at merge by the coordinator; `ci/macvm.sh macos` once
  after P5-C.
- **Notices:** user text from `assets/notices/notices.json`, one contiguous block of rows per WP, no host-device nouns;
  "Measuring", "Stop playback to measure", the S-rule refusals, the tune's step texts.
- **Web:** unit tests and Playwright e2e in the container (`ci/check.sh --step web`), Chromium and Firefox, 375 px with
  no sideways scroll; the microphone in e2e is the browsers' fake media device fed with a WAV the simulator rendered.
- **Qt:** `ci/check.sh --step qt`, `--step qt-offscreen`, `--step qt-windows`; registrar-based controllers.

Sizes are this project's units (about one hour of agent time with review); estimates.

### Wave P5-A: the maths, the simulator, the document model (pure, parallel)

**P5DSP: Stimulus model, impulse responses, responses, microphone RTA.** [TODO] Size 5. Wave P5-A. **Owns:**
`crates/dspi-measure/src/{stimulus,ir,response,rta,export}.rs` (and a folder per module for tests), the FFT dependency
lines in `crates/dspi-measure/Cargo.toml` under its marker, the `Cargo.lock` change, the licence rows the new crates need
(`ci/licence-inventory.py` if it asks, the notices regenerated by `apps/desktop/packaging/licences/gen_rust_notices.py`
and `apps/terminal/tools/gen_rust_notices.py` once a client links the crate: at P5SES's merge at the latest).
**Implements:** section 1.1 steps 3, 4, 6, 7: `stimulus` (log, linear and stepped sweeps, pink noise's spectrum,
the walk order and cycle schedule, fades and windows, at 44.1/48/96 kHz, sample-faithful to `siggen.c` for the sweeps);
`ir` (the coarse matched filter, cycle segmentation, the drift line fit with outlier rejection, band-limited fractional
resampling, per-cycle deconvolution by the sweep's inverse (regularised spectral division), coherent averaging, the
pre-roll noise floor, windows: fixed gate and frequency-dependent (default 15 cycles)); `response` (FFT to the
`LogGrid`, unwrapped phase, smoothing 1/1 to 1/48 octave and variable "psychoacoustic", the `valid` mask from SNR and
spread, minimum-phase excess removal for display only); `rta` (Welch with Hann, 50 % overlap, fractional-octave bands,
pink-referenced so pink noise reads flat, calibrated through a `mic` response, exponential and fixed averaging, a held
copy); `export` (REW text: `freq mag phase` with a header; IR as text). Bounds: `MAX_CAPTURE_SAMPLES` (4 Mi), sweep 1 to
15 s, 8 repeats, 16 outputs a run. **Done when:** a known impulse response convolved with the modelled sweep is recovered
with an error below -60 dB; a microphone clock 100 ppm off is measured within 2 ppm and corrected; two outputs 1.234 ms
apart in one walked run come out 1.234 ms ± 5 µs apart at 48 kHz, also with 50 ms of unknown latency and 80 ppm drift;
3 repeats raise the SNR by 4.8 dB ± 0.5; the sweep model matches an independent integer re-implementation of
`siggen.c`'s recurrence (quoted line by line in the test; vectors from a host build of `siggen.c` if the firmware tree has
one) for the first and last 4096 samples at each rate; pink noise through `rta` reads flat within 0.5 dB per 1/3 octave
100 Hz to 10 kHz; a capture with a gap is `NoSignal`, never a panic; `proptest_*` over random captures and parameters;
wasm32 build; `cargo deny check` passes with the new crates. **Deps:** the seam. **Clients:** P5SIM's tests, P5SES,
P5LIVE, P5TUNE, P5TM.

**P5CAL: Microphone calibration, targets, positions.** [TODO] Size 3. Wave P5-A. **Owns:**
`crates/dspi-measure/src/{cal,target,position}.rs` (and test folders), `crates/dspi-measure/testdata/p5/cal/` (fixtures
**written by hand in each format**, never a vendor's file). **Implements:** T7's parsers and the sensitivity to dB SPL
(the vendor's published convention cited in a comment with where it was read; a calibrator path `spl_offset_from_tone(capture,
94 or 114)`; relative when neither); T3's targets: built-ins by parameters (flat; tilt in dB/octave; house curve =
bass shelf gain, corner and Q plus a tilt; a default per `Coupler` defined by parameters and named ours), import of REW
and CSV targets and coupler compensations, normalisation at a reference band (default 500 Hz to 2 kHz mean); T8's
maths: power average of magnitudes, the spread, near-field to far-field splice with a level match in a one-octave
overlap, reseat averaging, a left/right match measure. Files at most 1 MiB and 8192 lines, points sorted and
de-duplicated, outside 10 Hz to 22 kHz ignored, non-finite refused per line with the line number. **Done when:** each
format's fixture parses to the expected response and sensitivity; a UMIK-style 90° file is recognised as 90°; a file
with phase keeps it; garbage, an empty file, 10 000 lines, a binary file and non-UTF-8 text are typed errors; built-ins
are exact at their defining points; a splice is continuous within 0.1 dB at the splice frequency; `proptest_*` fuzz
every parser. **Deps:** the seam. **Clients:** P5MDOC (documents), P5SES, P5TUNE, P5LIVE.

**P5FIT: The bounded EQ fit.** [TODO] Size 4. Wave P5-A. **Owns:** `crates/dspi-measure/src/fit.rs` (and `fit/`).
**Implements:** T4: `fit(measured: &Response, target: &Response, chain: &ChannelChain, model: &dyn SectionModel,
options: &FitOptions) -> Result<FitResult>`; options: range, smoothing, max boost and cut, Q range, minimum |gain|, the
band indices it may use (free bands by default: flat or bypassed), shelves allowed, dip policy (no fill of points the
`valid` mask or the position spread marks, no more than the max boost into a dip deeper than 10 dB), headroom
compensation; greedy placement on the largest error area, then bounded refinement (coordinate descent or
Levenberg-Marquardt with box constraints) of every fitted band; stops at an RMS error target, the band budget or an
iteration cap. Returns the bands, the predicted response, the RMS error before and after in the range, the gain cut for
headroom, and notes in words. The core quantizes the result with its setters' quantizers and predicts again
(OD-P5c). **Done when:** a simulated room with three modes is flattened to ±1.5 dB in 20 to 300 Hz with at most 5 bands;
a 15 dB narrow null gets at most the max boost; a dip that varies across positions is left alone; no band ever leaves
its bounds or uses a band index not allowed; the error after is never above the error before (else no change is
returned); identical input gives bit-identical output; 10 bands on the standard grid fit in under 200 ms native in
release (iteration cap); `proptest_*`. **Deps:** the seam. **Clients:** P5TUNE, P5LIVE.

**P5XO: Alignment and the crossover search.** [TODO] Size 4. Wave P5-A. **Owns:** `crates/dspi-measure/src/{align,xover}.rs`
(and folders). **Implements:** T2: `align(ways: &[WayMeasurement], options) -> AlignResult` (coarse onset, fine delay
and polarity by the flattest summed magnitude through each transition, delays normalised and within bounds, a
confidence per pair); T10 step 5: `crossover_search(drivers, transitions, target, constraints, model) -> XoProposal`:
driver responses de-embedded from measurements and their chains, candidate corners on a 1/12-octave grid within the
window, families and orders from `dspi_proto::xover::meta` (only types the model can build), levels cut-only, delay and
polarity per candidate, the cost of section 1.4 step 5, bounded candidates (at most 20 000 evaluations a transition);
`predict_sum` for the sum check (step 6). **Done when:** on synthetic drivers (second-order roll-offs and a breakup
peak) of a three-way speaker the corners come within 1/6 octave of the designed optimum, a 0.35 ms offset is recovered
to ±10 µs, an inverted tweeter is flipped, "align only" keeps every corner; a protective high-pass from the constraints
is never lowered and no output gain is ever raised; a model that cannot build a type never gets it proposed; identical
input, identical output; runtime under 2 s native release for a three-way pair; `proptest_*`. **Deps:** the seam.
**Clients:** P5TUNE.

**P5SIM: The simulated world and WAV files.** [TODO] Size 2.5. Wave P5-A. **Owns:** `crates/dspi-measure/src/sim.rs`
(and `sim/`), `crates/dspi-measure/src/wav.rs`, `crates/dspi-measure/testdata/p5/` (goldens other than `cal/`).
**Implements:** behind feature `sim`: drivers (sensitivity, mechanical high-pass, roll-off, breakup peak, acoustic
offset), a speaker as ways per output, a room (modes with decay, one or two reflections), a headphone on each `Coupler`
type with a seal-leak parameter per reseat, a microphone (its own response; a matching calibration text), a noise floor,
the microphone's clock offset in ppm, latency and capture rate; `render(world, stimulus per output, chain per output,
model, device rate) -> capture samples`, deterministic by seed. `wav`: read and write PCM 16, 24, 32-bit and float 32,
mono or a picked channel, at most 64 MiB, typed errors. **Done when:** rendering an impulse through a flat world gives
the impulse; a 50 ppm world drifts by 50 ppm; goldens of three worlds (three-way speaker in a room, two-way near-field,
headphones on the 711 coupler) are written and stable; WAV round trips and `proptest_*` fuzz of the reader. **Deps:**
the seam (its own test stimulus; P5DSP's model is used from P5-B on). **Clients:** every later test; P5DEMO; the web
e2e's fake microphone WAVs.

**P5MDOC: The document model and the contract.** [TODO] Size 2.5. Wave P5-A. **Owns:** new
`crates/dspi-measure/src/doc.rs` (its `pub mod doc;` line in `lib.rs` under `// -- WP P5MDOC --`, the one `lib.rs` edit of
P5-A), new `docs/integrator/measurement.md`; rows in `docs/terminal/preset-format.md` (the three kinds),
`docs/GLOSSARY.md` (Measurement, Generator, Sweep, Impulse response, Target, Microphone calibration, Placement, Coupler,
Way, Auto-tune, Live response, Microphone RTA, Device analyser), `docs/integrator/hub-api.md` (a new section 22
"Measurement documents" and the REST rows of 14.2, marked draft), `docs/PARITY.md` (rows). **Implements:** section
1.6's three kinds as serde structs with `validate()` (bounds: 256 KiB, grids at most `MAX_GRID_POINTS`, 16 outputs, 9
positions, IR excerpt 16384 samples, names 256 characters, notes 4096; non-finite refused; unknown keys kept on read and
written back, as the family's rule); `measurement.md`: the workflow of sections 1.1 to 1.5, the S-rules, the documents
with examples, the hub messages and REST reads (draft), what the core never does. **Done when:** every kind round
trips; each bound refuses one past it; a document of another kind is refused by each parser; `proptest_*` over mutated
documents; the links resolve; the no-host-noun check passes. **Deps:** the seam. **Clients:** P5DOCK, P5SES, P5HUB,
P5TUNE.

### Wave P5-B: the core session, the core documents, the clients' capture

**P5DOCK: Documents in the core.** [TODO] Size 2.5. Wave P5-B. **Owns:** new `crates/dspi-core/src/measure_doc.rs` (and
`measure_doc/`), new `crates/dspi-session/src/measure_store.rs` (and `measure_store/`), the three kind words in
`files.rs`'s `doc_kind` and the refusal in `config_import` under a marker, the `pub mod` lines under markers, binding
wiring for its calls. **Implements:** the local store (a folder per serial for measurements, `mic/` and `target/`
folders, an `index.json` cache rebuilt from the documents, atomic writes, the corrupt-file rule, the same ids rule as
`snapshot_store`); calls `measure_docs(kind, serial?)`, `measure_doc_get(id)`, `measure_doc_put(text)`,
`measure_doc_rename`, `measure_doc_note`, `measure_doc_delete`; `mic_import(name, file_text) -> mic doc` (P5CAL's
parsers), `target_import(name, file_text, use, coupler?)`, `target_builtins() -> Vec<TargetInfo>`,
`measurement_export_rew(id or text, output) -> String`; the `SectionModel` implementation over `curves.rs`'s cascade
(`pub(crate)`, used by every later core WP); on the web the calls take and give document text (no store), as snapshots do.
**Done when:** store tests as P1SN's (torn index, missing documents, ids in two folders refused, a full store); each
import from P5CAL's fixtures; a `config_import` of a measurement is refused with nothing sent; the `SectionModel` agrees
with `compute_magnitude_curve` and `compute_phase_curve` within 0.01 dB / 0.1° on the curves reference table
(`curves_ref.rs`); the same text through C, UniFFI and wasm; `ci/drift.sh` clean. **Deps:** P5CAL, P5MDOC (P5-A).
**Clients:** P5SES, P5HUB, P5TUNE, P5TM, the screens.

**P5SES: The measurement session.** [TODO] Size 5. Wave P5-B. **Owns:** new `crates/dspi-core/src/measure.rs` and
`crates/dspi-core/src/measure/` (`session.rs`, `safety.rs`, `preflight.rs`, `plan.rs`, tests), its `pub mod` line, the
`MeasureListener` in `callbacks.rs` under a marker, the `measure` screen row in `assets/modes/modes.json`, its notice
rows, binding wiring (C, UniFFI, wasm: `crates/dspi-wasm/src/measure.rs`). **Implements:** sections 1.1, 1.2, 1.3 and
1.5: `measure_preflight(setup) -> Preflight {blockers, warnings}`, `measure_start(setup) -> MeasureState`,
`measure_capture_push(samples, rate_hz, clipped)`, `measure_tick()`, `measure_next_position()`, `measure_stop()`,
`measure_state()`, `measure_result() -> document text`; the setup (outputs with roles and speakers, placement, the `mic`
document, stimulus overrides, level ceilings within the hard caps); the device lock with lease renewal; the pre-roll,
the ramp, the walked run, analysis in slices, positions and reseats, the result document (P5MDOC's `measurement` with the
measured chains from the mirror); every S-rule; the generator only through `siggen_set_config` and `siggen_control`.
**Done when:** with P5SIM's world rendered from the session's own generator configuration and pushed as capture, on the
demo device: a three-output run gives responses within ±0.5 dB of the world's truth (100 Hz to 10 kHz) and arrivals
within ±10 µs; S1 to S12 each have a test: preflight refuses `vol.user` -6 dB, a `High` output without a high-pass, input
audio at -40 dBFS, read-only mode, a Viewer through the fake hub (denied), a locked `measure` page, a pending foreign
guard; a ramp stops at the SPL cap with a calibrated microphone and at the ceiling without; a capture clip, an output
peak at 0 dBFS, a foreign preset load, a foreign write on a measured output, a lost lock and a capture that stops each
abort with exactly one STOP_NOW and no other write; no generator configuration ever has `RAW`, an infinite repeat or a
`level_db` above the cap (a property test over setups); the lock is renewed and released; a listener calling back
into `DspiCore` does not deadlock; each tick's work stays under the budget on the 96 kHz case; the wasm build runs a
whole session caller-driven (no thread). **Deps:** P5DSP, P5CAL, P5SIM, P5MDOC (P5-A); P5DOCK's `SectionModel` (same
wave: P5SES uses a test `SectionModel` until P5DOCK merges; the merge swaps one line). **Clients:** P5LIVE, P5TUNE,
P5DEMO, the screens.

**P5WCAP: Web: the microphone.** [TODO] Size 1.5. Wave P5-B. **Owns:** new `apps/web/src/core/capture.ts`,
`apps/web/src/core/capture-worklet.ts`, `test/unit/capture_p5wcap.test.ts`, `test/e2e/capture_p5wcap.spec.ts`, the
Playwright launch flags for a fake microphone in the web CI config under a marker. **Implements:** input device list and
choice (labels after permission), `getUserMedia({audio: {deviceId, echoCancellation: false, noiseSuppression: false,
autoGainControl: false, channelCount: 1}})`, an AudioWorklet that posts `Float32Array` blocks (transferred, about 20 ms
each) with the context's rate and a clip flag to a callback; start, stop, device loss as an event; the secure-context
and permission refusals as notice keys; nothing played (no destination node is ever connected). **Done when:** unit
tests on the block framing; e2e in Chromium (`--use-fake-device-for-media-stream --use-file-for-fake-audio-capture=<a
P5SIM WAV>`) and Firefox (fake media streams): blocks arrive with the WAV's content at the context's rate, a denied
permission is the notice, a stop releases the device (the track ends), no `AudioDestinationNode` is connected (asserted).
**Deps:** none. **Clients:** P5WMS.

**P5QCAP: Qt: the microphone, and the image.** [TODO] Size 2.5. Wave P5-B. **Owns:** `ci/container/Containerfile.qtport`
(Qt Multimedia for Linux and MinGW), `ci/container/images.conf` (QTPORT and COREMERGE tags bumped), new
`apps/desktop/qt/src/platform/AudioCapture.{h,cpp}`, its cmake wp file, the Windows packaging lines for the Multimedia
module and its native audio backend (no FFmpeg plugin bundled), `apps/desktop/qt/smoke/` one capture smoke test.
**Implements:** a `QAudioSource` reader off the UI thread producing mono `f32` blocks with the rate and a clip flag; the
input device list; device loss as a signal; nothing ever played. **Done when:** the images build with
`ci/container/build-images.sh` (podman, nothing installed on the host; if the agent may not build images it reports
BLOCKED and the coordinator builds them before the wave, as earlier bumps); `--step qt`, `qt-offscreen` (a fake
`QAudioDevice`-free path: the reader fed from a WAV through a test seam) and `qt-windows` pass; the licence inventory
lists Qt Multimedia. **Deps:** none. **Clients:** P5QUI.

### Wave P5-C: tune, live, hub, demo

**P5TUNE: Auto-tune.** [TODO] Size 4.5. Wave P5-C. **Owns:** new `crates/dspi-core/src/measure/tune.rs` (and
`measure/tune/`), its `mod` line in `measure.rs` under a marker, its notice rows, binding wiring. **Implements:** section
1.4: `measure_layout_proposal() -> Layout`, `measure_tune_propose(measurement id or text, layout, options) -> TuneProposal
{id, writes (path, indices, value, words), curves (before, predicted), held (what the limits held), warnings}`,
`measure_tune_sum_check(proposal)` (a run through P5SES), `measure_tune_apply(proposal id, guard_s) -> GuardedApply`
(the "before" snapshot, the guard, the undo group, `write_param` per write), `measure_tune_verify()`,
`measure_tune_keep()` (`apply_confirm`, the "after" snapshot with its `tune` block, the measurement's `tunes` row) and
`measure_tune_ab()` (`ab_start` or `ab_start_documents`); proposals pre-judged against the registry, the limits profile
and the platform (S9); a proposal for another serial refused; a measurement whose chains differ from the device's is
re-predicted against the current chains (OD-P5o), a measurement older than 30 days or of another placement warned.
**Done when:** with P5SIM's three-way stereo world on the demo device: the full flow from layout to keep, the predicted
and the verified sums within 1.5 dB RMS 200 Hz to 10 kHz, the tweeter's simulated 0.3 ms offset and an inverted mid
corrected, the room's bass modes reduced within ±2 dB 30 to 300 Hz; every write in the history as one "Auto-tune" step,
one undo puts every value back; Revert and the guard's timeout restore the "before" state byte for byte (the bulk image
compared); a limits profile with a 300 Hz high-pass minimum and a 3 dB boost cap is never exceeded and the proposal says
what it held; a read-only client and a Viewer get no proposal applied (`ReadOnly`, `Denied`) with nothing sent; the A/B
toggles between the two snapshots; deterministic proposal golden. **Deps:** P5SES, P5DOCK (P5-B); P5FIT, P5XO (P5-A).
**Clients:** P5WAT, P5QUI, P5TM.

**P5LIVE: Live response, direct adjustments, RTA hold and compare.** [TODO] Size 2.5. Wave P5-C. **Owns:** new
`crates/dspi-core/src/measure/live.rs` (and `measure/live/`), its `mod` line under a marker, its notice rows, binding
wiring. **Implements:** T9 and T5: `measure_live_start(output(s), mode: sweeps | pink_rta, smoothing)` (the session's
safety and lock; sweeps of 2.7 s, one output, magnitude-only; pink noise re-armed every 30 s, S7),
`measure_live_frame() -> LiveFrame {measured, target, predicted?, rta?}`, `measure_live_predict(pending writes)` (the
predicted curve of an edit before it is sent), `measure_live_cut_peak(output, freq_hz)` (one band from P5FIT on a
one-octave span, free band only, previewed, then one undo group), `measure_live_fit_range(output, f1, f2)` (a preview,
then one undo group); `rta_hold()`, `rta_compare()` for both the microphone RTA and the device analyser; averaging
presets. **Done when:** on the simulated world: a live sweep frame per sweep; an EQ edit by the user shows its predicted
curve at once and the measured one after the next sweep, within 1 dB; "cut this peak" on a simulated 6 dB peak gives
one band and one undo step; pink-noise RTA reads the world's response within 1 dB (1/3 octave); stopping stops the
generator; the device lock is held while live and released after; no write without preview. **Deps:** P5SES (P5-B),
P5FIT (P5-A). **Clients:** P5WMS, P5QUI.

**P5HUB: Measurement documents on the hub.** [TODO] Size 3. Wave P5-C. **Owns:** new `crates/dspi-hub/src/measurements.rs`
(and `measurements/`), its registration line in `service.rs` and `ServiceConfig::measure` under markers, the
`measurements/` block in `diag.rs` (the store's index only: names, kinds, serials and sizes, to keep the bundle small;
documents hold no secret and no audio), the REST rows in `rest.rs`'s `route` under a marker, new
`crates/dspi-core/src/hub_server/measure.rs` (core calls) with its `pub mod` line, binding wiring.
**Implements:** cap `hub_measure`; `measure.doc.list {kind, serial?}` (every role), `measure.doc.get {doc}` (every role),
`measure.doc.put {document}` (Control, Admin), `measure.doc.rename`, `measure.doc.note` (Control, Admin),
`measure.doc.delete` (Admin), event `measure.doc.changed {v, hub, kind, serial?, doc, change}`; bounds 256 KiB a
document, 100 measurements per device, 32 microphones, 64 targets, 64 MiB in all; the snapshot store's rules (the hub's
copy wins, never an overwrite: `conflict`); no new error code, no hub-own write; REST `GET /api/v1/devices/{serial}/measurements`,
`GET /api/v1/measurements/{id}`, `GET /api/v1/mics`, `GET /api/v1/targets` (additive within frozen v1; no `PUT` or
`POST`: `405` with `Allow`); core calls `hub_measure_docs`, `hub_measure_doc_get/put/rename/note/delete` and the merged
list in `measure_docs` (local and hub, the hub's winning), `Unsupported` without the cap and nothing sent. **Done when:**
tests through the real hub in-process: each message by role, the bounds, a conflict, the event, the corrupt-file rule,
REST conformance (a Viewer key reads, a `POST` is `405`), with and without TLS (`DSPI_HUB_TEST_TLS=on` and `compat`);
the core calls against an upstream-like hub without the cap are `Unsupported`; the merge prefers the hub's copy.
**Deps:** P5DOCK (P5-B), P5MDOC. **Clients:** the web and Qt document lists.

**P5DEMO: The simulated room in the demo.** [TODO] Size 1. Wave P5-C. **Owns:** new `crates/dspi-core/src/measure_sim.rs`
(feature `measure-sim`, its line in `crates/dspi-core/Cargo.toml` under a marker), the feature in the demo builds of
`crates/dspi-wasm/Cargo.toml` and the Terminal's and Qt's demo mode under markers. **Implements:** on a demo connection
only, a `SimCapture` that renders P5SIM's "three-way speaker in a room" world from the generator configuration the
session started and feeds `measure_capture_push`, labelled "Simulated microphone" everywhere; never on a real device
(the connection kind is checked; a test proves it). **Done when:** a demo-device session through each binding produces
a measurement without a microphone; a real (fake-USB-kind) connection refuses the simulated microphone. **Deps:** P5SES,
P5SIM. **Clients:** the screens' e2e, the user's demo mode.

### Wave P5-D: screens and the Terminal

**P5WMS: Web: measure, results, live.** [TODO] Size 3. Wave P5-D. **Owns:** new `apps/web/src/screens/measure/`
(`MeasureScreen.tsx`, `setup.tsx`, `run.tsx`, `results.tsx`, `live.tsx`, `graph.tsx`, `model.ts`), the Worker's
routing of capture blocks to `measure_capture_push` in `apps/web/src/core/worker.ts` under a marker, the TypeScript
records in `apps/web/src/core/measure-protocol.ts` (new), one marked line in `apps/web/src/ui/screens.tsx`,
`test/unit/measure_p5wms.test.ts`, `test/e2e/measure_p5wms.spec.ts`. **Implements:** setup (microphone and its
calibration import, placement, coupler, outputs and roles), preflight in words, the ramp and the run with progress and a
Stop that is always visible, results (measured, target, predicted; smoothing; positions and spread; untrusted points
faint), the live view with direct adjustments, the microphone RTA with hold and compare; documents saved to the hub (or
downloaded as text without one). **Done when:** e2e on the demo device with the simulated microphone (P5DEMO) and with
the fake media WAV (P5WCAP): a measurement end to end, the curves drawn, a "cut this peak" undoable, Stop stops the
generator, a Viewer through the hub sees the passive RTA only, 375 px. **Deps:** P5SES, P5LIVE, P5DEMO, P5WCAP.

**P5WAT: Web: auto-tune and documents.** [TODO] Size 2.5. Wave P5-D. **Owns:** new
`apps/web/src/screens/measure/{tune.tsx,docs.tsx,tune-model.ts}`, `test/unit/measure_p5wat.test.ts`,
`test/e2e/measure_p5wat.spec.ts`. **Implements:** the tune wizard of section 1.4 (layout, preflight, measure, each step's
result, the proposal as words with the held list, apply with the guard's countdown, verify, keep or revert, A/B);
the documents list (local and hub merged, rename, note, delete by role), the microphone and target libraries, REW
export. **Done when:** e2e on the demo device with the simulated three-way world: a tune proposed, applied, verified and
kept; another reverted; the A/B toggles; Control cannot delete a hub document; 375 px. **Deps:** P5TUNE, P5HUB, P5WMS's
`graph.tsx` (same wave: P5WAT imports it; the merge order is P5WMS first).

**P5QUI: Qt: measurement, live, auto-tune, documents.** [TODO] Size 4.5. Wave P5-D. **Owns:** new
`apps/desktop/qt/src/controllers/MeasureController.{h,cpp}`, `TuneController.{h,cpp}`, `apps/desktop/qt/qml/measure/`,
the registrar lines under markers, `apps/desktop/qt/smoke/SmokeMeasure.cpp`, its cmake wp file. **Implements:** the
web's P5WMS and P5WAT on Qt over the C ABI, capture through P5QCAP's `AudioCapture`, the tick from a timer on a worker
thread, the graph on the existing graph renderer (target dashed, predicted dotted). **Done when:** offscreen smoke on the
demo device with the simulated microphone: measure, live adjust and undo, tune applied and reverted; `qt-windows`
builds; translations extracted (`--step qt-ts`). **Deps:** P5QCAP (P5-B), P5TUNE, P5LIVE, P5HUB, P5DEMO (P5-C).

**P5TM: Terminal.** [TODO] Size 1.5. Wave P5-D. **Owns:** new `apps/terminal/src/measure.rs` (its `mod` line below TM15's
macro block), help text. **Implements:** `uconsole measure mic import FILE`, `measure target import FILE`, `measure
docs [--kind]`, `measure show ID`, `measure export ID --output N` (REW text), `measure analyse WAV --config DOC|--sweep
...` (offline: a capture recorded elsewhere with the generator settings named, through P5DSP and P5SES's analysis
path), `measure tune ID [--dry-run] [--guard S]` (the proposal printed in words; applied guarded with a Keep prompt),
`--demo` with the simulated microphone. No live capture in the Terminal (OD-P5m). **Done when:** command tests against
the demo device: a mic imported and listed, a P5SIM WAV analysed to the golden response, a tune printed and applied and
reverted, exit codes for `ReadOnly`, `Denied` and `LimitExceeded`. **Deps:** P5DOCK, P5SES, P5TUNE, P5DEMO.

### Wave P5-E: close-out

**P5FIN: Docs built and the manual.** [TODO] Size 0.75. Wave P5-E. **Owns:** the built marks in
`docs/integrator/measurement.md` and `hub-api.md` section 22, a user manual page `docs/manual/measurement.md` (placing
the microphone, the couplers, what the safety rules mean, what auto-tune changes and how to go back), `docs/PARITY.md`
marks, this plan's `[DONE]` marks. **Done when:** links resolve; the no-host-noun check passes. **Deps:** P5-D merged.

**P5HIL (the coordinator with the owner, not an agent WP).** Size 1. After P5-D. One real session on the K340 PCB unit
(`BB66A44F2AC1E7B6`) with the owner's measurement microphone and its calibration file, in the web console through
`uconsole web` and in Qt: (1) the calibration file imported and its sensitivity checked against the vendor's sheet;
(2) the safety paths first, with the generator at -120 dBFS: Stop, microphone unplugged, a preset load from another
console, each aborting with one STOP_NOW; (3) the ramp from the start level with the owner listening, the ceiling of
**headphone mode applied to every K340 output** unless the owner says which outputs carry the attached headphones
(memory `test-volume-limit`: the headphones stay attached); (4) a measurement per placement the owner can set up
(headphones on a coupler, or speakers on the amplifier outputs); (5) with speakers, a two- or three-way auto-tune kept
and then reverted; (6) the captures exported as WAV into `work/p5hil/` and, with the owner's consent (room sound), two
promoted to replay goldens in `crates/dspi-core/testdata/p5/hil/`. Recorded in `work/p5hil/REPORT.md`. **Owner action
required** (section 6).

### Later batch (not in phase 5's waves; owner's order: web and Qt first)

| WP | Size | What |
|---|---|---|
| P5AN: Android | 4.5 | `AudioRecord` with `UNPROCESSED` (USB microphones through Android's USB audio), the screens in Compose over UniFFI, the simulated microphone in demo |
| P5SW: Swift | 5 | `AVAudioEngine` input on macOS and iOS (USB microphones through the iPad's and Mac's audio), SwiftUI screens, on the Mac VM |

### Firmware-dependent items (`bofr-main`, extension opcode 0x00)

**Nothing in phase 5 needs new firmware.** The generator is upstream (v1.1.5 and later) and on `bofr-main`; the
extension channel is not used.

| Item | Firmware | WP | On stock firmware |
|---|---|---|---|
| Stimulus | the generator, vendor opcodes 0xA4 to 0xA8, `NOTIFY_EVT_SIGGEN_STATE` | P5SES | the same (upstream since v1.1.5); before v1.1.5: no generator, so passive microphone RTA and offline analysis only, and the screens say why |
| Abort on clip | `fetch_status` peaks and clip flags; 0xE0 clip events (`EXT_NOTIFY_FEATURES` bit 1) | P5SES | peaks at the clients' poll rate (S4 uses the peak, which every firmware has) |
| Delay range | `Platform::max_delay_ms` 21 (RP2040) or 42 ms (RP2350) | P5XO, P5TUNE | the same |
| Crossover types | the type table `dspi_proto::xover::meta` | P5XO | the same |

Considered and **not planned**: a generator time stamp or start-at-frame (`EXT_FRAME_COUNTER`/`EXT_APPLY_AT_FRAME` exist
only on alink USB-direct builds, and the cycle grid makes a stamp unnecessary); a firmware ceiling on the generator's
level or a `RAW` that keeps a protective high-pass (a protection against other consoles, worth proposing in a later
firmware batch, not needed by this phase, which never uses `RAW`); more PEQ bands (FW7, upstream, unknown cost; the fit
uses free bands); a microphone into the DSPi (an I2S ADC) for the device analyser (R2 chose a host microphone).

### Waves

No two WPs in one wave own the same file, and no WP depends on another WP of its own wave except where an entry says
how the seam covers it (P5SES on P5DOCK's `SectionModel`; P5WAT on P5WMS's graph, merged first). Registration lines,
`mod` lines, binding wiring, `screens.tsx` and the Qt registrar are additive under markers. `diag.rs` is edited once
(P5HUB, wave C). `notices.json` takes one block per WP.

| Wave | WPs (parallel) | Size | Needs |
|---|---|---|---|
| P5-A | P5DSP 5, P5CAL 3, P5FIT 4, P5XO 4, P5SIM 2.5, P5MDOC 2.5 | 21 | `main` with this plan's seam (`crates/dspi-measure`) |
| P5-B | P5DOCK 2.5, P5SES 5, P5WCAP 1.5, P5QCAP 2.5 | 11.5 | P5-A; the coordinator builds the bumped Qt images before or with P5QCAP |
| P5-C | P5TUNE 4.5, P5LIVE 2.5, P5HUB 3, P5DEMO 1 | 11 | P5-B; then `ci/macvm.sh macos` once for the regenerated Apple bindings |
| P5-D | P5WMS 3, P5WAT 2.5, P5QUI 4.5, P5TM 1.5 | 11.5 | P5-C |
| P5-E | P5FIN 0.75; P5HIL (coordinator and owner, 1) | 0.75 (+1) | P5-D |
| | **Total: 19 WPs** | **55.75** (+1 P5HIL) | maths 18.5 (P5-A less P5MDOC), core 15.5, hub 3, web 7, Qt 7, Terminal 1.5, docs 3.25; later batch 9.5 |

Each wave ends with one Fable batch review; the full gate runs after P5-C (core and hub complete) and after P5-D. P5-A
is the widest; if fewer agents run, P5DSP and P5MDOC go first (they gate P5SES and P5DOCK). P5QCAP's image bump can run
any time before P5QUI and should be built early, since every later Qt and Rust build uses the new images.

---

## 3. The security bar and the compatibility policy

**PLAN-SEC applies unchanged.** The new hub family is on `/dspi/v1` over SECSRV's TLS with pinning, REST behind API keys
with `Origin` refused; the web measures through the hub-served https page or the `uconsole web` proxy (a secure context,
which `getUserMedia` requires). On top of that:
- **Test audio cannot reach another output** (S1): there is no host player in any client; the generator is inside the
  DSPi. This is the owner's "guarded player only" rule met by having no player.
- **Levels are bounded in three places:** the preflight (`vol.user` at most -12 dB, the generator ceiling per mode, the
  hard caps), the ramp (closed loop on the microphone, the SPL cap), and the aborts (clip, SPL, anything foreign). The
  session writes no volume.
- **Roles:** playing the generator is Control or Admin through a hub (the existing policy, rows 156 and 158); a Viewer
  gets the passive RTA. `measure.doc.put/rename/note` Control and Admin, `delete` Admin, reads every role.
- **Every write is judged:** a tune's writes go through `write_param`, so the client read-only mode, a read-only
  connection, the installer lock, the limits profile and the device lock decide each one; the proposal is pre-judged so
  nothing refused is offered.
- **Untrusted input:** calibration and target files, documents from the hub or another client, WAV files and captured
  audio are data: bounded, parsed without panics, fuzzed (`proptest_*` in every parser WP). A document never carries
  an instruction the core acts on: a measurement proposes, a person applies.
- **Privacy** (S12): no raw audio leaves the core's memory unless the user exports a WAV; nothing audio in logs,
  events, the diagnostics bundle or the hub; documents hold derived curves only. The browser's microphone permission is
  asked only when the user starts a measurement or the RTA.
- **No new error code, no hub-own write**, bounded stores (256 KiB, 100 measurements a device, 32 microphones, 64
  targets, 64 MiB), files 0600 and atomic, the corrupt-file rule.

**The compatibility policy (owner, 2026-11-15).**
- **No shims between our components:** three new `meta.kind`s in the existing family; the hub family is new under its
  cap; a core without the call fails the build, not the run.
- **Our clients against an upstream Weeb Labs hub keep working:** every `hub_measure_*` call is `Unsupported` without the
  cap and sends nothing; documents then stay local; the measurement itself needs only the generator opcodes, which an
  upstream hub forwards by its policy (class `control`).
- **USB to the hardware keeps working:** nothing on the wire changes; the generator calls are the existing ones.
- **Our hub against upstream clients, best effort:** the new messages are unknown to them and ignored.

---

## 4. Testable without hardware, and what needs the owner

| What | How it is tested | WP |
|---|---|---|
| Sweep model, deconvolution, drift, averaging, windows, smoothing, RTA | synthetic signals with known answers; an independent re-implementation of `siggen.c`'s recurrence; `proptest_*` | P5DSP |
| Calibration, targets, positions | hand-written fixtures per format; fuzzing | P5CAL |
| Fit, alignment, crossover | synthetic drivers and rooms with designed optima; bounds as properties; determinism goldens | P5FIT, P5XO |
| The whole run, every S-rule | the demo device (it implements the generator) plus P5SIM's world fed as capture; the fake hub for roles and locks | P5SES, P5TUNE, P5LIVE |
| Documents and stores | store tests as P1SN's; the real hub in-process | P5DOCK, P5HUB |
| Browser microphone | Chromium's and Firefox's fake media devices with P5SIM WAVs; no destination node connected | P5WCAP, P5WMS, P5WAT |
| Qt microphone | the reader fed from a WAV through a test seam; offscreen smoke | P5QCAP, P5QUI |
| **Real acoustics, a real microphone's calibration, real levels in a room or on a coupler, the K340** | **P5HIL only**: the coordinator with the owner present | P5HIL |

What the simulator cannot prove and P5HIL must: that the vendor's sensitivity convention gives the right dB SPL with the
owner's microphone; that the browsers' and Qt's capture really has processing off on the owner's machines; that the
drift estimate holds with a real USB microphone (typically tens of ppm); that the ramp's start levels are quiet enough
in a real room and on a coupler; that a tune sounds better (A/B by ear, the owner's judgement).

---

## 5. Decisions

Every decision carries a firm recommendation; following the owner's rule of 2026-11-15 they are taken as recommended
unless the owner objects. Section 7 lists what remains for the owner (actions, no blocking question).

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-P5a** | **The stimulus** | (1) the DSPi's generator only; (2) host playback through the DSPi's USB audio; (3) both | **(1)** | The generator is on stock firmware since v1.1.5, sample-exact and repeatable (per-cycle reset), injected before the output's processing so the measured path is the real one, runs without a host stream, and cannot fall back to another output: the owner's test-audio rule is met by construction. (2) needs a guarded player with a watchdog on five platforms and is exactly what once played on the owner's speakers |
| **OD-P5b** | **The timing reference and clock drift** | (1) the generator's cycle grid in one walked, repeated run; drift from the arrivals' line fit; no absolute latency; (2) a loopback channel (a debug firmware build only); (3) an acoustic reference speaker before each sweep (REW's way); (4) a firmware time stamp | **(1)** | The walk puts all ways of a speaker in one capture on one device clock; the cycle period is exact in device samples, so arrivals give both the clock ratio and every way's relative delay. Relative delay and phase are what alignment and crossover design need. (2) does not exist on release firmware; (3) needs two simultaneous signals the generator does not make; (4) needs new firmware for nothing |
| **OD-P5c** | **Where the maths lives** | (1) a new pure crate `dspi-measure` (depends on `dspi-proto` only), filter responses through a `SectionModel` the core implements with its own cascade; (2) modules inside `dspi-core`; (3) inside `dspi-session` | **(1)** | Pure, property-tested and fuzzed in isolation, built for wasm on its own, usable by the core, the Terminal and later the hub without pulling the device stack; the `SectionModel` keeps one filter model (the graph's) so a prediction can never disagree with the curve the user sees |
| **OD-P5d** | **The FFT** | (1) `realfft` over `rustfft` (MIT or Apache-2.0, pure Rust, wasm-ready); (2) our own radix-2 | **(1)** | Mature and fast at the 2^19 to 2^21 sizes a sweep needs; licences already allowed by `deny.toml`; writing and verifying our own is cost without benefit |
| **OD-P5e** | **The capture seam** | (1) sans-IO: the client pushes microphone blocks, the core's state says when it wants them, `measure_tick` drives time; (2) a `Capture` trait the core calls per platform; (3) audio in the core per platform | **(1)** | The wasm core cannot open a microphone; the RTA's caller-driven tick already works this way on the web; (1) keeps the core free of audio APIs and threads, and every client's part is a small reader |
| **OD-P5f** | **Where a measurement runs for the web and the fleet** | (1) where the microphone is: the browser's core (wasm) or the desktop app; the hub stores documents and arbitrates through the device lock; (2) a hub-side measurement with the microphone on the hub's machine | **(1)** | The microphone belongs at the listening position, not at the rack; the web's core is the core, so the web loses nothing; the hub adds no write and no audio stack. A hub-side capture can be added later as one more capture source if asked |
| **OD-P5g** | **The documents** | (1) kinds `measurement`, `mic`, `target` in the `.dspipreset` schema-1 family, derived curves on a 1/96-octave grid plus a 16384-sample IR excerpt, 256 KiB; the tune's result as two snapshots; (2) a new format; (3) raw captures in the documents | **(1)** | PRO1's rule (one family, `meta.kind`); the hub's snapshot store's bound fits; the snapshots make A/B, recall and the hub's store work unchanged; raw audio is private and large (S12) |
| **OD-P5h** | **How a tune is applied** | (1) typed writes through `write_param` in one undo group inside a guarded apply, between a "before" and an "after" snapshot; (2) a whole bulk image recalled as a snapshot | **(1)** | Each write is judged by limits, read-only and the lock; one undo reverts it; the guard reverts on timeout; the snapshots give A/B. (2) ends the undo history and is judged only as a whole image |
| **OD-P5i** | **Speaker safety** | (1) the S-rules of section 1.2: start -50/-60 dBFS, ceilings -12/-30 (hard -6/-20), SPL caps 95/90 (hard 105/100), `vol.user` at most -12 dB, never `RAW`, a protective high-pass required on `Mid` and `High`, abort on clip, SPL or anything foreign, finite runs, the lock lease; (2) user-set levels only | **(1)** | R5 asks for exactly this; the numbers are conservative defaults the user can lower and raise only to hard caps; de-embedding gives driver data without ever bypassing a crossover |
| **OD-P5j** | **Delay and polarity** | (1) delays normalised so the latest way gets 0 (only additions), within the platform's and the profile's range; polarity as the invert of every crosspoint feeding the output; (2) delays relative to the current values | **(1)** | Minimal total latency and no negative delay; the matrix invert is the device's only polarity control |
| **OD-P5k** | **Crossover tuning's reach** | (1) corners within half an octave of the current ones and the same family by default, wider and other families by choice; levels cut only; "align only" available; (2) a free search from scratch | **(1)** | A tune refines what the installer built and the driver allows; a free search can move a tweeter's corner far from its design and is offered only when asked |
| **OD-P5l** | **The EQ fit's policy** | (1) free bands only by default; speakers 20 to 300 Hz by default, headphones 20 Hz to 10 kHz; boost at most +3/+6 dB and the profile's cap; no filling of variable or deep dips; headroom by an equal output-gain cut; (2) all 10 bands, full range | **(1)** | Never overwrite a user's band silently; above the Schroeder region a single-position correction usually sounds worse; boosting nulls wastes headroom and does not work |
| **OD-P5m** | **Which clients first** | (1) web and Qt screens in this phase, the Terminal offline (documents, WAV analysis, the tune from a document, the demo), Android and Swift a later batch; (2) every client now | **(1)** | The owner's order ("web and Qt first"); the Terminal has no portable audio input and its users can record elsewhere; every client gets the same calls, so the later batch is screens and a reader each |
| **OD-P5n** | **Concurrency** | (1) the device lock "Measuring" with a 30 s lease renewed by `measure_tick`; foreign writes or generator stops abort the run; one session per core; (2) no lock | **(1)** | Another console changing EQ mid-sweep spoils the measurement and may confuse the safety reasoning; the lease is also the dead-man switch |
| **OD-P5o** | **A measurement older than the device's state** | (1) each output's measured chain is stored; the tune de-embeds with it and predicts against the current chain; age and placement only warn; (2) refuse when the chains changed | **(1)** | The acoustics did not change when an EQ did; de-embedding makes the measurement reusable, and the verify run catches a moved speaker |
| **OD-P5p** | **Targets shipped** | (1) only targets defined by parameters in our own code, named ours; any other imported from a file; (2) bundle published third-party curve data | **(1)** | No licence question to answer; the importer covers every curve a user already has |
| **OD-P5q** | **The demo's simulated microphone** | (1) the demo device gets P5SIM's world, labelled "Simulated microphone", demo connections only; (2) tests only | **(1)** | The feature can be tried and every screen's e2e runs without hardware, and the label makes it unmistakable |
| **OD-P5r** | **REST for measurements** | (1) reads of documents only, within frozen v1; (2) a measurement started over REST | **(1)** | A measurement needs a microphone and a person; integrators can read the results |
| **OD-P5s** | **Qt's audio input** | (1) Qt Multimedia `QAudioSource` (an image bump, native backends, no FFmpeg plugin); (2) ALSA/PipeWire and WASAPI by hand | **(1)** | One reader for Linux and Windows; the image bump is a one-time cost, built by the coordinator |

---

## 6. What not to do in phase 5

- Play test audio from any host, or build a player "for old firmware"; use `SIGGEN_FLAG_RAW`; stage an infinite generator
  run; write a volume, a mute or a gain while measuring; raise any output gain in a proposal.
- Bypass a protective high-pass to measure a driver (de-embed instead), or propose a high-pass below the S6 rule.
- Write anything as a side effect of a measurement; apply a tune without a preview, the "before" snapshot and the guard;
  apply a tune automatically after a measurement.
- Keep, log, upload or bundle raw microphone audio; put audio in a document.
- Open an audio device in the core, start a thread for measuring, or block a UI thread on analysis.
- Add a new file format, a new error code, a hub-own write, or a measurement started over REST.
- Bundle third-party target or calibration data.
- Build measurement screens in Android or Swift in this phase, or a microphone on the hub's machine.
- Add firmware for this phase.
- Open a pull request to Weeb Labs for anything in this phase.

---

## 7. Owner questions

**None blocking.** Every decision in section 5 has a firm recommendation.

**Owner actions (not questions), needed for P5HIL only, after P5-D:**
1. **The microphone and its calibration file**: which measurement microphone (e.g. a UMIK-1) and its file, so P5HIL can
   check the sensitivity convention with a real file.
2. **The setup**: headphones on which coupler (IEC 60318-4, flat-plate, HATS), and/or speakers on the K340's amplifier
   outputs (which outputs, how many ways), and which outputs carry the headphones that stay attached.
3. **A go for test audio on the K340** under section 1.2's rules, with the owner present for the ramp. The standing go
   of 2026-11-14 covers flashing alink test images only.
4. **Consent for two HIL captures** (room sound) to become replay goldens in the repository, or not.
