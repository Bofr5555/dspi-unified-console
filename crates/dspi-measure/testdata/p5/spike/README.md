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
