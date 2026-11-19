# PRO phase 5: measurement and auto-tune

*WP PLANP5, 2026-11-16, written on `main` at `462fcde1` (phase 3 merged through P3-C and WBCLIP). The spec is PRO1's
report (`work/pro1/REPORT.md` section 2.6 rows T1 to T5, section 4 "Phase 5", section 5, section 6 "What not to do",
question 9) and **the owner's requirements of 2026-11-16, which are binding** (R1 to R5 below). Phases 1 to 3 are merged;
this plan is written against that code, cited `file:line`. Plan plus one compiling seam: the crate skeleton
`crates/dspi-measure` (its shared types in `src/types.rs`, its dependencies, an empty file per module so wave P5-A needs
no shared edit, its `deny.toml` row), committed with this plan. No other code.*

*Revised after the Fable plan review (`work/review-planp5/REPORT.md`, ACCEPT WITH CHANGES): every required change RC1 to
RC12 is applied (each is cited where it lands, "RC5c" and so on), the review's simplifications are moved to section 8
("Later"), and its optional suggestions S-a, S-b, S-c, S-d, S-e, S-f, S-h and the web capture settings are adopted. The
waves are re-sized (section 2, "Waves").*

**Goal.** Tune a DSPi with a calibrated measurement microphone instead of by ear, in every client from one
implementation: measure what each output really does in the room (or on a headphone coupler), show it against a target
live, adjust directly from the measured curve, and let the system tune a multi-way speaker by itself (time alignment,
polarity, crossover corner and levels, then a target EQ fit), with every change previewed, bounded, guarded and
reversible. **The core owns the workflow and the maths** (a new pure crate `dspi-measure` for the maths, a sans-IO
`measure` module in `dspi-core` for the workflow); a client contributes only its microphone's samples and its screens.
**The stimulus is the DSPi's own signal generator**, so no host ever plays test audio; the microphone is a host audio
input. Results are documents of the `.dspipreset` schema-1 family, filed locally or on the hub; applying a tune is the
guarded apply, the undo group and the A/B compare that exist. Web and Qt get the screens first, the Terminal gets the
documents and the apply path, Android and Swift come in a later batch.

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
- **Bias toward progress (owner):** the first version is the smallest one that meets R1 to R5 soundly; what the review
  judged optional is listed in section 8, not dropped.
- **Process:** parallel Opus WPs in waves of disjoint owned files, one Fable review per wave, nits folded into later WPs;
  toolchains in containers, no tmpfs, no devices in a WP; the K340 and the microphone stay with the coordinator and the
  owner (P5SPIKE, P5HIL). Swift work on the Mac VM is slow, so Apple WPs are few and separate: this phase has none.
- **Naming:** "measurement" is a run and its document; "generator" is the DSPi's test signal generator (`siggen`);
  "RTA" alone is ambiguous from now on: the **device analyser** is the firmware's (`rta.rs`), the **microphone RTA** is
  the host's (P5RESP). "alink"/"dlink" as before; "Snapshot" is the host whole-device state.

**What exists today (checked in the code on `462fcde1` and `bofr-main` at `8ba8b72`):**

