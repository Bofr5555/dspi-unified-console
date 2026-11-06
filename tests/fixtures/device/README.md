# Expected mirror states

The device capture itself is `../link/device/` (`platform.bin`,
`bulk_v32.bin`: RP2350, firmware 1.1.6 beta 5, wire V32; from
WeebLabs/DSPi-Link `05949b8`).

| File | What |
|---|---|
| `dspstate_v32.json` | The old desktop core's decode of that capture (`parse_platform` then `decode_bulk`, `compat` = 1), as the Android app's `DspState` record with camelCase names. WP C0's `state::decode_bulk` must reproduce it. |
| `dspstate_default.json` | The old core's `DspState::default()` (no device), including the upmixer, subharm and tube default tables. WP C0's `state::default_state()` must reproduce it. |

Copied unchanged from Bofr5555's Android console (`dspi-android` `main`,
`fixtures/`), where `rust/dspi-ffi/examples/dump_state.rs` generated them.
Values: integers for `u8`/`u16` codes, `bulkRaw` as 6136 integers, nested
lists `[wire channel][band]` and `[input][output]`.
