# Measurement and auto-tune

*WP P5MDOC, 2026-11-16: the contract of PRO phase 5 ([`docs/core-merge/PLAN-P5.md`](../core-merge/PLAN-P5.md)). Status
(WP P5FIN, 2026-11-19): **built**, every section, on `main` (waves P5-A to P5-D). The P5-E batch (P5WAT, P5HOLD,
P5QUI2, P5DAYTON) is built too (reviewed 2026-11-19, `work/review-p5e/REPORT.md`). Section 10 says which client offers what. The user's
page is [docs/manual/measurement.md](../manual/measurement.md).*

A DSPi is tuned with a calibrated measurement microphone instead of by ear. The **device's own test signal generator**
plays the stimulus, so no client ever plays test audio; the client contributes only the microphone's samples and its
screens. One implementation serves every client: the maths is the pure crate `dspi-measure`, the workflow the sans-IO
`measure` module of `dspi-core`. Results are documents of the `.dspipreset` schema-1 family (section 7), filed locally or
on the hub; applying a tune is the guarded apply, the undo group and the A/B compare that already exist.

Where this page and the code disagree, the code is right and the page is the bug.

## Contents

| § | Covers | Built by | State |
|---|---|---|---|
| 1 | [The signal path](#1-the-signal-path) | P5TIME, P5RESP, P5SES, P5ARRIVAL, P5RAMPFIX | built |
| 2 | [Safety](#2-safety) | P5SES, P5RAMPFIX, P5TUNE | built |
| 3 | [Placements and couplers](#3-placements-and-couplers) | P5CAL, P5SES | built |
| 4 | [Auto-tune, step by step](#4-auto-tune-step-by-step) | P5XO, P5FIT, P5TUNE, P5TRUST, P5TRUST2 | built (the hold's anchor and Keep during a verify: P5HOLD) |
| 5 | [The live view and direct adjustments](#5-the-live-view-and-direct-adjustments) | P5LIVE, P5CURV | built |
| 6 | [Locks, read-only and other clients](#6-locks-read-only-and-other-clients) | P5SES, P5CURV | built |
| 7 | [The documents](#7-the-documents) | P5MDOC (model), P5DOCK (stores and calls) | built (the `dayton` format: P5DAYTON) |
| 8 | [On the hub](#8-on-the-hub) | P5HUB | built ([hub-api.md section 22](hub-api.md#22-measurement-documents)) |
| 9 | [What the core never does](#9-what-the-core-never-does) | all | built (the S-rules' tests) |
| 10 | [In the clients](#10-in-the-clients) | P5WCAP, P5QCAP, P5WMS, P5QUI, P5TM, P5DEMO; P5WAT, P5QUI2 | built |

---

## 1. The signal path

*State: built (P5TIME: stimulus and timing; P5RESP: responses; P5SES: the run; P5ARRIVAL: the arrival and the gate's
findings, after P5SPIKE).*

1. **Stimulus: the device's generator** (firmware v1.1.5 and later). It **replaces the matrix mix** on the outputs it
   plays, then passes through each output's crossover, PEQ, delay, trim and volumes. So an output's *measured chain* is
   its crossover, PEQ, output gain and delay; the crosspoints that feed it in normal use (their gain and polarity, the
   *programme feed*) are not in the generator's path and are recorded beside the chain for predictions of programme
   audio. Run kinds:

   | Kind | What plays | Use |
   |---|---|---|
   | `walked` | a log sweep, one output per cycle round-robin, 2 to 8 passes (default 3) | every per-output response; timing between the outputs of one run |
   | `summed` | the outputs of one crossover transition at once, the generator's `invert_mask` set to their net polarity | the mandatory check of predicted timing (section 4 step 6) |
   | `liveSweep` | one output, one sweep, repeated by the live view | the live view (magnitude only) |
   | `pink` | finite pink noise on one output, at most 60 s | the microphone RTA |

   Sweeps are 1 to 15 s (default 5.5 s, 2.7 s live), 20 Hz to 20 kHz, with a gap of 1000 ms for speakers and 300 ms on a
   coupler; the level comes from the safety ramp (section 2); never `SIGGEN_FLAG_RAW`, never infinite.
2. **Capture: the client's microphone**, pushed to the core in mono `f32` blocks with frame indices
   (`measure_capture_push(first_frame, samples, rate_hz, clipped)`); 48 kHz by default. **Any gap or repeat in the frame
   indices aborts the run.** The web captures through an AudioWorklet with no output node; Qt through `QAudioSource`.
3. **Safety first in the push path:** each block is checked for clipping, SPL and frame continuity before it is stored.
4. **The core renders its own copy of the sweep**, sample-faithful to the firmware's integer phase accumulator, fades and
   walk order, at 44.1, 48 and 96 kHz.
5. **Cycle identity:** the cycle index is pinned by the START acknowledgement and checked against the generator's
   status during the run; a mismatch aborts.
6. **Timing:** one arrival per cycle; one common slope (the clock ratio between device and microphone) and one intercept
   per output, over at least two passes; the ratio removed per cycle in the frequency domain; each cycle deconvolved and
   the repeats averaged coherently. Delays between outputs **of one run** are valid; nothing compares arrivals across runs.
   A cycle's arrival is the peak of the **envelope** of its deconvolved response, weighted by the pre-roll's noise and
   limited to the sweep's band, refined against the **mean of the output's other repeats** (noise-weighted, the lag within
   half a carrier period of the envelope's). When the envelope and the correlation disagree about which carrier peak is
   the arrival (a band-limited way: a tweeter), the cycle's arrival is reported **ambiguous** and left out of the fit and
   the gate's largest residual; it is never a number the gate trusts. An output's onset (the centre of its windows and
   `arrival_ms` of a result) is where that envelope first reaches 30 % of its peak.
7. **The timing quality gate:** a run is "timing reliable" only when every cycle's residual is at most 10 µs, no arrival
   is ambiguous, the capture clock did not wander inside a sweep, and the repeats agree in phase in every third-octave band
   of the check that is loud enough (20 dB per repeat, summed over the band). The check's bands are, by default, the
   run's **transitions** (where two of its outputs are both loud and within 12 dB of each other; an output in none, a run
   of one output, from 100 Hz to 10 kHz). A band's bound is the larger of 10° and 5 µs as a phase at its centre (10°
   through every usual crossover, 18° at 10 kHz). Otherwise its magnitudes are kept and it is marked "timing
   unreliable": delay, polarity and crossover proposals are refused with that reason. The summed run stays the arbiter of
   delay and polarity (step 8). The reasons come in words and, in `dspi_measure::ir::TimingQuality::issues`, typed:

   | Finding | Means | What to change |
   |---|---|---|
   | ambiguous arrival | the analyser could not tell which carrier peak is the arrival | more level or passes; it is not the capture |
   | capture slip | a cycle sits off the clock fit while its delay is steady along its sweep: frames were lost or repeated between cycles | the capture path (another client, no resampler) |
   | clock wander | "the capture clock wandered X µs / Y ppm inside its sweep against the other repeats" | a capture device with a stable clock (P5SPIKE's USB microphone wandered by 55 to 100 ppm); magnitudes are unaffected |
   | phase spread | the repeats of an output disagree in phase in a band, more than its bound | usually one of the above; else movement or noise |

8. **No loopback, no reference speaker, no firmware time stamp.** The one check that does not depend on the capture clock
   is the summed run, mandatory before any timing write is offered.
9. **Noise and trust:** a 1 s pre-roll gives the noise floor; a point is trusted (`valid`) when its SNR is above 20 dB and
   its repeat-to-repeat spread under 1 dB (the repeats' windowed responses, beyond the spread their noise explains, which
   is taken as at most 2 dB).
   Untrusted points are drawn faint and never fitted.
10. **Memory:** a capture holds at most 8 Mi frames (`MAX_CAPTURE_SAMPLES`); a run that would exceed it is refused at the
    plan step. Responses are held on a 1/96-octave grid from 10 Hz to 22 kHz (1066 points).

## 2. Safety

*State: built (P5SES, P5RAMPFIX, P5TUNE). S1 to S13 are those WPs' tests.*

| Rule | What it means |
|---|---|
| **S1** No player | The stimulus is the device's generator; no client plays test audio, nothing falls back to another output. Without a generator only the microphone RTA of programme audio is offered |
| **S2** Volume | Never written while measuring; the preflight refuses `vol.user` above -12 dB (80 % of its range) |
| **S3** Start quiet, ramp | Short walked sweeps from -50 dBFS (speakers) or -60 dBFS (headphones), +6 dB a step, until 35 dB in-band SNR or a ceiling: -12 dBFS (hard cap -6) for speakers, -30 dBFS (hard cap -20) for headphones; with a calibrated microphone also 95 dB SPL (speakers) or 90 dB SPL (coupler). **Heard in band, never broadband** (P5RAMPFIX): a sweep step is judged by the sweep's matched filter, whitened by the pre-roll's noise, in the sweep's band within 100 Hz to 10 kHz, against the same filter's output away from its peak; a pink step by its third-octave band levels against the pre-roll's; low-frequency rumble never counts as signal (`dspi_measure::hearing`). Fails closed: by start + 18 dB a step must be heard (in-band SNR at least 6 dB), once heard each 6 dB step must raise the in-band level by 3 dB, and no ramp ends on a step that is not heard, the start level only included (`measure.mic_not_hearing`, `measure.mic_not_following`, `measure.start_level_quiet`). A walked run plays at the lowest of its outputs' ramp results |
| **S4** Abort on clip | A capture sample at -1 dBFS, an output's peak at -0.5 dBFS, the SPL cap: the generator stopped at once, the run discarded |
| **S5** Abort on anything foreign | Another console's stop or write, any volume change, a rate change, programme audio on the inputs, the lock lost, a disconnect, a capture gap, a cycle mismatch, an overrun of 5 s, Stop |
| **S6** The tweeter rule | A `Mid` or `High` output is measured only through an active high-pass (defaults: `High` 1 kHz, `Mid` 80 Hz, at least 12 dB/octave; a limits profile's own values replace them). Driver data comes from de-embedding the known chain, never from bypassing it |
| **S7** Finite runs | Every run ends on its own: `repeat` at least 1, pink at most 60 s, a run at most 120 s |
| **S8** Silence first | Refused while any input peaks above -70 dBFS |
| **S9** Bounded proposals | Inside the registry, the limits profile and the platform; PEQ boost at most +3 dB (speakers) or +6 dB (headphones), cuts at most -15 dB, Q 0.3 to 10; crossover and level changes cut only; the EQ's net boost offset by an output-gain cut |
| **S10** Guarded and reversible | A tune is applied only after a second preflight, a "before" snapshot, inside a guarded apply (the planned verify plus a margin, at most 590 s) as one undo group |
| **S11** Locks and roles | A writable connection, read-only mode off, Control or Admin through a hub, the `measure` screen not locked by an installer |
| **S12** Privacy | Raw microphone audio stays in the core's memory for the run only; documents hold derived curves only; a WAV is written only when the user exports one |
| **S13** No unmodelled processing | Refused while crossfeed, psychoacoustic bass, tube, subharmonic, loudness or an engaging limiter acts on a measured output |

## 3. Placements and couplers

*State: built (P5CAL, P5SES). The JSON shape is `dspi_measure::types::Placement`, as stored in section 7.3.*

| Placement (`mode`) | Runs | Stored | Used for |
|---|---|---|---|
| `listeningPosition` | one walked run of an alignment group | each output's response with phase, arrival and timing quality | alignment, crossover, fit |
| `multiPosition` (`count` 3 to 9) | one run per position; position 1 is the only timing reference | the power average of the magnitudes, the spread per point, position 1's phase and arrival | the fit (dips that vary more than 3 dB are not filled) |
| `nearField` | one run per driver, the microphone close to it | the near-field response, labelled so | shown and compared; not used by the tune in this phase |
| `headphones` (`coupler`, `reseats` 1 to 9) | reseats, left and right each | the average of reseats with the spread, the coupler, the compensation applied | the fit per side toward the coupler's target |

Coupler types: `iec60318_4` (IEC 60318-4 / 711 ear simulator), `flatPlate`, `hats` (a head and torso simulator or
another ear coupler with pinna). Each has a built-in default target defined by parameters in our own code; no third-party
curve data is bundled, any other target or compensation is imported from a file.

## 4. Auto-tune, step by step

*State: built (P5XO, P5FIT, P5TUNE, P5TRUST, P5TRUST2, P5HOLD).* Each step shows its result before the next; the user may stop after any step with
nothing written.

1. **Layout:** speakers and ways proposed from the device's state and the matrix's pairing, each output's programme feed
   read; the user confirms it.
2. **Preflight** (section 2): every blocker in words.
3. **Measure:** one walked run per alignment group; each driver's own response is the measurement divided by its known
   chain, trusted only where the measurement's SNR at that point (which carries the chain's attenuation) is at least 20 dB.
4. **Delay and polarity**, only from a timing-reliable run; delays normalised so the group's smallest delay keeps its
   current value; polarity as the net invert of the feeding crosspoints.
5. **Crossover:** corner (1/12-octave steps, within the drivers' trusted ranges, never below S6), levels cut only, the
   current family and order kept, each candidate scored by the predicted sum against the target. A `High` way's
   high-pass is never lowered on the magnitude sum alone: with the delays kept no lower corner is a candidate, and
   every crossover write is a timing write. Each candidate's (and `align`'s) timing is held within a quarter of the
   corner's period, at least 0.25 ms, of the arrival-based delay; a held choice that would flip the polarity, disagrees
   with the window's own best polarity or lies on the hold's edge keeps the pair's delay and polarity, with the note
   "the timing could not be settled at this corner". P5HOLD: the hold is anchored on the drivers' own
   arrivals (each way's arrival less its measured chain's onset), so steep pairs at 300 Hz and 3 kHz get their timing;
   at a low corner (120 Hz) LR6, LR8 and some BW3/BW4 searches still keep theirs with that note.
6. **Summed check (mandatory):** one summed run per transition with the current chain. More than 1.5 dB RMS between the
   measured and the predicted sum withdraws every delay, polarity and crossover write.
7. **Target fit:** the free PEQ bands toward the target, within the range and caps of S9.
8. **Proposal:** every write in words, the curves before and predicted, the limits held to. Nothing is written yet.
9. **Apply:** the preflight again, "Before auto-tune" snapshot filed, `apply_guard_begin("Auto-tune", <the planned verify plus a margin>)`, one undo
   group of typed writes, each judged as any write. When the planned verify needs a guard longer than 590 s the guard
   is still 590 s and the proposal carries the warning `measure.tune.guard_capped`.
10. **Verify** inside the guard: a walked run and a summed run per transition. It passes when every summed run, verified
    sum and tuned output is within 1.5 dB RMS of its prediction. A layout without transitions has no sum: there the
    verify also fails when one of its outputs could not be compared with its prediction (review P5-D).
11. **Keep or revert:** Keep confirms the guard, files "After auto-tune" with a `tune` block and offers the A/B compare of
    the two snapshots; Revert or the timeout reverts. After a verify that did not pass, the first Keep is refused with
    `measure.tune.keep_unverified`; show it and keep only on a second Keep the user confirms. `apply_confirm` on the
    tune's guard is refused the same way (a guard banner's Keep is no way round it), a Keep within 1 s of the refusal is
    refused again (a double click is one press), and stopping the verify does not clear it. While a verify runs, Keep
    (the tune's and `apply_confirm` on its guard) is refused with `measure.tune.keep_during_verify`: wait for the
    verdict (P5HOLD). Keep before any verify has started keeps at once; say "Not verified". The measurement document
    records the tune (section 7.3, `tunes`).

## 5. The live view and direct adjustments

*State: built (P5LIVE; the passive RTA and a document's curves P5CURV).* One output's short sweeps repeated (one magnitude result per sweep) or finite pink noise with the
microphone RTA, shown with the target and with the predicted curve of a pending change. "Cut this peak" and "fit this
range" act on a point or span of the measured curve; each is previewed, then applied as one undo group (unguarded, as a
manual edit of one band). The microphone RTA (Welch, pink-referenced, 1/1 to 1/48 octave, fast, slow or infinite
averaging, hold and compare) is the client's; the **device analyser** is the firmware's and unchanged. The passive
microphone RTA (`measure_rta_passive_*`, P5CURV) is the same maths over programme audio and touches no device (section
6). A screen draws a stored measurement through `measure_doc_curves(text, {smoothing, target, targetDocument})`: each
output's curve smoothed, the target levelled to it, on the standard grid with the mask; no curve maths in a client.

## 6. Locks, read-only and other clients

*State: built (P5SES; the passive RTA P5CURV).*

- A run holds the device lock (`device_lock_acquire("Measuring", 30 s)`, renewed while the session is active, released
  at the end, on abort and on drop). Over DSPi Link every other session's writes get `locked`; over USB a foreign write
  is caught by S5.
- A client in read-only mode, a read-only connection or a Viewer through the hub gets the passive microphone RTA and the
  documents only. The passive RTA never runs beside this core's own generator runs: `measure_rta_passive_start` is
  `busy` while a measurement, the live view or the auto-tune's runs are active, and a run that starts ends it (review
  P5-D), so it never shows this console's stimulus as programme audio.
- One measurement session per device; a second `measure_start` is `busy`.
- The measurement runs where the microphone is: in the browser's core for the web, in the Qt app; never on the hub's
  machine. The hub stores documents and arbitrates through the lock; it adds no write of its own.

---

## 7. The documents

*State: built (P5MDOC: `crates/dspi-measure/src/doc.rs`, tests `doc/tests_wp_p5mdoc.rs`). The stores and the core's calls
(7.7) are P5DOCK's; the hub's store is P5HUB's (section 8).*

### 7.1 Shape and kinds

Three new `meta.kind`s of the `.dspipreset` schema-1 family ([document-format.md](document-format.md),
[preset-format.md](../terminal/preset-format.md)). Each document is a JSON object:

```text
{"schemaVersion": 1,
 "meta": {"kind": "<kind>", "name": "...", "savedUtc": "2026-11-16T10:00:00Z", "appVersion": "..."},
 "<kind>": { ...the block... }}
```

| `meta.kind` | Block | Holds | Bound | Scope |
|---|---|---|---|---|
| `measurement` | `measurement` | a run's responses per output, the chain each was measured through, the stimulus, the capture's timing quality, the microphone used, the tunes applied from it | **1 MiB** | per device serial |
| `mic` | `mic` | a microphone's calibration on a grid, its sensitivity, where it came from | **256 KiB** | the user's (hub-wide on a hub) |
| `target` | `target` | a target curve, built-in by parameters or imported points, an optional coupler compensation, the reference band | **256 KiB** | the user's (hub-wide on a hub) |

- `schemaVersion` must be 1; another number, or none, is refused.
- The `meta.kind` word is read with ASCII case and surrounding spaces ignored. Each reader **refuses every other kind**:
  a preset (no `kind`), a `snapshot`, a `template`, `limits`, and the other two of these three.
- They configure no device: `config_import` refuses them with `InvalidArgument` and sends nothing (P5DOCK).
- A tune's result is not a new kind: it is two snapshots, "Before auto-tune" and "After auto-tune", the latter with an
  additive `tune` block (P5TUNE).
- **No impulse response and no audio** is ever in a document (S12).

### 7.2 Curves

A curve is held on a logarithmic grid given by its three numbers, `{"minHz", "maxHz", "pointsPerOctave"}` (at most 2048
points; the standard grid is `{10, 22000, 96}`, 1066 points). Its arrays are base64 strings (standard alphabet, padded)
of little-endian integers, each with exactly one value per grid point:

| Key | Values | Integer | Step | Range |
|---|---|---|---|---|
| `mag` | magnitude, dB (dB SPL when `spl` is true) | `i16` | 0.01 dB | ±327.67 dB |
| `phase` | unwrapped phase, degrees (optional) | `i32` | 0.1° | ±214 748 364.7° |
| `spread` | spread across positions or reseats, dB (optional) | `i16` | 0.01 dB | 0 to 327.67 dB |
| `valid` | the trusted mask (optional; absent means every point is trusted) | bitset | point *i* is bit *i* mod 8 (least significant first) of byte ⌊*i*/8⌋ | `ceil(points/8)` bytes |
| `spl` | the magnitudes are absolute | `true`, `false` (default) | | |

Values are rounded to the step when written; a read gives the rounded value, so a second write is byte-identical. A
standard-grid curve with phase and mask is about 9 KB. A wrong length, a string that is not base64, or a grid past 2048
points is refused; padding bits of the bitset are ignored. Example (a grid of 10 points, 100 Hz to 1 kHz at 1/3 octave):

```json
{"grid": {"minHz": 100.0, "maxHz": 1000.0, "pointsPerOctave": 3},
 "mag": "QB88IFEgah9dHiAe7B4FIGkgvB8=",
 "phase": "AAAAAJj+//8w/f//yPv//2D6///4+P//kPf//yj2///A9P//WPP//w==",
 "valid": "/gM=",
 "spl": true}
```

`mag` decodes to 80.00, 82.52, 82.73, ... dB SPL; `phase` to 0, -36, -72, ... degrees; `valid` marks every point but the
first.

### 7.3 `measurement`

| Key | Type | Meaning |
|---|---|---|
| `id` | string, optional | the store's id (1 to 64 of `0-9`, `a-z`, `-`, not starting with `-`; the snapshot store's rule) |
| `serial` | string | the measured device |
| `platform` | string | `RP2040`, `RP2350` |
| `savedMs` | integer | when the run ended, ms since the Unix epoch |
| `placement` | object | section 3: `{"mode": "listeningPosition"}`, `{"mode": "multiPosition", "count": 5}`, `{"mode": "nearField"}`, `{"mode": "headphones", "coupler": "iec60318_4", "reseats": 3}` |
| `mic` | object, optional | `{id?, name, serial?, sensitivityDb?, approximate, calHash?}`: the `mic` document used (its id and `sourceHash`) |
| `stimulus` | object | `{kind, f1Hz, f2Hz, durationMs, repeat, gapMs, levelDb, deviceRateHz}`: `kind` one of section 1's run kinds; `durationMs` 1000 to 15000 (a sweep) or 1 to 60000 (`pink`); `repeat` 1 to 8; `gapMs` at most 10000; `levelDb` the generator's peak, -120 to 0 dBFS; `deviceRateHz` 44100, 48000 or 96000 |
| `capture` | object | `{rateHz, clockRatio, timingReliable, maxResidualUs?}`: `rateHz` 8000 to 384000; `clockRatio` microphone to device, within ±1 %; `timingReliable` the gate of section 1 step 7. The core also writes, as keys other readers keep unchanged, position 1's timing detail and `rampSteps`: `[{output, levelDb, snrDb}]`, each ramp step of the first run with its in-band SNR (S3, P5RAMPFIX) |
| `outputs` | array, 1 to 16 | one per measured output, each output once (below) |
| `positions` | integer | positions or reseats averaged, 1 to 9 (default 1) |
| `note` | string, optional | the user's note, at most 4096 characters |
| `tunes` | array, optional | `{at, snapshotBefore, snapshotAfter?, kept}` per tune applied from it (at most 64): `at` in ms since the Unix epoch, the snapshots' store ids |

Each entry of `outputs`:

| Key | Type | Meaning |
|---|---|---|
| `output` | integer | the output, 0 to 15 |
| `role` | string | `full`, `sub`, `low`, `mid`, `high` |
| `speaker` | string, optional | the speaker it belongs to, as the layout names it (`L`, `R`, `Sub`) |
| `chain` | object | the measured chain: `{peq: [band], crossover: [band], gainDb, delayMs, feed: {gainDb, inverted}}`, each bank at most 32 bands, a band `{filterType, bypass, freq, q, gain, qp}` in the firmware's raw values; `feed` the programme feed (not in the generator's path) |
| `arrivalMs` | number, optional | the arrival relative to the output's own cycle start (timing-reliable runs only) |
| `response` | curve | section 7.2, with `phase` where known, `valid`, and `spread` for several positions or reseats |

A three-way stereo measurement on the standard grid, with phase, mask and spread on every output and full chains, is
about 98 KB as filed (pretty-printed) and 81 KB compact (test `wp_p5mdoc_three_way_stereo_with_phase_is_under_100_kib`).
A short example (one output, the 10-point grid of 7.2):

```json
{
  "schemaVersion": 1,
  "meta": {"kind": "measurement", "name": "Living room, left", "savedUtc": "2026-11-16T10:00:00Z", "appVersion": "0.5.0"},
  "measurement": {
    "serial": "E66164084B3C2E2B",
    "platform": "RP2350",
    "savedMs": 1763287200000,
    "placement": {"mode": "listeningPosition"},
    "mic": {"name": "UMIK-1", "sensitivityDb": -18.3, "approximate": true},
    "stimulus": {"kind": "walked", "f1Hz": 20.0, "f2Hz": 20000.0, "durationMs": 5500, "repeat": 3,
                 "gapMs": 1000, "levelDb": -24.0, "deviceRateHz": 48000},
    "capture": {"rateHz": 48000, "clockRatio": 1.00008, "timingReliable": true, "maxResidualUs": 3.2},
    "outputs": [{
      "output": 0, "role": "low", "speaker": "L",
      "chain": {"peq": [], "crossover": [{"filterType": 33, "bypass": false, "freq": 300.0, "q": 0.707, "gain": 0.0, "qp": 0.0}],
                "gainDb": 0.0, "delayMs": 0.0, "feed": {"gainDb": 0.0, "inverted": false}},
      "arrivalMs": 3.1,
      "response": {"grid": {"minHz": 100.0, "maxHz": 1000.0, "pointsPerOctave": 3},
                   "mag": "QB88IFEgah9dHiAe7B4FIGkgvB8=",
                   "phase": "AAAAAJj+//8w/f//yPv//2D6///4+P//kPf//yj2///A9P//WPP//w==",
                   "valid": "/gM=", "spl": true}
    }],
    "positions": 1
  }
}
```

### 7.4 `mic`

| Key | Type | Meaning |
|---|---|---|
| `id` | string, optional | the store's id |
| `name` | string | the microphone's name, at most 256 characters |
| `serial`, `vendor` | string, optional | as the file names them |
| `format` | string | the file it was imported from: `umik` (miniDSP UMIK-1/UMIK-2 text with a sensitivity line), `rew` (frequency, dB, optional phase), `csv`; P5DAYTON: `dayton` (a `*1000Hz` sensitivity line in dBV/Pa, -70 to 0; a UMM-6 file with a `Sens Factor` line is `umik`, vendor Dayton Audio) |
| `angle` | integer | the calibration's angle of incidence: 0 or 90 (default 0) |
| `sensitivityDb`, `gainDb` | number, optional | the file's values, in the vendor's convention (P5CAL cites it): dBFS at 94 dB SPL for `umik` and `rew`; dBV/Pa for `dayton`, whose `splOffsetDb` assumes a full-scale sine of 1 V RMS (0 dBV, `cal::ASSUMED_FULL_SCALE_DBV`) and is approximate |
| `splOffsetDb` | number, optional | the offset from dBFS to dB SPL, when known: dB SPL = dBFS + `splOffsetDb`, with the level convention below |
| `approximate` | boolean | the dB SPL scale is approximate (no calibrator step was done) |
| `response` | curve | the correction (magnitude, and phase when the file had it) |
| `sourceName`, `sourceHash` | string, optional | the imported file's name and hash (a measurement's `mic.calHash`) |
| `note` | string, optional | at most 4096 characters |

```json
{
  "schemaVersion": 1,
  "meta": {"kind": "mic", "name": "UMIK-1 700-1234 (90°)", "savedUtc": "2026-11-16T10:00:00Z", "appVersion": "0.5.0"},
  "mic": {
    "name": "UMIK-1", "serial": "700-1234", "vendor": "miniDSP", "format": "umik", "angle": 90,
    "sensitivityDb": -18.3, "gainDb": 18.0, "approximate": true,
    "response": {"grid": {"minHz": 100.0, "maxHz": 1000.0, "pointsPerOctave": 3}, "mag": "UAAyABQAAAAAAPb/AAAeAFoAoAA="},
    "sourceName": "700-1234_90deg.txt"
  }
}
```

**The level convention (one for every level the maths states).** A level in dBFS is an RMS level against digital full
scale ±1.0 in which **a full-scale sine reads 0 dBFS** (AES17, and what REW shows): `10·log10(mean square) + 3.01`. The
calibrator step, `splOffsetDb` and the microphone RTA's bands all use it, so a 94 dB calibrator reads 94 dB SPL in the
RTA's 1 kHz band after the step. The generator's `levelDb` and the clip limits of S4 are *peak* levels, a different
quantity.

### 7.5 `target`

| Key | Type | Meaning |
|---|---|---|
| `id` | string, optional | the store's id |
| `name` | string | at most 256 characters |
| `use` | string | `speakers` or `headphones` |
| `coupler` | string, optional | headphone targets only: `iec60318_4`, `flatPlate`, `hats` |
| `definition` | object | `{"type": "builtin", "builtin": "<id>", "params": {"<name>": number}}` (the ids and parameter names below; a parameter left out takes its default, an unknown id or name is refused when the target is evaluated), or `{"type": "imported", "curve": curve, "sourceName"?}` |
| `compensation` | curve, optional | a coupler compensation, subtracted from measurements before the fit |
| `referenceBand` | object | `{lowHz, highHz}`: where the target is normalised, 0 < `lowHz` < `highHz` ≤ 100 kHz |
| `note` | string, optional | at most 4096 characters |

The built-in targets (`dspi_measure::target::builtins`; all are our own parametric curves):

| `builtin` | `params` (default) | Curve |
|---|---|---|
| `flat` | none | 0 dB everywhere |
| `tilt` | `dbPerOctave` (-0.5) | a straight line through 0 dB at 1 kHz |
| `house` | `shelfGainDb` (6), `shelfCornerHz` (120), `shelfQ` (0.707), `tiltDbPerOctave` (-0.3) | a bass shelf plus a tilt |
| `coupler-iec60318-4`, `coupler-flat-plate`, `coupler-hats` | none | our default for that coupler type |

Ranges: a tilt within ±3 dB/octave, a shelf gain within ±20 dB, a corner from 10 Hz to 22 kHz, a shelf Q from 0.3 to 2.

```json
{
  "schemaVersion": 1,
  "meta": {"kind": "target", "name": "Ear simulator, our default", "savedUtc": "2026-11-16T10:00:00Z", "appVersion": "0.5.0"},
  "target": {
    "name": "Ear simulator, our default", "use": "headphones", "coupler": "iec60318_4",
    "definition": {"type": "builtin", "builtin": "house", "params": {"shelfGainDb": 4.0, "tiltDbPerOctave": -0.8}},
    "compensation": {"grid": {"minHz": 100.0, "maxHz": 1000.0, "pointsPerOctave": 3}, "mag": "AAAAAAAAMgBkAMgALAGQASwByAA="},
    "referenceBand": {"lowHz": 500.0, "highHz": 2000.0}
  }
}
```

### 7.6 Reading rules

Every document is untrusted input (from a file, the hub or another client). The readers in `dspi_measure::doc`
(`MeasurementDoc::from_text`, `MicDoc::from_text`, `TargetDoc::from_text`, `MeasureDocument::from_text` for any of the
three, `doc_kind` for a cheap sort) never panic, and:

- refuse a text longer than its kind's bound **before** parsing it (a byte-order mark is skipped);
- refuse a document of another kind or schema (`Invalid`), and text that is not such a document (`Parse`);
- refuse every value out of its bounds: grids of more than 2048 points, more than 16 outputs or an output index of 16 or
  more, an output twice, more than 9 positions or reseats (a `multiPosition` takes 3 to 9), names over 256 characters,
  notes over 4096, other texts (serials, hashes, vendors, file names) over 256, more than 32 bands in a bank, a number
  that is not finite (a JSON number too large for its field reads as infinite and is refused), and the ranges of the
  tables above;
- **keep unknown keys**: a key this build does not know, in the document, `meta`, a block, an output, a chain, a curve or
  any other object these tables list, is written back unchanged, so an edit by an older build loses nothing. The objects
  of a band, the programme feed, a grid and the placement have a fixed shape; unknown keys inside them are dropped.

Writers (`to_text`) validate first and write pretty JSON, refused past the kind's bound, so nothing a writer produces is
refused when read back.

### 7.7 The calls and the stores

*State: built (P5DOCK, P5HUB).* The core's calls take and give document text, as snapshots do: `measure_docs(kind,
serial?)`, `measure_doc_get(id)`, `measure_doc_put(text)`, `measure_doc_rename`, `measure_doc_note`,
`measure_doc_delete`; `mic_import(name, file_text)`, `target_import(name, file_text, use, coupler?)`,
`target_builtins()`, `measurement_export_rew(id or text, output)` (REW text, not a document). The local store keeps a
folder per serial for measurements and `mic/` and `target/` folders, an `index.json` cache rebuilt from the documents,
atomic writes and the snapshot store's id and corrupt-file rules. Store caps: 100 measurements a device, 32 microphones,
64 targets, 64 MiB in all. With a hub the lists merge in `measure_docs_merged`, the hub's copy winning (section 8).

## 8. On the hub

*State: built (P5HUB).* The hub stores and serves the three kinds under the cap `hub_measure`; the messages are in
[hub-api.md section 22](hub-api.md#22-measurement-documents) (a large document travels in parts of one frame each). The
core's calls are `hub_measure_docs`, `hub_measure_doc_get`, `hub_measure_doc_put`, `hub_measure_doc_rename`,
`hub_measure_doc_note`, `hub_measure_doc_delete`, and `measure_docs_merged(kind, serial)`, which lists this console's
documents and the hub's, the hub's copy winning (a differing local copy is listed beside it, never overwritten). A
client against a hub without the cap (an upstream Weeb Labs hub) keeps its documents locally and sends nothing; the
measurement itself needs only the generator opcodes, which a hub forwards by its policy (class `control`).

## 9. What the core never does

- Play test audio from the client, use `SIGGEN_FLAG_RAW`, or stage an infinite generator run.
- Write a volume, a mute or a gain while measuring, or raise any output gain in a proposal.
- Bypass a protective high-pass to measure a driver, or propose a high-pass below the S6 rule.
- Measure through crossfeed, subharmonic, psychoacoustic bass, tube, loudness or an engaging limiter.
- Offer a delay, polarity or crossover write from a run that failed the timing gate, or without a passed summed check;
  compare arrivals across separate runs.
- Write anything as a side effect of a measurement, apply a tune without a preview, a second preflight, the "before"
  snapshot and the guard, or apply a tune automatically.
- Keep, log, upload or bundle raw microphone audio, or put audio in a document.
- Open an audio device, start a thread for measuring, or use an FFT whose results depend on the machine.
- Act on a document: a measurement proposes, a person applies.

## 10. In the clients

*State: built.* The user's description of the
screens is [docs/manual/measurement.md](../manual/measurement.md); the feature table is
[PARITY.md section J](../PARITY.md#j-measurement-and-auto-tune-pro-phase-5).

| Client | Capture | Screens and commands |
|---|---|---|
| Web (`apps/web/src/screens/measure/`) | P5WCAP: an AudioWorklet with no output node, frame-indexed blocks to the Worker | P5WMS: setup (microphone, calibration import, placement, coupler, outputs, passes), preflight in words, the ramp and the run with Stop, Results through `measure_doc_curves` (smoothing, target), Live with "cut this peak" and "fit this range", the microphone RTA, the passive RTA tab for a Viewer or a read-only connection. P5WAT: the Auto-tune tab (layout, measure (the placement the user's: speakers at the listening position, or headphones on a coupler with their lower levels; three passes), summed check, proposal in words, apply with the guard's countdown, the verify started by itself after an apply, Keep's two steps, Revert, A/B, the snapshots downloadable) and the Documents tab (local and hub merged, rename, note, delete by role, mic and target import, REW export) |
| Qt (`apps/desktop/qt/qml/measure/`) | P5QCAP: `QAudioSource` off the UI thread | P5QUI: Tools > Measure… with Measure, Results (REW export), Live, Auto-tune (Verify by a button, Keep, Keep Anyway, Revert, A/B) and Documents; a Viewer is told so and the tabs that play are not offered. P5QUI2: the Passive RTA tab, Results through `measure_doc_curves`, the client read-only mode in `playBlocker` |
| Terminal (`apps/terminal/src/measure.rs`) | none (OD-P5m) | P5TM: `uconsole measure mic import`, `target import`, `docs`, `show`, `export --output N`, `tune ID [--dry-run] [--guard S]`, `run` (demo only). Timing writes need the summed check, so off the demo only an EQ-only tune is applied, unverified, said so |
| Demo device, every client | P5DEMO: the "Simulated microphone", demo connections only | the whole flow without hardware |
| Android, Swift | | later batch (PLAN-P5, "Later batch") |

Measuring and aligning **several units** with one microphone (PLAN-AT: one time base, the hub's `system.measure.*`, the
summed check across units, and the apply inside the hub's guard) is
[multi-unit-measurement.md](multi-unit-measurement.md).

## Reference

| What | Where |
|---|---|
| The plan, decisions and WPs | [`docs/core-merge/PLAN-P5.md`](../core-merge/PLAN-P5.md) |
| Shared types (grid, response, chain, placement) | `crates/dspi-measure/src/types.rs` |
| The documents | `crates/dspi-measure/src/doc.rs`, `doc/curve.rs` |
| The family's other kinds | [preset-format.md](../terminal/preset-format.md), [document-format.md](document-format.md) |
| The words clients show | [`docs/GLOSSARY.md`](../GLOSSARY.md) section 3.4 |
| The user manual | [docs/manual/measurement.md](../manual/measurement.md) |
| Several units together | [multi-unit-measurement.md](multi-unit-measurement.md); the hub's side [hub-api.md section 23](hub-api.md#23-measuring-several-units) |