| Phase-5 need | State | Builds on |
|---|---|---|
| A stimulus | **The firmware has a test signal generator**, upstream since v1.1.5 (`~/prog/DSPi` `dfc9fb3`, 2026-07-05) and on `bofr-main`, RP2040 and RP2350 alike (`Documentation/Features/test_signals_spec.md`). 15 types incl. log sweep (4) and pink noise (3, **normalised to 1/8 of the peak**, `siggen.c:340`, so about 15 dB below a sweep's RMS at the same `level_db`); `level_db` is a **peak dBFS, -120..0** (`siggen.c:1085-1088`); finite or infinite `repeat`; `SIGGEN_FLAG_WALK` plays the masked outputs one per cycle and `repeat` counts passes (`siggen.c:1150-1156`, `:689`); **per-cycle synth state is reset, so repeated sweeps are bit-identical** (`:554-562`); 5 ms fades; no gap after the last cycle (`:685`); `invert_mask` for correlated signals; `SIGGEN_FLAG_RAW` bypasses crossover and PEQ; a preset load stops it; `NOTIFY_EVT_SIGGEN_STATE` 0x07 on every start and stop; it runs with no host stream (the pump, `audio_pipeline.c:1951`) | the whole signal path (section 1.1) |
| Where it is injected | **The generator replaces the matrix mix** on the outputs it plays (`audio_pipeline.c:503-536`): crosspoint gain and crosspoint polarity never act on it. After the injection come the output's crossover and PEQ, and on `bofr-main` also **output crossfeed** (`:561`, `:594-598`), **subharmonic synth** (`:603`), **psychoacoustic bass** (`:617`), **tube** (`:627`), **volume-keyed loudness** (`:669`) and the **lookahead limiter** (`:689`, 32 samples, `limiter.h:26`), then delay, trim, master and user volume and mute (`:351`) | the measured chain (RC1) and the preflight's refusals (RC2) |
| Generator in the core | `siggen_fetch_caps` (`crates/dspi-core/src/stats.rs:255`), `siggen_get_config` (`:320`), `siggen_set_config` on the `Writable`, journalled history-only (`:344`), `siggen_control` (`:370`), `siggen_get_status` (`:397`); the read-only mode refuses it (`read_only.rs`); the demo device implements it with timing (`crates/dspi-transport/src/fake/analysis.rs:17-20`) | P5SES drives it; nothing new on the wire |
| Generator over the hub | `SIGGEN_SET_CONFIG` 0xA4 and `SIGGEN_CONTROL` 0xA6 are class `control` (`tests/fixtures/link/policy/commands.json`), so Control and Admin may play it, Viewer may not | the web measures through the hub as the caller |
| The device analyser | `rta.rs` (C9), `dspi_session::rta`, `rta_sharing` (CG19); web `screens/analysis/SpectrumStrip.tsx`, Qt `RtaController` | unchanged in phase 5 |
| Generator and analyser screens | web `apps/web/src/screens/analysis/ToolsScreen.tsx`, `siggen.ts`; Qt `SiggenController`, `RtaController` | the measurement screens are new, beside them |
| Filter maths | `dspi_proto::dsp` (`coefficients` `:75`, `response_at` `:260`, `phase_at` `:818`), `dspi_proto::xover` (`sections` `:197`, `meta` `:80`); the core's cascade with the Linkwitz Transform (`crates/dspi-core/src/curves.rs:83`, `:97`), `curve_sections` (`:433`), the setters' quantizers (`:388-415`) | candidates are evaluated with the core's own cascade through `SectionModel` (OD-P5c) |
| Typed writes | `set_filter` (`params.rs:88`), `set_crossover` (`:120`), `set_matrix_route` with `invert` (`:660`), `set_output_gain` (`:721`), `set_output_delay` (`:760`), `write_param` (`write_param.rs:171`); delay is 0..`Platform::max_delay_ms` (21 or 42 ms, `registry.rs:906-911`); `vol.user` -60..0 dB (`registry.rs:506-514`) | a tune is a list of these writes (OD-P5h) |
| Undo, guard, A/B, snapshots | undo groups `begin_group`/`end_group` (`journal.rs:1087`, `:1116`); `apply_guard_begin` (`guarded.rs:476`), `apply_confirm` (`:485`), `apply_revert` (`:520`), hub-side guards (P1HA, `crates/dspi-hub/src/apply_guard.rs`); `ab_start` (`ab.rs:396`), `ab_start_documents` (`:410`); `snapshot_capture_document` (`snapshot.rs:433`), the stores (`dspi_session::snapshot_store`, the hub's `snapshots.rs`, 256 KiB a document) | the apply path of every tune and every direct adjustment |
| Documents | `.dspipreset` schema 1, `meta.kind` read by `doc_kind` (`crates/dspi-core/src/files.rs:2027-2031`): `preset`, `snapshot`, `template`, `limits` | three more kinds: `measurement`, `mic`, `target` (OD-P5g) |
| Limits | `OutputLimits` (`crates/dspi-session/src/limits.rs:167`: `max_gain_db`, `highpass_min_hz`, `highpass_min_slope_db`, `delay_min_ms`, `delay_max_ms`, `limiter_ceiling_db`), the profile (`:195`: `max_peq_boost_db`, `master_volume_max_db`, `locked_pages`); every typed write is judged (`crates/dspi-core/src/limits.rs`) | caps of the fit, the crossover search and the delays; the tweeter rule (S6) |
| Device lock | `device_lock_acquire(reason, timeout_ms)`, lease at most 60 s (`presets.rs:482`, `:77`), `device_lock_release` (`:507`); over DSPi Link other sessions get `locked` | a measurement holds it (section 1.5) |
| Meters, clip, notifications | `fetch_status` (`connect.rs:239`): `peaks`, `clip_flags`, hold and max, polled by every client; 0xE0 clip events on `bofr-main` (`EXT_NOTIFY_FEATURES` bit 1); `NOTIFY_EVT_MASTER_VOLUME` 0x01, `PARAM_CHANGED` 0x02, `INPUT_FORMAT` 0x05 (`crates/dspi-proto/firmware/notify.h:29-46`) | the silence check and the aborts without a new read |
| Modes and page locks | `assets/modes/modes.json` screens; an installer locks pages by id | a `measure` screen id (P5SES) |
| Templates | `assets/templates/3way-rp2350.dspipreset` (300 Hz / 3 kHz), `2way-*`, `headphones-*`, `k340.dspipreset` | layouts the tune recognises (section 1.4 step 1) |
| Host audio | **None anywhere**: no capture and no playback in the core, the clients or the hub | P5WCAP (web), P5QCAP (Qt) are the first |
| FFT | none before this plan; the seam adds `rustfft` without its SIMD features (OD-P5d) | P5TIME, P5RESP |
| Firmware loopback | a `DSPI_LOOPBACK` build option returns output slot 0 over USB (`CMakeLists.txt:132-140`, debug only); no host build of `siggen.c` exists | an optional silent electrical check in P5HIL (S-a) |
| Qt image | no Qt Multimedia in `ci/container/Containerfile.qtport:29`, `:33` (Fedora has `qt6-qtmultimedia-devel`, `mingw64-qt6-qtmultimedia`) | P5QCAP's image bump |

**Corrections to PRO1 found while checking:**
- PRO1 T1 has the host "play a log sweep or pink noise through the DSPi as its USB output, or the device's generator".
  The generator exists on stock firmware and cannot fall back to another output; it is sample-exact and repeatable;
  it needs no per-platform playback code. **Generator only**; no host playback is built (OD-P5a).
- PRO1 T1 and section 6 say phase needs "a loopback reference", so "ship magnitude-only first". For what the tune needs
  (relative delay and polarity between ways) the generator's walked, repeated sweeps put every way on **one cycle grid
  in one capture** on one DAC clock; the grid is the timing reference and gives the clock drift (section 1.1). This
  works only with explicit failure detection (capture slips, cycle identity) and a mandatory summed check that does
  not depend on the capture clock (RC5): both are in the plan.
- PRO1 T1 gives the core "a `Capture` seam per platform". The core opens no audio device (the web's wasm core cannot);
  it is **sans-IO**: a client pushes its microphone's frames, the core asks for them (OD-P5e).
- PRO1 T4 fits "the 10 PEQ bands". A fit uses the **free bands** (flat or bypassed) unless the user releases others
  (OD-P5l).
- PRO1 T5 wants "calibrated input level if the firmware adds it". The calibrated level is the microphone's, on the host,
  from its calibration file (R2); no firmware.
- PRO1 question 9 is answered by R2: a USB microphone on the host.
- PRO1 sizes phase 5 at about 20 units. The owner's requirements add the microphone calibration, placements and
  couplers, the live view, the multi-way auto-tune, the hub documents, the safety engine, Qt capture and a simulated
  world for testing without hardware: **this plan is 56.25 units, plus 1.75 for the two coordinator runs with the
  owner** (section 2, "Waves"); section 8's deferred items about 5.5; a later Android and Swift batch about 9.5.

**Not in this phase:** host playback of test audio (never, unless the owner asks, OD-P5a); measurement screens in
Android and Swift (a later batch); a microphone on the hub's machine (OD-P5f); impulse-response convolution or FIR
filters on the device (the firmware has none); more PEQ bands (FW7, upstream); AutoEQ's handling (PRO1 T6, separate);
fleet-wide tuning; a measurement started over REST; the items of section 8.

---

## 1. Goal and scope, item by item

PRO1's T1 to T5 keep their ids; the owner's requirements add T7 to T12 (T6 is PRO1's AutoEQ item, not in this phase).

| Item | What the core does | Where | Clients |
|---|---|---|---|
| **T1 transfer function** | From the generator's walked log sweeps and the microphone's capture: cycle identity, the clock drift from the cycle grid corrected per cycle, deconvolution to an impulse response per output and cycle, the timing quality gate, coherent averaging of repeats, windows per use (alignment about 5 cycles, fit 15 cycles, S-c), magnitude and phase on a 1/96-octave grid from 10 Hz to 22 kHz, fractional-octave smoothing, a noise floor from a pre-roll and a per-point "trusted" mask; REW text export of a response | `dspi-measure` `stimulus`, `ir` (P5TIME), `response`, `export` (P5RESP); the run in `dspi-core` `measure` (P5SES) | every client through the core |
| **T2 delay and polarity** | From the ways' impulse responses on the common grid of one walked run: coarse arrival from the onset, fine delay and polarity that make the summed response flattest through each crossover region; delays normalised so the speaker's smallest delay keeps its current value (RC6); polarity applied as the net invert of the crosspoints feeding the output (the programme path, RC1); **offered only when the timing quality gate passed and the summed check confirmed it** (RC5d, RC5f) | `align` (P5XO); the tune (P5TUNE) | all |
| **T3 target curves** | Built-in targets defined by parameters in our own code (flat, tilt, house curve with a bass shelf; a default per coupler type), import of any REW or CSV target, coupler compensation import, normalisation at a reference band; shown dashed on every graph, used by the fit | `target` (P5CAL); documents (P5DOCK) | all |
| **T4 room and target EQ fit** | Fit the channel's free PEQ bands toward the target inside a range (speakers: 20 Hz to 300 Hz by default, wider by choice; headphones 20 Hz to 10 kHz), bounded boost and cut, no filling of position-dependent or deep dips, headroom kept by an equal output-gain cut; preview with the predicted curve, apply guarded with A/B | `fit` (P5FIT); the tune and the live view (P5TUNE, P5LIVE) | all |
| **T5 RTA improvements** | The **microphone RTA** (Welch, pink-referenced, 1/1 to 1/48 octave, calibrated to dB SPL when the microphone is) with averaging presets (fast, slow, infinite), hold and compare of two captures | `rta` (P5RESP); `measure/live.rs` (P5LIVE) | web, Qt |
| **T7 microphone calibration** | Parse miniDSP/UMIK-1/UMIK-2 `.txt` (a `"Sens Factor =…dB, AGain=…dB, SERNO: …"` line, 0° and 90° files), REW calibration text (frequency, dB, optional phase; `*`, `#` and `"` comment lines), CSV (comma, semicolon, tab); bounded; to the grid; the sensitivity to a dB SPL scale with the vendor's published convention cited in the code, **labelled approximate** (the OS input gain is not known to a browser) until a calibrator step (94 or 114 dB at 1 kHz) was done; relative dB when neither; a `mic` document per microphone | `cal` (P5CAL); documents (P5DOCK) | all |
| **T8 placements and couplers** | Speakers: listening position; several positions (3 to 9) power-averaged with the spread kept; near-field per driver, stored and shown as a labelled near-field measurement. Headphones: coupler type (IEC 60318-4 ear simulator, flat-plate, HATS or other ear coupler), reseats (1 to 9) averaged, left and right each; each coupler type with its own default target and an optional compensation file | `types::Placement` (seam), `position` (P5CAL); the run (P5SES) | all |
| **T9 live response and direct adjustments** | Live mode: a single output's short sweeps repeated (one magnitude result per sweep) or finite pink noise with the microphone RTA, shown with the target and with the **predicted** curve of a pending change; direct adjustments: "cut this peak" and "fit this range" from a point or a span on the measured curve, each previewed, then one undo group | `measure/live.rs` (P5LIVE) | web, Qt |
| **T10 auto-tune** | Section 1.4: layout, per-way measurement, delay and polarity, crossover corner and levels, the **mandatory** summed check, the target fit, one proposal with every write in words, applied as one guarded undo group between a "before" and an "after" snapshot, verified by walked and summed runs inside the guard, kept or reverted, A/B offered | `align`, `xover`, `fit`; `measure/tune.rs` (P5TUNE) | web, Qt, Terminal (apply from a document) |
| **T11 documents and the hub** | Kinds `measurement`, `mic`, `target` in the `.dspipreset` schema-1 family (section 1.6); a local store; the hub's `measure.doc.*` family and its store, merged with the local list as snapshots are (the hub's copy wins) | `dspi-measure` `doc` (P5MDOC), `measure_doc.rs`, `measure_store.rs` (P5DOCK), `crates/dspi-hub/src/measurements.rs` (P5HUB) | all; the web reads the hub's |
| **T12 safety** | Section 1.2: a preflight, a closed-loop level ramp from a quiet start with the run's own stimulus, mode ceilings, abort on clip, SPL, a capture gap or anything foreign, only finite generator runs, never `RAW`, never a raised volume, the tweeter rule, no unmodelled processing in the path, the device lock as a dead-man lease | `measure/safety.rs` (P5SES) | all |

### 1.1 The signal path (T1, OD-P5a, OD-P5b)

1. **Stimulus: the DSPi's generator.** It **replaces the matrix mix** on the outputs it plays, then passes through each
   output's crossover, PEQ, delay, trim and volumes (RC1). So the *measured chain* of an output is crossover, PEQ,
   output gain and delay (`types::ChannelChain`), and the *programme path* adds the feeding crosspoints' gain and
   polarity (`ChannelChain.feed`), which the generator never sees: predictions of programme audio add it; a summed
   generator run reproduces the outputs' net polarity through the generator's `invert_mask`. Processing after the
   injection that the maths does not model (crossfeed, subharmonic, psychoacoustic bass, tube, loudness, an enabled
   limiter) is refused by the preflight (S13). Run kinds (RC11): **walked** (log sweep, `SIGGEN_FLAG_WALK`, one output
   per cycle round-robin, `repeat` = passes 2 to 8, default 3), **summed** (the outputs of one transition at once, no
   walk, `invert_mask` set to their current or proposed net polarity), **single live sweep** (one output, one sweep,
   repeated by the live view), **finite pink** (one output, at most 60 s). Sweep `duration_ms` 1 to 15 s (default 5.5 s,
   2.7 s live), 20 Hz to 20 kHz, `gap_ms` **1000 ms for speakers, 300 ms on a coupler** (S-b), `level_db` from the ramp,
   **never `RAW`**, never infinite.
2. **Capture: the host's microphone, pushed by the client with frame indices** (RC5c). The client opens the microphone
   and pushes mono `f32` blocks to `measure_capture_push(first_frame, samples, rate_hz, clipped)`: web through an
   AudioWorklet (`AudioWorkletNode` with `numberOfOutputs: 0`, an `AudioContext` with `sinkId: {type: "none"}` where
   supported, no forced `sampleRate` in Firefox, `first_frame` from the worklet's `currentFrame`, a MessagePort straight
   to the core Worker); Qt through `QAudioSource` with its processed-frame count. **Any gap or repeat in frame indices
   aborts the run** (S5). The default capture rate is 48 kHz (RC8). The capture runs from the pre-roll to a **post-roll
   of one gap after the last cycle** (the firmware adds no gap after it, S-b).
3. **Safety first in the push path** (RC8): each block is checked for clipping, SPL and frame continuity, O(block), before
   it is stored; these checks never wait for analysis.
4. **The core renders its own copy of the sweep.** `dspi-measure::stimulus` models the generator's log sweep as
   `siggen.c` renders it: the integer 48-bit phase accumulator with the Q31 per-sample growth (`siggen.c:360-372`,
   `:926-937`; its quantisation, about 1 part in 5·10⁴, is why the analytic sweep is not good enough), the attack and
   release windows, the cycle length `fs * duration_ms / 1000`, the gap in samples, the walk order and the per-cycle
   reset, at 44.1, 48 and 96 kHz.
5. **Cycle identity** (RC5b). The cycle index is pinned by the START: the core notes the capture frame at which its
   `SIGGEN_CTL_START` was acknowledged (and the `NOTIFY_EVT_SIGGEN_STATE` RUN event), the first arrival must fall in the
   window that implies, and during the run `SiggenStatus.active_channel` and `cycles_done` (polled about 4 Hz) must agree
   with the cycle the capture is in. A mismatch aborts the run: a missed first cycle can never shift every response onto
   the wrong output.
6. **The timing model** (RC5a). A coarse matched filter gives one arrival per cycle. The fit is **one common slope (the
   clock ratio between the DSPi and the microphone) and one intercept per output** (its acoustic delay plus the common
   latency), over at least **two passes**. The ratio is removed **per cycle in the frequency domain** (S-e): for a log
   sweep a ratio error `e` is a delay `e·t(f)` with `t(f) = T·ln(f/f1)/ln(f2/f1)`, taken out as a phase term; no
   fractional resampler, and only per-cycle data is kept. Each cycle is then deconvolved (regularised spectral division
   by the modelled sweep) and the repeats of an output averaged coherently. An output's intercept relative to its own
   cycle start is its delay; the common latency and the host's buffering cancel between outputs **of one run**. Nothing
   compares arrivals across runs (several positions use position 1's run only).
   **The arrival** (P5ARRIVAL, after P5SPIKE 2026-11-18, RC5a): the peak of the **envelope** of the deconvolved
   response, weighted by the pre-roll's noise (the matched filter's whitening of P5RAMPFIX) and limited to the sweep's
   band, not the largest `|h|` of the full band (a tweeter's two opposite-sign lobes 45-50 µs apart traded heights
   between passes and the peak jumped by 60 µs). It is refined against the **mean of the output's other repeats**
   (leave one out), maximum likelihood (`|R|²/N` over the bins with SNR above 10 dB), the lag limited to **half a carrier
   period around the envelope's**: the correlation of a band-limited response has peaks one carrier period apart
   (0.4-0.8 of the main one on the K340's tweeter), and P5TIME's full-band correlation against the first cycle locked
   one of them (-81.6 µs). Where another peak the envelope rules out reaches 0.9 of the chosen one, or the chosen one is
   the window's edge, the arrival is **ambiguous**: a typed finding, the cycle out of the fit and of the gate's largest
   residual, never a number the gate trusts. (A plain 0.5 against the first peak would refuse every flat speaker, whose
   sidelobes sit at about 0.55, and the real tweeter, which the correlation decides correctly.) The onset is the band-
   limited envelope's (30 % of its peak), so a window's centre no longer jumps with the lobes. The cycle-identity
   search (step 5) judges "heard" the same way, whitened by the capture before the START's window: P5TIME's whole-band
   peak missed a woofer behind its LR4 low-pass at the ramp's -50 dBFS start in P5SIM's -85/-90 dBFS noise (P5TUNE's
   follow-up) and the run aborted; every identity check stays as strict.
7. **The timing quality gate** (RC5d). A run is "timing reliable" only when every cycle's residual from the fit is at
   most **10 µs** and the repeats of each output agree in complex value (phase) through the transitions (spread at most
   10° where trusted). Otherwise the result keeps its magnitudes and is marked **"timing unreliable"**; delay, polarity
   and crossover proposals are refused with that reason. A single-output, single-pass run (the live view) is
   magnitude-only by definition.
   **The phase check per band** (P5ARRIVAL): in third-octave bands, each loud enough at **20 dB summed over the band**
   per repeat (not 30 dB per 1/96-octave point, which left 0 to 7 points for the K340's tweeter) and within 30 dB of its
   output's loudest band; by default **the run's transitions** (every band where two outputs of the run are both loud
   enough and within 12 dB of each other; an output in none, a run of one output, from 100 Hz to 10 kHz;
   `phase_check_bands` replaces the default). A band's bound is the larger of **10°** and **5 µs as a phase** at its
   centre: the plan's 10° through every usual crossover (below 5.6 kHz), no longer 2.8 µs at 10 kHz. The gate stays
   fail-closed and RC5's strictness for delay, polarity and crossover proposals is unchanged; the summed run remains the
   arbiter of delay and polarity (RC5f). **Findings are typed** (`TimingIssue`) as well as in words, so that an
   analyser limit, a slip and the capture clock are told apart: an **ambiguous arrival**; a **capture slip** (a cycle
   off the fit by more than the bound while its delay stays steady along its sweep); **clock wander** (each repeat's
   delay along its sweep against the mean of the other repeats, its own residual taken out, per third-octave band,
   unwrapped along frequency; a movement of four times its noise counts, more than the residual bound inside one sweep
   is a reason: "the capture clock wandered X µs / Y ppm inside its sweep"); a **phase spread** per band. P5SPIKE's
   iMM-6C: residuals 13.8 µs (only3, was 81.6) and 4.1 µs (ramp30) on the real samples, and wander of 20-80 µs at 30-95
   ppm reported on every cycle: the remaining disagreement is the capture clock, which no estimator removes (the
   owner's control experiment with an electrical loopback decides its source).
8. **No loopback, no reference speaker, no firmware time stamp** (OD-P5b). The absolute latency from USB to air is not
   measured; nothing needs it. The one check that does not depend on the capture clock at all is the **summed run**
   (section 1.4 step 6), which is mandatory before any timing write is offered (RC5f).
9. **Noise and trust.** A 1 s pre-roll with the generator idle gives the noise floor; per point, SNR above 20 dB and a
   repeat-to-repeat spread under 1 dB mark it trusted (`Response.valid`); untrusted points are drawn faint and never
   fitted. (WP P5TRUST: the spread is of the repeats' **windowed** responses, the fit's window, beyond the spread their
   noise explains; the SNR rule judges the noise.)
10. **Memory and analysis** (RC8). Capture at most `MAX_CAPTURE_SAMPLES` = 8 Mi frames (32 MiB); the plan step refuses
    a setup whose planned frames (pre-roll + cycles + post-roll at the capture rate) exceed it, so the largest
    default run (a whole stereo three-way system as one alignment group: 6 outputs × 3 passes × 6.5 s ≈ 117 s, 5.6 M
    frames at 48 kHz) fits and a 96 kHz capture of the same
    does not (the notice says to use 48 kHz or fewer outputs a run). An analysis slice is one cycle's FFT (up to 2^19
    points, a few hundred ms in wasm) and runs in the cycle's gap or after the generator has stopped, never between a
    push's safety check and the next block; `measure_tick` only schedules slices and keeps the lease.

### 1.2 Safety (T12)

The owner's rule, R5, item by item, and what enforces it. **S1 to S13 are the tests of P5SES and P5TUNE.**

- **S1. No host player, ever.** The stimulus is the DSPi's generator; no client plays test audio. If the DSPi leaves
  USB, nothing anywhere falls back to another output. A client that finds no generator (firmware before v1.1.5) offers
  the microphone RTA of programme audio only, and the screens say why. The web's capture graph has no output node and
  its context no output device where the browser allows it (section 1.1 step 2).
- **S2. Volume.** The session never writes `vol.user`, `vol.master`, a mute or an output gain while measuring. Preflight
  **refuses** when `vol.user` is above **-12 dB** (80 % of the registry's -60..0 dB range; stricter than what an OS slider
  at 80 % sends), with the notice to lower it; the master volume is judged by the limits profile as today.
- **S3. Start quiet, ramp with the run's own stimulus, fail closed, cap** (RC3). Before a run, per output, a ramp of
  **short walked log sweeps (about 1 s)**, the same kind as the run, so the run is never louder than what was measured
  (pink noise, 1/8-normalised, is only used for the pink-noise RTA and ramps itself the same way). Start levels:
  speakers **-50 dBFS**, headphones **-60 dBFS**; +6 dB a step until the microphone's SNR is 35 dB in 100 Hz to 10 kHz or
  a ceiling. **The SNR is in band, never broadband** (P5RAMPFIX, after P5SPIKE 2026-11-18: a real microphone's floor
  was -51 dBFS broadband but nearly all 50 Hz rumble, so a step clearly audible at 1 kHz read "not heard"): a sweep
  step is judged by the matched filter of the known sweep (P5TIME's model at the capture rate), whitened by the
  pre-roll's noise spectrum, in the sweep's band within 100 Hz to 10 kHz, its loudest 2 ms against the same output's
  loudest 2 ms away from the peak; a pink step by its third-octave band levels against the pre-roll's
  (`dspi_measure::hearing`). Low-frequency rumble never counts as signal. **Fail closed:** once the signal is above
  the noise (in-band SNR 6 dB), a 6 dB step must raise the microphone's in-band level by at least 3 dB; by start +
  18 dB the signal must be above the noise; and no ramp ends on a step that is not above the noise (a ceiling or the
  SPL cap reached first, and the start level only: `measure.start_level_quiet`, "too quiet to hear at the start
  level: measure with the ramp"). Otherwise the ramp stops and the user is asked ("the microphone does not hear this
  output: check the microphone and the wiring") before any higher level; the notice lists each step's in-band SNR,
  and the document keeps them (`capture.rampSteps`). Ceilings
  (generator peak): speakers **-12 dBFS**, headphones **-30 dBFS** by default; the user may lower them and raise them only
  to the hard caps, **-6 dBFS** and **-20 dBFS**. With a calibrated microphone the ramp also stops at **95 dB SPL**
  (speakers) or **90 dB SPL** (on a coupler), Leq over 125 ms (hard caps 105 and 100). **A walked run has one
  `level_db`: the minimum of its ways' ramp results.** Never full scale.
- **S4. Abort on clip.** A capture sample at -1 dBFS or above, a measured output's peak at -0.5 dBFS or above in
  `fetch_status` (or a 0xE0 clip event where the firmware has it), the SPL past its cap: `SIGGEN_CTL_STOP_NOW` at once,
  the run discarded, the notice says which. A ramp step that clips ends the ramp at 12 dB below that step.
- **S5. Abort on anything foreign** (RC5c, RC5h). A `NOTIFY_EVT_SIGGEN_STATE` stop that is not ours (another console, a
  preset load); a foreign write on a measured output's chain or on a volume (the journal's `note_foreign`, a
  `PARAM_CHANGED` or `BULK_INVALIDATED` of another origin); **any volume change, including the host's USB volume**
  (`NOTIFY_EVT_MASTER_VOLUME`, the user-volume notification); **`NOTIFY_EVT_INPUT_FORMAT`** (a rate change); an input
  stream starting or stopping; **programme audio appearing on the inputs** (input peak above -70 dBFS); the device lock
  lost or `locked`; a disconnect; **a gap or repeat in the capture's frame indices**; a cycle-identity mismatch (section
  1.1 step 5); the run taking 5 s longer than planned; the user's Stop. STOP_NOW where the device is reachable, the run
  discarded, nothing retried.
- **S6. The tweeter rule** (RC4). Never `SIGGEN_FLAG_RAW`. Preflight refuses to measure an output whose role is `Mid` or
  `High` unless it has an active high-pass (a crossover band of a high-pass type, or a PEQ high-pass). Without a limits
  profile: **`High` at or above 1 kHz and at least 12 dB/octave, `Mid` at or above 80 Hz and at least 12 dB/octave**;
  with one, its `highpass_min_hz` and `highpass_min_slope_db` replace these (lower values only by the installer's
  profile). The slope is always checked. The tune never proposes a high-pass below these. Driver data without the
  filter comes from de-embedding the known chain (section 1.4 step 3), never from bypassing it.
- **S7. Only finite runs.** Every generator configuration the session stages has `repeat >= 1`, or for pink a
  `duration_ms` of at most 60 s (the live view re-arms noise every 30 s while the client is alive). A run is at most
  120 s and within `MAX_CAPTURE_SAMPLES` at the capture rate. A client that dies leaves a run that ends on its own, and a
  device lock whose lease (30 s, renewed by `measure_tick`) runs out.
- **S8. Silence first.** Preflight refuses while any input's peak in `fetch_status` is above -70 dBFS over 1 s ("stop
  playback to measure"); during a run S5 aborts on it. The session never switches the input source.
- **S9. Bounded proposals.** Every write of a proposal is judged before it is offered: inside the registry's range, the
  limits profile (gain, high-pass, delay, boost), the platform's delay maximum; PEQ boost at most **+3 dB** (speakers) or
  **+6 dB** (headphones) and never above `max_peq_boost_db`; cuts at most -15 dB; Q 0.3 to 10; crossover and level
  changes **cut only** relative to the speaker, re-normalised so the loudest way keeps its previous gain (no gain ever
  raised, and repeated tunes do not ratchet levels down, S-f); the EQ's net boost offset by an equal output-gain cut. A
  proposal with a write the core would refuse is not offered.
- **S10. Guarded and reversible.** A tune is applied only inside `apply_guard_begin` (by default as long as the planned
  verifying runs plus a margin, at most the guard's 590 s; WP P5TRUST, before a flat 300 s), as one undo group, after a "before" snapshot is filed and **after the preflight is run again**
  (silence, locks, roles, and every output's chain still the one the proposal was computed for; RC11). Keep, Revert, the
  timeout, a failed write or a lost connection behave as every guarded apply does. A direct adjustment is one undo
  group, unguarded (one band, as a manual edit).
- **S11. Locks and roles.** Nothing is played or written without a writable connection, the client read-only mode off,
  the role (Control or Admin through a hub), the `measure` screen not locked by an installer, and no foreign guard or
  A/B compare pending.
- **S12. Privacy.** Raw microphone audio stays in the core's memory for the run only; it is never logged, put in an
  event, a document, the diagnostics bundle or sent to the hub. A WAV of a capture is written only when the user exports
  it. Documents hold derived curves only.
- **S13. No unmodelled processing in the path** (RC2). Preflight refuses, naming what to switch off (the session writes
  nothing): output crossfeed on a pair containing a measured output (`cf.on`, `cf.outputs`), psychoacoustic bass
  (`bass.on`, `bass.mask`), tube (`tube.on`, `tube.mask`), subharmonic synth (`sub.on`, `sub.mask`), loudness (`loud.on`,
  `loud.mask`), and an enabled limiter on a measured output unless its threshold is above the run's ceiling; a run aborts
  when a measured output's limiter gain-reduction meter moves. Input-side EQ that is not flat is a warning (it is outside
  what is measured, S-h).

### 1.3 Placements and couplers (T8)

| Placement | Runs | What is stored | Used for |
|---|---|---|---|
| Listening position | one walked run of the outputs to be aligned together (the alignment group, section 1.4 step 4) | each output's response with phase, arrival, timing quality | alignment, crossover, fit |
| Several positions (3 to 9) | position 1 is the listening position and the only timing reference; one run per position, the user moves the microphone between them (`measure_next_position`) | per output: the power average of magnitudes, the spread per point, position 1's phase and arrival | the fit (dips that vary more than 3 dB across positions are not filled); alignment from position 1 only |
| Near-field | one run per driver with the microphone close to it | the near-field response, labelled near-field | shown and compared; not used by the tune in this phase (the splice is section 8) |
| Headphones | coupler type; reseats 1 to 9, left and right each | per side: the average of reseats with the spread; the coupler type; the compensation applied | the fit per side toward the coupler's target |

Coupler types and their defaults: **IEC 60318-4 ear simulator** (built-in default: a parametric diffuse-field-like curve
with a bass shelf, labelled as ours), **flat-plate coupler** (a flat-plate-specific parametric default; a vendor's
compensation file is better and the notice says so), **HATS or other ear coupler with pinna** (as the ear simulator,
with the pinna's own compensation file if the user has one). The headphone preflight says "on the coupler, not on a
head" (S-h). No third-party target data is bundled (OD-P5p); any target or compensation is imported from a file.

### 1.4 Auto-tune, step by step (T10, R5)

Example: a stereo three-way speaker (outputs 0 to 2 left low, mid, high; 3 to 5 right). Every step shows its result
before the next, and the user may stop after any step with nothing written.

1. **Layout.** The core proposes speakers and ways from the device's state (an output with only a low-pass is a `Low`,
   high-pass and low-pass a `Mid`, only a high-pass a `High`, a subwoofer by name or band; the matrix's sources pair left
   and right) and reads each output's **programme feed** (crosspoint gain and net polarity, RC1); the user confirms or
   edits it. A template's layout (`3way-rp2350`, `2way-*`, `headphones-*`) is recognised.
2. **Preflight** (section 1.2): every blocker in words, nothing played until it is clear.
3. **Measure.** One walked run per **alignment group** (step 4) at the listening position, after S3's ramp; with several
   positions, the further runs. The measured responses hold the measured chain each way had, so **each driver's own
   response is the measurement divided by its known chain** (de-embedding through `SectionModel`), with its **own
   validity mask**: trusted only where the measurement's SNR minus the chain's attenuation is at least 20 dB (RC12; review
   P5-C2: that is the measured SNR **per point**, which already is lower by the chain's attenuation, taken as it is:
   subtracting the attenuation from it again asked 69 dB in the passband an octave past an LR4 corner).
4. **Delay and polarity** (T2), only if the run passed the timing quality gate. The **alignment group** is the set of
   outputs aligned to each other, measured in one walked run: by default one speaker's ways; "align the whole system"
   puts both sides and the subwoofer in one run when S7's 120 s and the capture bound allow, else it is refused with the
   reason (RC6). Delays are normalised so the group's smallest delay **keeps its current value** (a speaker's existing
   common delay, left/right distance, sub or lip-sync alignment is not erased; RC6), within the platform's range and the
   profile's. Polarity is the net invert of the output's feeding crosspoints. The user may keep the current delays.
5. **Crossover** (corner and levels): per transition (low/mid, mid/high), a search over the corner (1/12-octave steps,
   within half an octave of the current one by default, wider by choice but **only where both drivers' de-embedded
   responses are trusted one octave each side**, RC12; never below S6's high-pass rule), keeping the current family and
   order, with the ways' levels (cut only, S9), delay and polarity, each candidate scored by the predicted acoustic sum
   (including the programme feed) against the target through the transition. "Keep the crossover, align only" is an
   option. WP P5TRUST: "trusted one octave each side" is 85 % of the span's points in both drivers, half of every third
   of an octave, the corner's own third of an octave as the span, no hole wider than 1/12 octave, and a driver at least
   20 dB down in its candidate's stopband needs no trust of its own there; a corner moves only when its sum is at least
   1 % better than the current one's; a best corner on the edge of the search (the window's or the trusted range's) is
   no minimum, so the corner is kept and only the timing proposed; a tweeter's high-pass is never lowered on the
   magnitude sum alone (with the delays kept no lower corner is a candidate; every crossover write is a timing write,
   applied only after the summed check passed) and never below S6.
6. **Summed check, mandatory** (RC5f). One summed run per transition with the **current** chain: both ways play the
   generator's correlated copy, `invert_mask` set to the outputs' current net polarities (the generator bypasses the
   crosspoints, RC1). The measured sum is compared with the sum predicted from the walked run's responses and timing. If
   they differ by more than **1.5 dB RMS** through the transition (one octave each side), the walked run's timing is not
   trusted: every delay, polarity and crossover write is withdrawn from the proposal with the reason; the EQ fit may
   remain. The proposed timing is checked again by the summed runs of step 10, inside the guard.
7. **Target fit** (T4): per speaker side, the free PEQ bands toward the target, on the predicted response with the new
   crossover, within the fit's range and caps (S9). WP P5TRUST: on a whole speaker the fit's level is anchored to the
   target levelled in the reference band, dips deeper than the boost cap plus 1 dB are left out of the fit and its level,
   and bands of Q 0.7 or more and low shelves are allowed (never a high shelf; at most 0.5 dB into the reference band).
8. **Proposal.** One list: every write in words ("Output 2 (L high): crossover 2.8 kHz LR4 → 3.1 kHz LR4; delay 0 →
   0.21 ms; PEQ band 6: peak -3.5 dB at 1.9 kHz Q 2.1"), the curves before and predicted, the limits it was held to.
   Nothing is written yet.
9. **Apply.** The preflight again (S10); `snapshot_capture_document` "Before auto-tune <date>" filed (the hub's store
   through the hub, else the local one); `apply_guard_begin("Auto-tune", <the planned verify plus a margin>)`; `begin_group("Auto-tune")`; the writes
   through `write_param` (each judged by read-only, limits and the lock as any write); `end_group`. A refused write stops
   the apply and the guard reverts.
10. **Verify** inside the guard: one walked run of the tuned outputs **and one summed run per transition** with the new
    net polarities (a walked run cannot see polarity or delay, RC1); measured against predicted; the guard's countdown
    shown.
11. **Keep or revert.** Keep: `apply_confirm`, "After auto-tune <date>" filed with a `tune` block (the measurement's id,
    the algorithm version, the options), and the A/B compare of the two snapshots offered (`ab_start`). Revert or the
    timeout: the guard's revert; nothing else changes. After a verify that did not pass, the first Keep is refused with
    `measure.tune.keep_unverified` and only a second Keep, the user's explicit confirmation, keeps it (WP P5TRUST).
    Review P5TRUST: the rule is the guard's, so the generic `apply_confirm` of the tune's guard is held to it as the
    tune's own Keep is; the second Keep counts from 1 s after the refusal (a double click is one press); and the failed
    verdict holds through a Stop and a new verify without a verdict, until a verify passes or another apply.

### 1.5 Locks, read-only, other clients, the hub

- **The device lock.** A run holds `device_lock_acquire("Measuring", 30 s)`, renewed by `measure_tick` while the session
  is active, released at the end, on abort and on drop. Over DSPi Link every other session's writes get `locked`, and
  its lock indicator shows the reason. Over USB the lock asks no one; a foreign write is then caught by S5. The user's
  own edits in the live view are the lock holder's and pass.
- **Read-only.** A client in read-only mode, a read-only connection or a Viewer through the hub cannot play the
  generator: it gets the **passive** microphone RTA (programme audio, no stimulus) and document viewing; preflight names
  the reason.
- **Other clients** see the generator's state from its notification (existing), the lock's reason, and saved
  documents through the hub's `measure.doc.changed`. A measurement in progress is the measuring client's.
- **The hub** stores and serves documents and arbitrates through the lock it already has; **the measurement runs where
  the microphone is**: in the browser's core for the web, in the Qt app, never on the hub's machine (OD-P5f). Every
  device write is the measuring client's, judged as the caller's; the hub adds no write of its own.
- **Several devices.** One session per `DspiCore` (per device); a second `measure_start` on the same core is `busy`.
- **Screens:** the `measure` screen shows in Advanced and Installer modes (S-h).

### 1.6 Documents (T11, OD-P5g, RC7)

All three are `.dspipreset` schema-1 documents (`{"schemaVersion": 1, "meta": {"kind": …, "name": …, "savedUtc": …,
"appVersion": …}, …}`), parsed with bounds, refused by `config_import` (they configure no device: a clear
`InvalidArgument`). Curves are stored compactly: magnitudes rounded to 0.01 dB and phases to 0.1°, each as a base64
array of little-endian `i16`/`i32` in those steps, the `valid` mask as a base64 bitset; the grid as its three numbers. A
standard-grid output with phase is then about 9 KB. **No impulse-response excerpt in this phase** (section 8).

| Kind | Holds | Bound | Scope |
|---|---|---|---|
| `measurement` | `measurement {serial, platform, savedMs, placement, mic (id, name, serial, sensitivity, approximate, the calibration's hash), stimulus (kind, f1, f2, duration, repeat, gap, level_db, device rate), capture (rate, clock ratio, timing quality: reliable or not, max residual), outputs: [{output, role, speaker, chain (the measured chain and its programme feed), arrival_ms, response (grid, mag, phase?, valid, spl), spread?}], positions, notes}`; after a tune, `tunes: [{at, snapshotBefore, snapshotAfter, kept}]` | **1 MiB** (its own store) | per device serial |
| `mic` | `mic {name, serial?, vendor?, format (umik, rew, csv, dayton: P5DAYTON), angle (0, 90), sensitivity_db?, gain_db?, splOffset_db?, approximate, response (grid, mag, phase?), sourceName, sourceHash}` | 256 KiB | the user's (hub-wide on a hub) |
| `target` | `target {name, use (speakers, headphones), coupler?, definition (builtin params, or imported points), compensation?, reference band}` | 256 KiB | the user's (hub-wide on a hub) |

The tune's result is not a new kind: it is two **snapshots** (`meta.kind = "snapshot"`), "Before auto-tune" and "After
auto-tune", the latter with an additive `tune` block. REW export is text the user saves, not a document.

---

## 2. Work packages

**Shared rules for every P5 WP.** `PLAN-CLIENTS.md` section 1, `WP-BRIEF.md`, the wave1 brief, PLAN.md section 1, and:
- **`dspi-measure` is pure:** no I/O, no threads, no clock, no `dspi-core`, no `dspi-transport`; every public function
  has a `proptest_*` that it neither panics nor returns non-finite values; it builds for `wasm32-unknown-unknown`
  (`DSPI_PROFILE=cross ci/run.sh 'cargo build -p dspi-measure --target wasm32-unknown-unknown'`). Its lint set is the
  hub's (`unwrap_used`, `expect_used`, `indexing_slicing`, `arithmetic_side_effects` warned; `-D warnings` in CI).
- **The seam is fixed for P5-A:** `crates/dspi-measure/Cargo.toml` already has every dependency P5-A needs (`serde_json`,
  `base64`, `rustfft` with `default-features = false`), `lib.rs` already has every `mod` line, `deny.toml` lists the
  crate. **No P5-A WP edits `Cargo.toml`, `Cargo.lock`, `lib.rs` or `deny.toml`**; one that needs to says so in its
  report. Shared types are `crates/dspi-measure/src/types.rs`: a WP that needs another one adds it there under its
  marker and lists it; nobody changes a field's meaning.
- **The core's measurement code is sans-IO:** no thread, no sleep, no audio API; time from the core's clock seam
  (`dspi_transport::clock::Instant`, as `rta.rs`); listeners called with the `Inner` lock released; every device write
  through an existing typed setter on the `Writable`. Never `SIGGEN_FLAG_RAW`, never a write to a volume while measuring,
  never an infinite generator run (section 1.2).
- **Determinism** (RC10): one FFT (`rustfft`, scalar only, no `realfft`). **Quantised proposals** (the writes a tune or a
  fit offers) are identical for identical input on one target, and their goldens compare exactly; curves and anything
  after an FFT or a libm call compare with a tolerance (native glibc and wasm libm differ in the last place). No
  hash-map iteration order, no time, seeded noise in the simulator. Goldens under `crates/dspi-measure/testdata/p5/` and
  `crates/dspi-core/testdata/p5/`.
- **Testing without hardware** (section 4): the simulator (P5SIM) and the demo device; **no microphone, no audio
  device, no K340 in any WP**; the real runs are P5SPIKE's and P5HIL's.
- **Bindings** (C, UniFFI, wasm) are shared and additive, under markers: run `ci/drift.sh write`, regenerate the Android
  contract and classify new methods in `WriteGuardedCore` (writes: `measure_start`, `measure_tune_apply`,
  `measure_live_*` that write; reads: the rest). Apple bindings are regenerated at merge by the coordinator;
  `ci/macvm.sh macos` once after P5-C.
- **Notices:** user text from `assets/notices/notices.json`, one contiguous block of rows per WP, no host-device nouns.
- **Web:** unit tests and Playwright e2e in the container (`ci/check.sh --step web`), Chromium and Firefox, 375 px with
  no sideways scroll; the microphone in e2e is the browsers' fake media device: Chromium fed with a WAV the simulator
  rendered (content checks), Firefox's fake stream is a tone (flow checks only).
- **Qt:** `ci/check.sh --step qt`, `--step qt-offscreen`, `--step qt-windows`; registrar-based controllers.

Sizes are this project's units (about one hour of agent time with review); estimates.

### Wave P5-A: the maths, the simulator, the document model (pure, parallel)

**P5TIME: Stimulus model and the timing core.** [TODO] Size 3.5. Wave P5-A. **Owns:**
`crates/dspi-measure/src/{stimulus,ir}.rs` (and `stimulus/`, `ir/` for tests). **Implements:** section 1.1 steps 4 to 7:
`stimulus` (the log sweep sample-faithful to `siggen.c`'s integer recurrence, fades and windows, the walk order and cycle
schedule incl. no gap after the last cycle, at 44.1/48/96 kHz); `ir` (the coarse matched filter, cycle identity from a
given START frame and the expected schedule, the common-slope per-output-intercept fit over at least two passes,
frequency-domain per-cycle drift removal, per-cycle deconvolution by regularised spectral division, coherent averaging,
the pre-roll noise floor, windows: fixed gate and frequency-dependent, about 5 cycles for alignment and 15 for the fit;
the **timing quality gate** with its residuals). Bounds: `MAX_CAPTURE_SAMPLES` 8 Mi, sweep 1 to 15 s, 8 repeats,
16 outputs a run. **Done when:** a known impulse response convolved with the modelled sweep is recovered with an error
below -60 dB; a microphone clock 100 ppm off is measured within **0.2 ppm** over an 18 s baseline and removed; two
outputs 1.234 ms apart in one walked run come out 1.234 ms ± 5 µs apart at 48 kHz with 50 ms of unknown latency and
80 ppm drift; 3 repeats raise the SNR by 4.8 dB ± 0.5; **each of P5SIM's capture faults (a dropped block, a repeated
block, a step slip, slow ratio wander, a device-side slip) is detected — by the frame-index check where it is visible
there, else by the quality gate — and none yields a delay off by more than 10 µs without "timing unreliable"**; a START
frame one cycle off is a cycle-identity failure; the sweep model matches an independent re-implementation of
`siggen.c:360-372` and `:926-937` (quoted line by line in the test; there is no host build of `siggen.c`) for the first
and last 4096 samples at each rate; `proptest_*` over random captures and parameters; wasm32 build. **Deps:** the seam.
**Clients:** P5RESP, P5SES, P5TUNE.

**P5RESP: Responses, smoothing, the microphone RTA, export.** [TODO] Size 2. Wave P5-A. **Owns:**
`crates/dspi-measure/src/{response,rta,export}.rs` (and folders). **Implements:** `response` (an impulse response or a
spectrum to the `LogGrid`, unwrapped phase, smoothing 1/1 to 1/48 octave, the `valid` mask from SNR and spread);
`rta` (Welch with Hann, 50 % overlap, fractional-octave bands, pink-referenced so pink noise reads flat, calibrated
through a `mic` response, exponential and fixed averaging, a held copy and a compare); `export` (REW text: `freq mag
phase` with a header). It takes spectra or impulse responses as input, so it needs nothing of P5TIME's in the same wave.
**Done when:** a known response on the grid within 0.05 dB; smoothing of a 1/3-octave-wide peak by 1/3 octave halves it
within 0.5 dB; pink noise through `rta` reads flat within 0.5 dB per 1/3 octave 100 Hz to 10 kHz; an export parses back
in REW's column format; `proptest_*`. **Deps:** the seam. **Clients:** P5SES, P5LIVE, P5DOCK (export).

**P5CAL: Microphone calibration, targets, positions.** [TODO] Size 2.5. Wave P5-A. **Owns:**
`crates/dspi-measure/src/{cal,target,position}.rs` (and test folders), `crates/dspi-measure/testdata/p5/cal/` (fixtures
**written by hand in each format**, never a vendor's file). **Implements:** T7's parsers (UMIK-1/2, REW, CSV) and the
sensitivity to dB SPL (the vendor's published convention cited in a comment with where it was read; `approximate`
until a calibrator step; `spl_offset_from_tone(capture, 94 or 114)`; relative when neither); T3's targets: built-ins by
parameters (flat; tilt in dB/octave; house curve = bass shelf gain, corner and Q plus a tilt; a default per `Coupler`
defined by parameters and named ours), import of REW and CSV targets and coupler compensations, normalisation at a
reference band (default 500 Hz to 2 kHz mean); T8's maths: power average of magnitudes, the spread, reseat averaging.
Files at most 1 MiB and 8192 lines, points sorted and de-duplicated, outside 10 Hz to 22 kHz ignored, non-finite refused
per line with the line number. **Done when:** each format's fixture parses to the expected response and sensitivity; a
UMIK-style 90° file is recognised as 90°; a file with phase keeps it; garbage, an empty file, 10 000 lines, a binary file
and non-UTF-8 text are typed errors; built-ins are exact at their defining points; `proptest_*` fuzz every parser.
**Deps:** the seam. **Clients:** P5MDOC, P5SES, P5TUNE, P5LIVE.

**P5FIT: The bounded EQ fit.** [TODO] Size 4. Wave P5-A. **Owns:** `crates/dspi-measure/src/fit.rs` (and `fit/`).
**Implements:** T4: `fit(measured: &Response, target: &Response, chain: &ChannelChain, model: &dyn SectionModel,
options: &FitOptions) -> Result<FitResult>`; options: range, smoothing, max boost and cut, Q range, minimum |gain|, the
band indices it may use (free bands by default: flat or bypassed), shelves allowed, dip policy (no fill of points the
`valid` mask or the position spread marks, no more than the max boost into a dip deeper than 10 dB), headroom
compensation; greedy placement on the largest error area, then bounded refinement (coordinate descent or
Levenberg-Marquardt with box constraints) of every fitted band; stops at an RMS error target, the band budget or an
iteration cap. Returns the bands, the predicted response, the RMS error before and after in the range, the gain cut for
headroom, notes in words. The core quantizes the result with its setters' quantizers and predicts again (OD-P5c).
**Done when:** a simulated room with three modes is flattened to ±1.5 dB in 20 to 300 Hz with at most 5 bands; a 15 dB
narrow null gets at most the max boost; a dip that varies across positions is left alone; no band ever leaves its
bounds or uses a band index not allowed; the error after is never above the error before (else no change is returned);
identical input gives identical quantised bands; 10 bands on the standard grid fit in under 200 ms native release
(iteration cap); `proptest_*`. **Deps:** the seam. **Clients:** P5TUNE, P5LIVE.

**P5XO: Alignment and the crossover corner.** [TODO] Size 3.5. Wave P5-A. **Owns:**
`crates/dspi-measure/src/{align,xover}.rs` (and folders). **Implements:** T2: `align(ways, group, options) ->
AlignResult` (refused unless every way's measurement is "timing reliable"; coarse onset, fine delay and polarity by the
flattest summed magnitude through each transition including the programme feed, delays normalised so the group's
smallest delay keeps its current value (RC6), within bounds; a confidence per pair); de-embedding with its validity mask
(SNR minus chain attenuation at least 20 dB, RC12); `crossover_corner(drivers, transitions, target, constraints, model)
-> XoProposal`: candidate corners on a 1/12-octave grid within the window **and inside both drivers' trusted range one
octave each side**, the current family and order kept, levels cut-only and re-normalised (S-f), delay and polarity per
candidate, the predicted-sum cost through the transition, bounded (at most 5 000 evaluations a transition);
`predict_sum` for the summed check, with the generator's `invert_mask` or the programme feed. **Done when:** on
synthetic drivers of a three-way speaker the corners come within 1/6 octave of the designed optimum, a 0.35 ms offset is
recovered to ±10 µs, an inverted tweeter is flipped, "align only" keeps every corner; a group with an existing common
delay of 3 ms keeps its smallest delay at 3 ms; a measurement marked "timing unreliable" gets no alignment; a candidate
whose transition leaves either driver's trusted range is never evaluated; no protective high-pass from the constraints
is lowered, no output gain raised; identical input gives identical quantised proposals; under 2 s native release for a
three-way pair; `proptest_*`. **Deps:** the seam. **Clients:** P5TUNE.

**P5SIM: The simulated world and WAV files.** [TODO] Size 2.5. Wave P5-A. **Owns:** `crates/dspi-measure/src/{sim,wav}.rs`
(and `sim/`), `crates/dspi-measure/testdata/p5/` (goldens other than `cal/`). **Implements:** behind feature `sim`:
drivers (sensitivity, mechanical high-pass, roll-off, breakup peak, acoustic offset), a speaker as ways per output, a
room (modes with decay, one or two reflections), a headphone on each `Coupler` type with a seal-leak parameter per
reseat, a microphone (its own response; a matching calibration text), a noise floor, the microphone's clock offset in
ppm, latency and capture rate; **capture faults** (a dropped block, a repeated block, a step slip, slow ratio wander, a
device-side slip; RC5e), each with the frame indices a real client would report; `render(world, stimulus per output,
chain per output, model, device rate) -> capture frames`, deterministic by seed. `wav`: read and write PCM 16-bit and
float 32, mono or a picked channel, at most 64 MiB, typed errors. **Done when:** an impulse through a flat world gives
the impulse; a 50 ppm world drifts by 50 ppm; each fault appears in the rendered capture as specified; goldens of three
worlds (three-way speaker in a room, two-way near-field, headphones on the 711 coupler) are stable within tolerance; WAV
round trips and `proptest_*` fuzz of the reader. **Deps:** the seam (a test stimulus of its own; P5TIME's model is used
from P5-B on). **Clients:** every later test; P5DEMO; the web e2e's fake microphone WAVs.

**P5MDOC: The document model and the contract.** [TODO] Size 2.5. Wave P5-A. **Owns:** `crates/dspi-measure/src/doc.rs`
(and `doc/`), new `docs/integrator/measurement.md`; rows in `docs/terminal/preset-format.md` (the three kinds),
`docs/GLOSSARY.md` (Measurement, Generator, Sweep, Impulse response, Target, Microphone calibration, Placement, Coupler,
Way, Alignment group, Auto-tune, Live response, Microphone RTA, Device analyser, Timing quality), `docs/integrator/hub-api.md`
(a new section 22 "Measurement documents", draft), `docs/PARITY.md` (rows). **Implements:** section 1.6's three kinds as
serde structs with `validate()` (bounds: `measurement` 1 MiB, `mic` and `target` 256 KiB, grids at most
`MAX_GRID_POINTS`, 16 outputs, 9 positions, names 256 characters, notes 4096; non-finite refused; unknown keys kept on
read and written back) and the compact curve encoding (rounded steps, base64, the bitset); `measurement.md`: the
workflow of sections 1.1 to 1.5, the S-rules, the documents with examples, the hub messages (draft), what the core never
does. **Done when:** every kind round trips (curves within their rounding); each bound refuses one past it; a
three-way stereo measurement with phase on the standard grid stays under 100 KiB; a document of another kind is refused
by each parser; `proptest_*` over mutated documents; the links resolve; the no-host-noun check passes. **Deps:** the
seam. **Clients:** P5DOCK, P5SES, P5HUB, P5TUNE.

**Wave P5-A review outcomes** (`work/review-p5a/REPORT.md`; binding for P5-B and later, they amend the entries above):
- **One level convention:** a full-scale sine reads 0 dBFS (`types::FULL_SCALE_SINE_DB`); `cal::level_dbfs`,
  `spl_offset_from_tone`, a `mic` document's `splOffsetDb` and the microphone RTA's bands all use it.
- **One smoothing kernel** (`response::smooth_magnitudes`), two domains: dB for a graph (`response::smooth`), power for
  the fit and any level estimate (`SmoothingDomain::Power`).
- **T2's rule is phase agreement, not "the flattest summed magnitude":** `align` and `crossover_corner` both choose
  delay and polarity by the two ways' phase agreement through their overlap (a Linkwitz-Riley pair sums almost as flat
  with the wrong polarity half a period later, and a driver resonance then decided; a correctly wired tweeter was
  flipped on P5SIM's room). A flip needs a 10 % margin. An **odd-order Butterworth pair** is aligned to quadrature and
  its polarity is never changed. The summed check (step 6) is unchanged.
- **Windows are centred on a response's onset** (`OutputIr.onset_index`), never on its peak (the peak of a band-limited
  way jumps between lobes from run to run). P5SES builds a `Response` from P5TIME's windowed values with
  `response::from_grid_values` (the arrival as its reference delay), masks every point outside `WalkedAnalysis.band_hz`
  and takes the SNR from `OutputIr.noise_db` plus `10·log10(Σw²)`; `crates/dspi-measure/tests/common/mod.rs` is the
  worked example, `tests/p5a_end_to_end.rs` the whole chain.
- **A fit that replaces a band in use lists that band** (rewritten, or flat): every entry of `FitResult.bands` is
  written. A fit of a speaker's sum applied to one way's PEQ is only right well below that way's crossover (P5TUNE: fit
  the bass on the woofer's chain below half its corner, or put the same band on every way that plays there).
- **Built-in targets** are stored by `target::Builtin::id` and `params`; `doc::Target::from_target`/`to_target` and
  `doc::MicCal::from_calibration` are the only conversions between P5CAL's types and the documents.
- `min_passes_reliable` stays 3: P5SES never lowers it; a two-pass run is "timing unreliable" by definition.

### Wave P5-B: the core session, the core documents, the clients' capture

**P5DOCK: Documents in the core.** [TODO] Size 2.5. Wave P5-B. **Owns:** new `crates/dspi-core/src/measure_doc.rs` (and
`measure_doc/`), new `crates/dspi-session/src/measure_store.rs` (and `measure_store/`), the three kind words in
`files.rs`'s `doc_kind` and the refusal in `config_import` under a marker, the `pub mod` lines under markers, the
`dspi-measure` dependency line in `crates/dspi-core/Cargo.toml` and `crates/dspi-session/Cargo.toml` under markers (the
only such edits of P5-B), binding wiring for its calls. **Implements:** the local store (a folder per serial for
measurements, `mic/` and `target/` folders, an `index.json` cache rebuilt from the documents, atomic writes, the
corrupt-file rule, the same ids rule as `snapshot_store`); calls `measure_docs(kind, serial?)`, `measure_doc_get(id)`,
`measure_doc_put(text)`, `measure_doc_rename`, `measure_doc_note`, `measure_doc_delete`; `mic_import(name, file_text)
-> mic doc`, `target_import(name, file_text, use, coupler?)`, `target_builtins() -> Vec<TargetInfo>`,
`measurement_export_rew(id or text, output) -> String`; the `SectionModel` implementation over `curves.rs`'s cascade
(`pub(crate)`, used by every later core WP); on the web the calls take and give document text, as snapshots do; the
regenerated Rust notices once a client links `rustfft`. **Done when:** store tests as P1SN's (torn index, missing
documents, ids in two folders refused, a full store); each import from P5CAL's fixtures; a `config_import` of a
measurement is refused with nothing sent; the `SectionModel` agrees with `compute_magnitude_curve` and
`compute_phase_curve` within 0.01 dB / 0.1° on the curves reference table (`curves_ref.rs`); the same text through C,
UniFFI and wasm; `ci/drift.sh` clean; the notices checks pass. **Deps:** P5CAL, P5MDOC, P5RESP (P5-A). **Clients:**
P5SES, P5HUB, P5TUNE, P5TM, the screens.

**P5SES: The measurement session.** [TODO] Size 6. Wave P5-B. **Owns:** new `crates/dspi-core/src/measure.rs` and
`crates/dspi-core/src/measure/` (`session.rs`, `safety.rs`, `preflight.rs`, `plan.rs`, `runs.rs`, tests), its `pub mod`
line, the `MeasureListener` in `callbacks.rs` under a marker, the `measure` screen row in `assets/modes/modes.json`, its
notice rows, binding wiring (C, UniFFI, wasm: `crates/dspi-wasm/src/measure.rs`), and **the fake-device hooks its tests
need** (RC11): a new `crates/dspi-transport/src/fake/measure_hooks.rs` with its `mod` line in `fake.rs` under a marker
(set input and output peaks, inject a foreign write, a volume or input-format notification, a lost lock, a preset load,
read the generator's schedule). **Implements:** sections 1.1, 1.2, 1.3 and 1.5: `measure_preflight(setup) ->
Preflight {blockers, warnings}`, `measure_start(setup) -> MeasureState`, `measure_capture_push(first_frame, samples,
rate_hz, clipped)` (safety checks first, O(block)), `measure_tick()`, `measure_next_position()`, `measure_stop()`,
`measure_state()`, `measure_result() -> document text`; **the run kinds walked, summed (with an invert mask), single live
sweep and finite pink** (RC11), all under the same S-rules, for P5TUNE and P5LIVE to call; the setup (outputs with roles,
speakers and alignment groups, placement, the `mic` document, stimulus overrides, ceilings within the hard caps,
capture rate); the device lock with lease renewal; the pre-roll, the ramp (S3 incl. fail-closed and the minimum of
ways), cycle identity from the START frame and the status poll, analysis slices in the gaps or after the stop, positions
and reseats, the post-roll, the result document; every S-rule incl. S13; the generator only through `siggen_set_config`
and `siggen_control`. **Done when:** with P5SIM's world rendered from the session's own generator configuration and
pushed as capture with frame indices, on the demo device: a three-output walked run gives responses within ±0.5 dB of
the world's truth (100 Hz to 10 kHz), arrivals within ±10 µs and "timing reliable"; a capture with a dropped block
aborts; a run with a slow ratio wander beyond the gate is "timing unreliable"; S1 to S13 each have a test: preflight
refuses `vol.user` -6 dB, a `High` output with a 500 Hz or a 6 dB/octave high-pass and no profile, crossfeed on a
measured pair, loudness on, input audio at -40 dBFS, read-only mode, a Viewer through the fake hub (denied), a locked
`measure` page, a pending foreign guard; a ramp stops at the SPL cap with a calibrated microphone and at the ceiling
without; a ramp whose microphone does not follow a step stops and asks; a walked run plays at the minimum of its ways'
ramp levels; a capture clip, an output peak at 0 dBFS, a limiter gain-reduction movement, a foreign preset load, a
foreign write on a measured output, a host volume change, an input-format change, a lost lock, a frame gap and a
cycle-identity mismatch each abort with exactly one STOP_NOW and no other write; no generator configuration ever has
`RAW`, an infinite repeat, a pink duration above 60 s or a `level_db` above the cap (a property test over setups); a
setup above `MAX_CAPTURE_SAMPLES` is refused at plan time; the lock is renewed and released; a listener calling back
into `DspiCore` does not deadlock; the wasm build runs a whole session caller-driven (no thread). **Deps:** P5TIME,
P5RESP, P5CAL, P5SIM, P5MDOC (P5-A); P5DOCK's `SectionModel` (same wave: P5SES uses a test `SectionModel` until P5DOCK
merges; the merge swaps one line). **Clients:** P5LIVE, P5TUNE, P5DEMO, P5SPIKE, the screens.

**P5WCAP: Web: the microphone.** [TODO] Size 2. Wave P5-B. **Owns:** new `apps/web/src/core/capture.ts`,
`apps/web/src/core/capture-worklet.ts`, a development-only spike page `apps/web/tools/spike/measure-spike.html` (and its
script; never in the built app), `test/unit/capture_p5wcap.test.ts`, `test/e2e/capture_p5wcap.spec.ts`, the Playwright
launch flags for a fake microphone in the web CI config under a marker. **Implements:** input device list and choice
(labels after permission), `getUserMedia({audio: {deviceId, echoCancellation: false, noiseSuppression: false,
autoGainControl: false, channelCount: 1}})`, an `AudioContext` with `sinkId: {type: "none"}` where supported and no
forced `sampleRate` in Firefox, an `AudioWorkletNode` with `numberOfOutputs: 0`, blocks of about 20 ms with their first
frame (`currentFrame`), the context's rate and a clip flag, posted on a MessagePort transferred to the core Worker;
start, stop, device loss as an event; the secure-context and permission refusals as notice keys; nothing played. The
spike page: open a device through the existing Worker, run `measure_start` with the start level only, push capture, show
and download the timing residuals (for P5SPIKE). **Done when:** unit tests on the block framing and frame continuity; e2e
in Chromium with a P5SIM WAV (content and frame indices) and Firefox (flow): a denied permission is the notice, a stop
ends the track, no node is connected to a destination and `sinkId` is "none" where supported (asserted). **Deps:** none
(the spike page needs P5SES's calls at run time, merged in the same wave). **Clients:** P5WMS, P5SPIKE.

**P5QCAP: Qt: the microphone, and the image.** [TODO] Size 3. Wave P5-B. **Owns:** `ci/container/Containerfile.qtport`
(Qt Multimedia for Linux and MinGW), `ci/container/images.conf` (QTPORT and COREMERGE tags bumped), new
`apps/desktop/qt/src/platform/AudioCapture.{h,cpp}`, its cmake wp file, the Windows packaging lines for the Multimedia
module and its native audio backend (no FFmpeg plugin bundled), `apps/desktop/qt/smoke/` one capture smoke test, a
`--measure-spike` development switch (residuals to a file, start level only; for P5SPIKE). **Implements:** a
`QAudioSource` reader off the UI thread producing mono `f32` blocks with their first frame (the processed-frame count),
the rate and a clip flag; the input device list; device loss as a signal; nothing ever played. **Done when:** the images
build with `ci/container/build-images.sh` (podman, nothing installed on the host; if the agent may not build images it
reports BLOCKED and the coordinator builds them before the wave); `QMediaDevices::audioInputs()` is shown to work on the
Windows build with the bundled native backend (an offscreen check under Wine is enough if the qt-windows step has one,
else the coordinator's check in P5SPIKE); `--step qt`, `qt-offscreen` (the reader fed from a WAV through a test seam)
and `qt-windows` pass; the licence inventory lists Qt Multimedia. **Deps:** none. **Clients:** P5QUI, P5SPIKE.

**P5SPIKE (the coordinator with the owner, not an agent WP).** Size 0.75. **Right after P5-B** (RC5g): the timing
approach's one real risk is tested before the tune is built on it. The owner's microphone and the K340 PCB unit, the
generator at the **start level only** (-60 dBFS, the owner present), walked runs of two outputs: in Chromium and Firefox
through P5WCAP's spike page (via `uconsole web`), and in Qt through `--measure-spike`. Recorded in `work/p5spike/REPORT.md`:
the clock ratio, every cycle's residual, frame gaps seen, the timing-quality verdict per client. **If any client's
residuals exceed the gate, P5-C starts only after the coordinator and the owner decide** (a different capture path for
that client, or magnitude-only there). **Owner action required** (section 7).

**P5ARRIVAL: the analyser after P5SPIKE.** [IMPLEMENTED on wp/P5ARRIVAL, 2026-11-18] Size 1. After P5SPIKE's offline analysis
(`work/p5spike-analysis/REPORT.md` section 6): the arrival from the noise-weighted band-limited envelope, the
leave-one-out refinement within half a carrier period, the envelope onset, the phase check per third-octave band at
the transitions with its bound as degrees or time, typed findings and the clock-wander diagnostic (section 1.1 steps
6 and 7). **Owns:** `crates/dspi-measure/src/ir.rs` (and `ir/`), `testdata/p5/spike/` excerpts of two real runs.
**Done when:** the real excerpts give residuals of 13.8 and 4.1 µs (the offline estimate 14.2 and 4.4; the old
refinement 81.6); a two-lobe tweeter world with 75 ppm clock wander makes P5TIME's refinement hop a carrier period and
this one not; P5SIM's ratio wander of 55 to 100 ppm is reported as wander, a slip between passes as a slip, the clean
world with no finding; an LR4 woofer at -50 dBFS in P5SIM's noise is identified (P5TIME's search aborted it); every
P5-A and P5-B timing test unchanged.

### Wave P5-C: tune, live, hub, demo

**P5TUNE: Auto-tune.** [TODO] Size 5. Wave P5-C. **Owns:** new `crates/dspi-core/src/measure/tune.rs` (and
`measure/tune/`), its `mod` line in `measure.rs` under a marker, its notice rows, binding wiring. **Implements:** section
1.4: `measure_layout_proposal() -> Layout` (with each output's programme feed), `measure_tune_propose(measurement id or
text, layout, options) -> TuneProposal {id, writes (path, indices, value, words), curves (before, predicted), held,
withdrawn (with reasons), warnings}`, `measure_tune_sum_check(proposal)` (the mandatory summed runs through P5SES; 1.5 dB
RMS; withdraws timing writes on failure), `measure_tune_apply(proposal id, guard_s) -> GuardedApply` (**the preflight
again**, the "before" snapshot, the guard, the undo group, `write_param` per write; refused unless the sum check passed
for every timing write), `measure_tune_verify()` (walked and summed runs), `measure_tune_keep()` (`apply_confirm`, the
"after" snapshot with its `tune` block, the measurement's `tunes` row) and `measure_tune_ab()`; proposals pre-judged
against the registry, the limits profile and the platform (S9); a proposal for another serial refused; a measurement
whose chains differ from the device's is re-predicted against the current chains (OD-P5o), one older than 30 days or of
another placement warned. **Done when:** with P5SIM's three-way stereo world on the demo device: the full flow from
layout to keep, the predicted and the verified sums within 1.5 dB RMS 200 Hz to 10 kHz, the tweeter's simulated 0.3 ms
offset and an inverted mid corrected, an existing 3 ms common delay kept, the room's bass modes reduced within ±2 dB 30
to 300 Hz; a world whose summed run disagrees (a simulated slip the gate missed) gets its timing writes withdrawn and
the EQ kept; a measurement marked "timing unreliable" gets no timing writes; every write in the history as one
"Auto-tune" step, one undo puts every value back; Revert and the guard's timeout restore the "before" state byte for
byte (the bulk image compared); an apply after programme audio started is refused by the re-run preflight with nothing
written; a limits profile with a 300 Hz high-pass minimum and a 3 dB boost cap is never exceeded and the proposal says
what it held; a read-only client and a Viewer get no proposal applied (`ReadOnly`, `Denied`) with nothing sent; the A/B
toggles; a deterministic proposal golden. **Deps:** P5SES, P5DOCK (P5-B); P5FIT, P5XO (P5-A); P5SPIKE's go.
**Clients:** P5WAT, P5QUI, P5TM.

**P5LIVE: Live response and direct adjustments.** [TODO] Size 2. Wave P5-C. **Owns:** new
`crates/dspi-core/src/measure/live.rs` (and `measure/live/`), its `mod` line under a marker, its notice rows, binding
wiring. **Implements:** T9 and T5: `measure_live_start(output, mode: sweep | pink_rta, smoothing)` (P5SES's single live
sweep and finite pink run kinds, the session's safety and lock; magnitude-only), `measure_live_frame() -> LiveFrame
{measured, target, predicted?, rta?}`, `measure_live_predict(pending writes)`, `measure_live_cut_peak(output, freq_hz)`
(one band from P5FIT on a one-octave span, free band only, previewed, then one undo group),
`measure_live_fit_range(output, f1, f2)` (a preview, then one undo group); hold and compare for the microphone RTA;
averaging presets. **Done when:** on the simulated world: a live frame per sweep; an EQ edit shows its predicted curve at
once and the measured one after the next sweep, within 1 dB; "cut this peak" on a simulated 6 dB peak gives one band and
one undo step; pink-noise RTA reads the world's response within 1 dB (1/3 octave); pink is re-armed and never longer
than 60 s; stopping stops the generator; the lock is held while live and released after; no write without preview.
**Deps:** P5SES (P5-B), P5FIT (P5-A). **Clients:** P5WMS, P5QUI.

**P5HUB: Measurement documents on the hub.** [TODO] Size 2.5. Wave P5-C. **Owns:** new `crates/dspi-hub/src/measurements.rs`
(and `measurements/`), its registration line in `service.rs` and `ServiceConfig::measure` under markers, the
`measurements/` block in `diag.rs` (the store's index only: names, kinds, serials and sizes), new
`crates/dspi-core/src/hub_server/measure.rs` (core calls) with its `pub mod` line, binding wiring. **Implements:** cap
`hub_measure`; `measure.doc.list {kind, serial?}` (every role), `measure.doc.get {doc}` (every role), `measure.doc.put
{document}` (Control, Admin), `measure.doc.rename`, `measure.doc.note` (Control, Admin), `measure.doc.delete` (Admin),
event `measure.doc.changed {v, hub, kind, serial?, doc, change}`; bounds 1 MiB a measurement and 256 KiB a mic or target
(the WP checks a 1 MiB document against the DSPi Link message limit and, past it, adds `seq`/`last` chunking to `get`
and `put` as `fw.image.put` has), 100 measurements per device, 32 microphones,
64 targets, 64 MiB in all; the snapshot store's rules (the hub's copy wins, never an overwrite: `conflict`); no new error
code, no hub-own write; core calls `hub_measure_docs`, `hub_measure_doc_get/put/rename/note/delete` and the merged list
in `measure_docs` (local and hub, the hub's winning), `Unsupported` without the cap and nothing sent. **Done when:** tests
through the real hub in-process: each message by role, the bounds incl. a 1 MiB document round trip, a conflict, the
event, the corrupt-file rule, with and without TLS (`DSPI_HUB_TEST_TLS=on` and `compat`); the core calls against a hub
without the cap are `Unsupported`; the merge prefers the hub's copy. **Deps:** P5DOCK (P5-B), P5MDOC. **Clients:** the
web and Qt document lists.

**P5DEMO: The simulated room in the demo.** [TODO] Size 1. Wave P5-C. **Owns:** new `crates/dspi-core/src/measure_sim.rs`
(feature `measure-sim`, its line in `crates/dspi-core/Cargo.toml` under a marker), the feature in the demo builds of
`crates/dspi-wasm/Cargo.toml` and the Terminal's and Qt's demo mode under markers. **Implements:** on a demo connection
only, a `SimCapture` that renders P5SIM's "three-way speaker in a room" world from the generator configuration the
session started and feeds `measure_capture_push` with frame indices, labelled "Simulated microphone" everywhere; never
on a real device (the connection kind is checked; a test proves it). **Done when:** a demo-device session through each
binding produces a measurement without a microphone; a real (fake-USB-kind) connection refuses the simulated microphone.
**Deps:** P5SES, P5SIM. **Clients:** the screens' e2e, the user's demo mode.

### Wave P5-D: screens and the Terminal

**P5WMS: Web: measure, results, live.** [TODO] Size 3. Wave P5-D. **Owns:** new `apps/web/src/screens/measure/`
(`MeasureScreen.tsx` with a tab slot for P5WAT, `setup.tsx`, `run.tsx`, `results.tsx`, `live.tsx`, `graph.tsx`,
`model.ts`), the Worker's routing of capture blocks to `measure_capture_push` in `apps/web/src/core/worker.ts` under a
marker, the TypeScript records in `apps/web/src/core/measure-protocol.ts` (new), one marked line in
`apps/web/src/ui/screens.tsx`, `test/unit/measure_p5wms.test.ts`, `test/e2e/measure_p5wms.spec.ts`. **Implements:** setup
(microphone and its calibration import, placement, coupler, outputs, roles and alignment groups), preflight in words,
the ramp and the run with progress and a Stop that is always visible, results (measured, target, predicted; smoothing;
positions and spread; untrusted points faint; "timing unreliable" shown), the live view with direct adjustments, the
microphone RTA with hold and compare; documents saved to the hub (or downloaded as text without one). **Done when:** e2e
on the demo device with the simulated microphone (P5DEMO) and with the fake media WAV (P5WCAP): a measurement end to
end, the curves drawn, a "cut this peak" undoable, Stop stops the generator, a Viewer through the hub sees the passive
RTA only, 375 px. **Deps:** P5SES, P5LIVE, P5DEMO, P5WCAP. **Clients:** P5WAT.

**P5QUI: Qt: measurement, live, auto-tune, documents.** [TODO] Size 4.5. Wave P5-D. **Owns:** new
`apps/desktop/qt/src/controllers/MeasureController.{h,cpp}`, `TuneController.{h,cpp}`, `apps/desktop/qt/qml/measure/`,
the registrar lines under markers, `apps/desktop/qt/smoke/SmokeMeasure.cpp`, its cmake wp file. **Implements:** P5WMS's
and P5WAT's screens on Qt over the C ABI, capture through P5QCAP's `AudioCapture`, the tick from a timer on a worker
thread, the graph on the existing graph renderer (target dashed, predicted dotted). **Done when:** offscreen smoke on the
demo device with the simulated microphone: measure, live adjust and undo, tune applied and reverted; `qt-windows`
builds; translations extracted (`--step qt-ts`). **Deps:** P5QCAP (P5-B), P5TUNE, P5LIVE, P5HUB, P5DEMO (P5-C).

**P5TM: Terminal.** [TODO] Size 1. Wave P5-D. **Owns:** new `apps/terminal/src/measure.rs` (its `mod` line below TM15's
macro block), help text. **Implements:** `uconsole measure mic import FILE`, `measure target import FILE`, `measure
docs [--kind]`, `measure show ID`, `measure export ID --output N` (REW text), `measure tune ID [--dry-run] [--guard S]`
(the proposal printed in words, withdrawn writes with reasons; applied guarded with a Keep prompt), `--demo` with the
simulated microphone. No live capture in the Terminal (OD-P5m). **Done when:** command tests against the demo device: a
mic imported and listed, a demo measurement shown and exported, a tune printed, applied and reverted, exit codes for
`ReadOnly`, `Denied` and `LimitExceeded`. **Deps:** P5DOCK, P5SES, P5TUNE, P5DEMO.

### Wave P5-E: the web tune, close-out

**P5WAT: Web: auto-tune and documents.** [TODO] Size 2.5. Wave P5-E (after P5WMS has merged: it fills P5WMS's tab slot
and uses its `graph.tsx`). **Owns:** new `apps/web/src/screens/measure/{tune.tsx,docs.tsx,tune-model.ts}`,
`test/unit/measure_p5wat.test.ts`, `test/e2e/measure_p5wat.spec.ts`, one marked line in `MeasureScreen.tsx`'s tab slot.
**Implements:** the tune wizard of section 1.4 (layout with alignment groups and programme feeds, preflight, measure,
each step's result, the summed check, the proposal as words with the held and withdrawn lists, apply with the guard's
countdown, verify, keep or revert, A/B); the documents list (local and hub merged, rename, note, delete by role), the
microphone and target libraries, REW export. **Done when:** e2e on the demo device with the simulated three-way world: a
tune proposed, applied, verified and kept; another reverted; a withdrawn timing write shown with its reason; the A/B
toggles; Control cannot delete a hub document; 375 px. **Deps:** P5WMS (P5-D), P5TUNE, P5HUB.

**P5FIN: Docs built and the manual.** [TODO] Size 0.75. Wave P5-E, after P5WAT (same review batch). **Owns:** the built
marks in `docs/integrator/measurement.md` and `hub-api.md` section 22, a user manual page `docs/manual/measurement.md`
(placing the microphone, the couplers, what the safety rules mean, what auto-tune changes and how to go back),
`docs/PARITY.md` marks, this plan's `[DONE]` marks. **Done when:** links resolve; the no-host-noun check passes.
**Deps:** P5WAT.

**P5HIL (the coordinator with the owner, not an agent WP).** Size 1. After P5-E. One real session on the K340 PCB unit
(`BB66A44F2AC1E7B6`) with the owner's measurement microphone and its calibration file, in the web console through
`uconsole web` and in Qt: (1) the calibration file imported and its sensitivity checked against the vendor's sheet;
(2) optionally, with the owner's agreement to that build, a `DSPI_LOOPBACK` firmware on the board for a **silent
electrical check** of the stimulus model and the grid timing (output slot 0 back over USB; S-a), then the reviewed image
back, under the flashing rules; (3) the safety paths with the generator at -120 dBFS: Stop, microphone unplugged, a
preset load from another console, a host volume change, each aborting with one STOP_NOW; (4) the ramp from the start
level with the owner listening, the ceiling of **headphone mode applied to every K340 output** unless the owner says
which outputs carry the attached headphones (memory `test-volume-limit`); (5) a measurement per placement the owner can
set up; (6) with speakers, a two- or three-way auto-tune with its summed check, kept and then reverted; (7) the captures
exported as WAV into `work/p5hil/` and, with the owner's consent (room sound), two promoted to replay goldens in
`crates/dspi-core/testdata/p5/hil/`. Recorded in `work/p5hil/REPORT.md`. **Owner action required** (section 7).

### Later batch (not in phase 5's waves; owner's order: web and Qt first)

| WP | Size | What |
|---|---|---|
| P5AN: Android | 4.5 | `AudioRecord` with `UNPROCESSED` (USB microphones through Android's USB audio), the screens in Compose over UniFFI, the simulated microphone in demo |
| P5SW: Swift | 5 | `AVAudioEngine` input on macOS and iOS, SwiftUI screens, on the Mac VM |

### Firmware-dependent items (`bofr-main`, extension opcode 0x00)

**Nothing in phase 5 needs new firmware.** The generator is upstream (v1.1.5 and later) and on `bofr-main`; the
extension channel is not used.

| Item | Firmware | WP | On stock firmware |
|---|---|---|---|
| Stimulus | the generator, vendor opcodes 0xA4 to 0xA8, `NOTIFY_EVT_SIGGEN_STATE` | P5SES | the same (upstream since v1.1.5); before v1.1.5: passive microphone RTA only, and the screens say why |
| Abort on clip | `fetch_status` peaks and clip flags; 0xE0 clip events (`EXT_NOTIFY_FEATURES` bit 1) | P5SES | peaks at the clients' poll rate (S4 uses the peak) |
| Aborts on foreign changes | `NOTIFY_EVT_MASTER_VOLUME`, `PARAM_CHANGED`, `BULK_INVALIDATED`, `INPUT_FORMAT` | P5SES | the same |
| Unmodelled processing (S13) | crossfeed, subharmonic, psychoacoustic bass, tube, loudness, limiter (`bofr-main`) | P5SES | whichever of these the firmware has; the preflight reads what the mirror holds |
| Delay range | `Platform::max_delay_ms` 21 (RP2040) or 42 ms (RP2350) | P5XO, P5TUNE | the same |

Considered and **not planned**: a generator time stamp or start-at-frame (`EXT_FRAME_COUNTER`/`EXT_APPLY_AT_FRAME` exist
only on alink USB-direct builds; the START frame and the status poll pin the cycle index); a firmware ceiling on the
generator's level or a `RAW` that keeps a protective high-pass (worth proposing in a later firmware batch; this phase
never uses `RAW`); more PEQ bands (FW7); a microphone into the DSPi (R2 chose a host microphone). `DSPI_LOOPBACK` is a
debug build used at most once, in P5HIL, with the owner's agreement.

### Waves

No two WPs in one wave own the same file, and no WP depends on another WP of its own wave except where an entry says
how the seam covers it (P5SES on P5DOCK's `SectionModel`; P5FIN after P5WAT in P5-E, one review batch). In P5-A nobody
edits `dspi-measure`'s `Cargo.toml`, `lib.rs`, `Cargo.lock` or `deny.toml` (the seam has them). In P5-B only P5DOCK adds
the `dspi-measure` dependency to `dspi-core` and `dspi-session`; P5SES's merge follows P5DOCK's. Registration lines, `mod`
lines, binding wiring, `screens.tsx` and the Qt registrar are additive under markers. `diag.rs` is edited once (P5HUB,
wave C). `notices.json` takes one block per WP. `fake.rs` takes one `mod` line (P5SES).

| Wave | WPs | Size | Needs |
|---|---|---|---|
| P5-A | P5TIME 3.5, P5RESP 2, P5CAL 2.5, P5FIT 4, P5XO 3.5, P5SIM 2.5, P5MDOC 2.5 | 20.5 | `main` with this plan's seam |
| P5-B | P5DOCK 2.5, P5SES 6, P5WCAP 2, P5QCAP 3 | 13.5 | P5-A; the coordinator builds the bumped Qt images before or with P5QCAP |
| — | **P5SPIKE** (coordinator and owner, 0.75) | (0.75) | P5-B merged; its result gates P5TUNE |
| P5-C | P5TUNE 5, P5LIVE 2, P5HUB 2.5, P5DEMO 1 | 10.5 | P5-B and P5SPIKE; then `ci/macvm.sh macos` once |
| P5-D | P5WMS 3, P5QUI 4.5, P5TM 1 | 8.5 | P5-C |
| P5-E | P5WAT 2.5, then P5FIN 0.75 | 3.25 | P5-D; then P5HIL (coordinator and owner, 1) |
| | **Total: 20 agent WPs** | **56.25** (+0.75 P5SPIKE, +1 P5HIL) | maths 18 (P5-A less P5MDOC), core 16.5, hub 2.5, web 7.5, Qt 7.5, Terminal 1, docs 3.25; section 8 about 5.5; later batch 9.5 |

Each wave ends with one Fable batch review; the full gate runs after P5-C and after P5-E. P5-A is the widest; if fewer
agents run, P5TIME and P5MDOC go first (they gate P5SES and P5DOCK). P5QCAP's image bump should be built early, since
every later Qt and Rust build uses the new images. P5-D's Qt WP is the longest pole; P5-E can start when P5WMS has
merged even if P5QUI is still running.

---

## 3. The security bar and the compatibility policy

**PLAN-SEC applies unchanged.** The new hub family is on `/dspi/v1` over SECSRV's TLS with pinning; the web measures
through the hub-served https page or the `uconsole web` proxy (a secure context, which `getUserMedia` requires). On top
of that:
- **Test audio cannot reach another output** (S1): there is no host player in any client; the generator is inside the
  DSPi. The web capture graph has no output node and no output device where the browser allows it.
- **Levels are bounded in three places:** the preflight (`vol.user` at most -12 dB, the generator ceiling per mode, the
  hard caps, no unmodelled processing), the ramp (the run's own stimulus, closed loop on the microphone, fail-closed,
  the SPL cap, the minimum of the ways), and the aborts (clip, SPL, limiter, frame gaps, anything foreign). The session
  writes no volume.
- **Roles:** playing the generator is Control or Admin through a hub (the existing policy); a Viewer gets the passive
  RTA. `measure.doc.put/rename/note` Control and Admin, `delete` Admin, reads every role.
- **Every write is judged:** a tune's writes go through `write_param` after a second preflight, so the client read-only
  mode, a read-only connection, the installer lock, the limits profile and the device lock decide each one; the
  proposal is pre-judged so nothing refused is offered; timing writes need the timing gate and the summed check.
- **Untrusted input:** calibration and target files, documents from the hub or another client, WAV files and captured
  audio are data: bounded, parsed without panics, fuzzed. A document never carries an instruction the core acts on: a
  measurement proposes, a person applies.
- **Privacy** (S12): no raw audio leaves the core's memory unless the user exports a WAV; documents hold derived curves
  only; the browser's microphone permission is asked only when the user starts a measurement or the RTA.
- **No new error code, no hub-own write**, bounded stores (1 MiB a measurement, 256 KiB a mic or target, 100 measurements
  a device, 32 microphones, 64 targets, 64 MiB), files 0600 and atomic, the corrupt-file rule. `deny.toml` names the
  new crate; `rustfft` and its dependencies are MIT or Apache-2.0.

**The compatibility policy (owner, 2026-11-15).**
- **No shims between our components:** three new `meta.kind`s in the existing family; the hub family is new under its
  cap; a core without the call fails the build, not the run.
- **Our clients against an upstream Weeb Labs hub keep working:** every `hub_measure_*` call is `Unsupported` without the
  cap and sends nothing; documents then stay local; the measurement itself needs only the generator opcodes, which an
  upstream hub forwards by its policy (class `control`).
- **USB to the hardware keeps working:** nothing on the wire changes.
- **Our hub against upstream clients, best effort:** the new messages are unknown to them and ignored.

---

## 4. Testable without hardware, and what needs the owner

| What | How it is tested | WP |
|---|---|---|
| Sweep model, cycle identity, drift, timing gate, deconvolution, averaging, windows | synthetic signals with known answers; an independent re-implementation of `siggen.c`'s recurrence; P5SIM's capture faults; `proptest_*` | P5TIME |
| Responses, smoothing, RTA, export | known spectra; `proptest_*` | P5RESP |
| Calibration, targets, positions | hand-written fixtures per format; fuzzing | P5CAL |
| Fit, alignment, crossover corner | synthetic drivers and rooms with designed optima; bounds as properties; quantised-proposal goldens | P5FIT, P5XO |
| The whole run, every S-rule | the demo device plus its measurement hooks, with P5SIM's world fed as capture with frame indices; the fake hub for roles and locks | P5SES, P5TUNE, P5LIVE |
| Documents and stores | store tests as P1SN's; the real hub in-process | P5DOCK, P5HUB |
| Browser microphone | Chromium's fake media device with P5SIM WAVs (content), Firefox's fake stream (flow); no destination node | P5WCAP, P5WMS, P5WAT |
| Qt microphone | the reader fed from a WAV through a test seam; offscreen smoke | P5QCAP, P5QUI |
| **The capture clock and slips of a real USB microphone in real browsers and Qt** | **P5SPIKE**, right after P5-B, start level only | P5SPIKE |
| **Real acoustics, a real microphone's calibration, real levels, the K340** | **P5HIL** | P5HIL |

What the simulator cannot prove: that the vendor's sensitivity convention gives the right dB SPL with the owner's
microphone; that the browsers' and Qt's capture really has processing off and keeps its frame count honest on the
owner's machines (P5SPIKE); that the ramp's start levels are quiet enough in a real room and on a coupler; that a tune
sounds better (A/B by ear, the owner's judgement).

---

## 5. Decisions

Every decision carries a firm recommendation; following the owner's rule of 2026-11-15 they are taken as recommended
unless the owner objects. Section 7 lists what remains for the owner (actions, no blocking question).

| # | Decision | Options | Recommendation | Why |
|---|---|---|---|---|
| **OD-P5a** | **The stimulus** | (1) the DSPi's generator only; (2) host playback through the DSPi's USB audio; (3) both | **(1)** | The generator is on stock firmware since v1.1.5, sample-exact and repeatable, runs without a host stream, and cannot fall back to another output: the owner's test-audio rule is met by construction. (2) needs a guarded player with a watchdog on five platforms and is exactly what once played on the owner's speakers |
| **OD-P5b** | **The timing reference and clock drift** | (1) the generator's cycle grid in one walked run of at least two passes: a common slope and one intercept per output, drift removed per cycle in the frequency domain, cycle identity pinned by the START and the status poll, frame-indexed capture, a timing quality gate, and a mandatory summed check before any timing write; (2) a loopback channel (a debug firmware build only); (3) an acoustic reference speaker (REW's way); (4) a firmware time stamp | **(1)** | One DAC clock and one sample-exact schedule for all ways; the unknown latency cancels within a run; the failure modes (capture slips, cycle identity) are detected explicitly, and the summed run does not depend on the capture clock at all. P5SPIKE tests it on real hardware before the tune is built on it. (2) is not release firmware (used once in P5HIL as a check); (3) needs two simultaneous signals the generator does not make; (4) needs new firmware |
| **OD-P5c** | **Where the maths lives** | (1) a new pure crate `dspi-measure` (depends on `dspi-proto`), filter responses through a `SectionModel` the core implements with its own cascade; (2) modules inside `dspi-core`; (3) inside `dspi-session` | **(1)** | Pure, property-tested and fuzzed in isolation, built for wasm on its own; the `SectionModel` keeps one filter model (the graph's) so a prediction can never disagree with the curve the user sees |
| **OD-P5d** | **The FFT** | (1) `rustfft` with `default-features = false` (scalar), no `realfft`; (2) `realfft` over `rustfft` with SIMD; (3) our own | **(1)** | MIT/Apache-2.0, pure Rust; scalar so native CI hosts and wasm compute alike (a run-time SIMD choice would make results differ between machines, RC10); speed at 2^19 is ample for analysis in the gaps |
| **OD-P5e** | **The capture seam** | (1) sans-IO: the client pushes frame-indexed microphone blocks, the core's state says when it wants them, `measure_tick` drives time; (2) a `Capture` trait the core calls; (3) audio in the core | **(1)** | The wasm core cannot open a microphone; the frame index makes slips visible; the core stays free of audio APIs and threads |
| **OD-P5f** | **Where a measurement runs for the web and the fleet** | (1) where the microphone is (the browser's core, the desktop app); the hub stores documents and arbitrates through the device lock; (2) a hub-side measurement with the microphone on the hub's machine | **(1)** | The microphone belongs at the listening position, not at the rack; the hub adds no write and no audio stack |
| **OD-P5g** | **The documents** | (1) kinds `measurement`, `mic`, `target` in the schema-1 family, compact rounded curves, no IR excerpt, 1 MiB for a measurement and 256 KiB for the others; the tune's result as two snapshots; (2) a new format; (3) raw captures or long IRs in the documents | **(1)** | PRO1's rule (one family); RC7's sizes (a three-way stereo measurement under 100 KiB); the snapshots make A/B, recall and the hub's store work unchanged; raw audio is private and large |
| **OD-P5h** | **How a tune is applied** | (1) typed writes through `write_param` in one undo group inside a guarded apply, after a second preflight, between a "before" and an "after" snapshot; (2) a bulk image recalled as a snapshot | **(1)** | Each write is judged; one undo reverts it; the guard reverts on timeout; the snapshots give A/B. (2) ends the undo history and is judged only as a whole |
| **OD-P5i** | **Speaker safety** | (1) the S-rules of section 1.2; (2) user-set levels only | **(1)** | R5 asks for exactly this; numbers are conservative defaults the user can lower and raise only to hard caps; RC3 and RC4's tightening makes the ramp fail closed and the tweeter rule meaningful |
| **OD-P5j** | **Delay normalisation and polarity** | (1) the alignment group's smallest delay keeps its current value, only additions on the others, within the platform's and the profile's range; polarity as the net invert of the feeding crosspoints; (2) the latest way gets 0 | **(1)** | (2) would erase a speaker's existing common delay (left/right distance, sub or lip-sync alignment) (RC6); the matrix invert is the device's only polarity control, and the generator does not pass through it, so summed runs use the generator's `invert_mask` |
| **OD-P5k** | **Crossover tuning's reach** | (1) the corner within half an octave by default, wider by choice but only inside both drivers' trusted range; the current family and order kept; levels cut only and re-normalised; "align only" available; (2) a free search including families and orders | **(1)** | A tune refines what the installer built; the family and order search is section 8 |
| **OD-P5l** | **The EQ fit's policy** | (1) free bands only by default; speakers 20 to 300 Hz by default, headphones 20 Hz to 10 kHz; boost at most +3/+6 dB and the profile's cap; no filling of variable or deep dips; headroom by an equal output-gain cut; (2) all 10 bands, full range | **(1)** | Never overwrite a user's band silently; above the Schroeder region a single-position correction usually sounds worse; boosting nulls wastes headroom |
| **OD-P5m** | **Which clients first** | (1) web and Qt screens, the Terminal for documents and the tune from a document, Android and Swift a later batch; (2) every client now | **(1)** | The owner's order; every client gets the same calls |
| **OD-P5n** | **Concurrency** | (1) the device lock "Measuring" with a 30 s lease renewed by `measure_tick`; foreign writes, volume or format changes, or generator stops abort the run; one session per core; (2) no lock | **(1)** | Another console changing anything mid-sweep spoils the measurement; the lease is also the dead-man switch |
| **OD-P5o** | **A measurement older than the device's state** | (1) each output's measured chain and programme feed are stored; the tune de-embeds with them and predicts against the current chain; age and placement only warn; (2) refuse when the chains changed | **(1)** | The acoustics did not change when an EQ did; the verify runs catch a moved speaker |
| **OD-P5p** | **Targets shipped** | (1) only targets defined by parameters in our own code; others imported; (2) bundle third-party curve data | **(1)** | No licence question; the importer covers every curve a user already has |
| **OD-P5q** | **The demo's simulated microphone** | (1) the demo device gets P5SIM's world, labelled, demo connections only; (2) tests only | **(1)** | The feature can be tried and every screen's e2e runs without hardware |
| **OD-P5r** | **Measurements over REST** | (1) none in this phase (reads are section 8); (2) reads now | **(1)** | Simplification; integrators can use the DSPi Link messages; reads are additive within frozen v1 later |
| **OD-P5s** | **Qt's audio input** | (1) Qt Multimedia `QAudioSource` (an image bump, native backends, no FFmpeg plugin); (2) ALSA/PipeWire and WASAPI by hand | **(1)** | One reader for Linux and Windows; the image bump is a one-time cost |
| **OD-P5t** | **When the timing approach meets real hardware** | (1) P5SPIKE right after P5-B, start level only, gating P5TUNE; (2) only in P5HIL at the end | **(1)** | The approach's one real risk must not be found last (RC5g) |

---

## 6. What not to do in phase 5

- Play test audio from any host, or build a player "for old firmware"; use `SIGGEN_FLAG_RAW`; stage an infinite generator
  run; write a volume, a mute or a gain while measuring; raise any output gain in a proposal.
- Bypass a protective high-pass to measure a driver (de-embed instead), or propose a high-pass below the S6 rule.
- Measure through crossfeed, subharmonic, psychoacoustic bass, tube, loudness or an engaging limiter (S13).
- Offer a delay, polarity or crossover write from a run that failed the timing gate or without a passed summed check.
- Compare arrivals across separate runs.
- Write anything as a side effect of a measurement; apply a tune without a preview, a second preflight, the "before"
  snapshot and the guard; apply a tune automatically after a measurement.
- Keep, log, upload or bundle raw microphone audio; put audio in a document.
- Open an audio device in the core, start a thread for measuring, run analysis between a push's safety check and the
  next block, or use a SIMD FFT whose results depend on the machine.
- Add a new file format, a new error code, a hub-own write, or a measurement started over REST.
- Bundle third-party target or calibration data.
- Build measurement screens in Android or Swift in this phase, or a microphone on the hub's machine.
- Add firmware for this phase.
- Open a pull request to Weeb Labs for anything in this phase.

---

## 7. Owner questions

**None blocking.** Every decision in section 5 has a firm recommendation.

**Owner actions (not questions):**
1. **For P5SPIKE, right after P5-B:** be present for a short run at the start level (-60 dBFS) on the K340 with the
   measurement microphone, in Chromium, Firefox and Qt; a go for that test audio under section 1.2's rules (the standing
   go of 2026-11-14 covers flashing alink test images only).
2. **The microphone and its calibration file** (e.g. a UMIK-1), for P5SPIKE and P5HIL.
3. **For P5HIL:** the setup (headphones on which coupler, and/or speakers on the K340's amplifier outputs: which outputs,
   how many ways), which outputs carry the headphones that stay attached, a go for test audio with the owner present for
   the ramp, optionally agreement to a one-time `DSPI_LOOPBACK` build for the silent electrical check (then the reviewed
   image back), and consent (or not) for two HIL captures to become replay goldens.

---

## 8. Later: deferred from the first version (the review's simplifications)

Not dropped: each is a small WP after phase 5, only if asked (about 5.5 units in all).

| Item | Size | Why deferred |
|---|---|---|
| Near-field to far-field splice (P5CAL, P5XO's driver data) | 0.75 | near-field stays a placement and a labelled measurement |
| Family and order search in the crossover; power-response and breakup penalties | 1 | the current family is the default anyway |
| Linear and stepped sweeps in the stimulus model | 0.25 | the runs use log sweeps only |
| "Psychoacoustic" variable smoothing; minimum and excess phase display | 0.5 | fractional-octave smoothing covers the fit and the graph |
| An IR excerpt in documents and IR export | 0.75 | RC7's sizes; the responses carry what the tune needs |
| Dayton-style calibration lines | 0.25 | UMIK, REW and CSV cover the common microphones. **Done in P5DAYTON (2026-11-19):** `*1000Hz` dBV/Pa line (-70..0), format `dayton`, scale referred to a 0 dBV full scale (approximate) |
| Left/right match measure for headphones | 0.25 | the fit per side is enough for a first version |
| REST reads of measurement documents | 0.5 | within frozen v1, additive later |
| `uconsole measure analyse WAV` (offline analysis of an external recording) and 24/32-bit integer WAV | 0.75 | the Terminal keeps documents and the tune |
| Hold and compare for the device analyser | 0.5 | the microphone RTA has it |
| A kind-generic document store shared with snapshots (review S-g) | (refactor) | `measure_store` copies `snapshot_store`'s rules; unify when a fourth store appears |
