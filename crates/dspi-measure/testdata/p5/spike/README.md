# Real microphone captures (P5SPIKE, 2026-11-18)

`k340_imm6c_cup_noise.wav`: 3.2 s, 48 kHz, mono, 32-bit float. The first 3.2 s of
the owner's raw `pw-record` capture during the first P5SPIKE ramp (`raw-01-ramp.wav`,
s24 mono, md5 c1c37d6002fd4fa704e47365e65389bc), converted sample for sample (24-bit
values are exact in f32). Microphone: Dayton Audio iMM-6C (USB), loose in a K340
headphone cup, room noise only: the ramp's outputs did not reach the cup, so nothing
of the generator is in it. Its floor is about -51 dBFS broadband, almost all
low-frequency rumble around 50 Hz; the bands from 1 kHz up lie some 30 dB lower.
The owner consented to keeping HIL recordings as test files. Used by WP P5RAMPFIX's
tests (`src/hearing/tests_wp_p5rampfix.rs`, the core's P5 session tests).

## Excerpts of two walked runs (WP P5ARRIVAL)

`only3_p5_excerpt.wav` (+ `.json`) and `ramp30_excerpt.wav` (+ `.json`): the same
microphone loose in the K340's left cup, the generator's walked 5.5 s log sweeps
(20 Hz to 20 kHz, 300 ms gaps) through the K340's 3.5 kHz crossover. Sources, the
owner's raw `pw-record` captures of P5SPIKE (s24 mono, 48 kHz):

| Excerpt | Source (md5) | Run |
|---|---|---|
| `only3_p5_excerpt` | `raw-only3-p5.wav` (3faef3d21d02cdb66c1bb3610ee45142) | output 3 (tweeter, high-pass) alone, 5 passes |
| `ramp30_excerpt` | `raw-left-ramp30.wav` (8f81c3f3f1d663ea37a1cb80144765b3) | outputs 2 (woofer, low-pass) and 3 walked, 3 passes, -30 dBFS |

Each WAV is the source's 24-bit samples unchanged, segment after segment: 300 ms
of pre-roll (noise only) ending 110 ms before the first arrival, then for every
cycle the part of its sweep that passes one band (woofer 1.1 to 4 kHz, tweeter
4.2 to 12.5 kHz, 20 ms either side). The `.json` says where each segment sits in
the source (`sourceFrame`, `frames`), the capture's first source frame
(`captureOrigin`), its length, the START frame the test uses (100 ms before the
first arrival) and the run's schedule. The test (`src/ir/tests_wp_p5arrival.rs`)
rebuilds the capture at the source's own frame positions, silence between the
segments, each cycle segment faded in and out over 20 ms (a Hann taper), and
analyses it as a walked run. The excerpts are what keeps each file under 1 MB;
the full captures and the offline analysis that these tests reproduce are in
the coordinator's `p5spike-analysis/REPORT.md` (not in the repository).

What the excerpts show: the analyser's old refinement locked one carrier
period off on the tweeter (-81.6 µs); the noise-weighted, leave-one-out
estimate gives residuals of about 14 µs (only3) and 4.4 µs (ramp30), the rest
being the capture clock's wander of 55 to 100 ppm inside the sweeps, which the
analyser now reports as such. The owner consented to keeping HIL recordings as
test files.
