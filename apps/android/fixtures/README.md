# Fixtures (owned by package FFI)

## Device capture (V32, RP2350, firmware 1.1.6-beta5)

Copied unchanged from WeebLabs/DSPi-Link, commit
`05949b845b9c2e7df3c9541fde7ea39fdd5d7522` (`05949b8`), directory
`fixtures/device/` (local clone: `../_ref/DSPi-Link`). DSPi-Link captured
them from a unit on 2026-10-17 (see `capture.json`).

| File | Size | SHA-256 | What |
|---|---|---|---|
| `bulk_v32.bin` | 6136 B | `23bea903d678108f6c29febab040bc5ebc84ac91e5801a4c73d83dec998d79d2` | REQ_GET_ALL_PARAMS reply, wire format V32 |
| `platform.bin` | 7 B | `64090dda861b20722d2ecb5deccc7e37ff76b57174a7d19da72ea70c5fac7c36` | REQ_GET_PLATFORM reply (`01 01 16 09 01 06 05`: RP2350, 9 outputs, 1.1.6 beta 5) |
| `capture.json` | 939 B | `af307f2be49a5c0a906a463182683fcd9015824d53831dc41b912e30215dab01` | DSPi-Link's description of the capture |

## Derived (generated, checked in)

| File | Produced by | Contents |
|---|---|---|
| `dspstate_v32.json` | `crates/dspi-core/tests/wp_an1_android_fixture.rs` | `platform.bin` + `bulk_v32.bin` decoded by the core (`state::apply_platform`, then `state::decode_bulk`; `compat` = 1 OK), serialised as the `DspState` record |
| `dspstate_default.json` | same | the core's state before any device (`DspiCore::new().get_state()`; includes the `UPMIX/SUBHARM/TUBE_DEFAULTS` tables) |

Field names are the Kotlin names of `se.bofr.dspi.core.DspState` (camelCase;
a test checks them against the generated `CoreTypes.kt`). Values are the raw
core values: integers for `u8`/`u16` codes, `bulkRaw` as 6136 integers 0..255
(Kotlin: `ByteArray`), nested lists for `filters`/`xover` `[wire channel][band]`
and the matrix `[input][output]`.

Not in the capture, so left at their defaults in `dspstate_v32.json`: what
`fetch_all` reads item by item after the bulk image (Core 1 mode, subharm
solo, DAC mute, UART/I2C configs and status, preset directory and names).

That test fails when the core's decoder and these files part ways (they were
written by the old core, and the new decoder matches them value for value).
Regenerate only on purpose:

```
ci/run.sh 'DSPI_WRITE_ANDROID_FIXTURE=1 cargo test --locked -p dspi-core --test wp_an1_android_fixture'
```

## Firmware opcodes

`firmware_req_opcodes.txt`: the `#define REQ_*` lines of the firmware's
`firmware/DSPi/config.h` at the commits pinned in `tools/REFERENCE.toml`
`[firmware]`, extracted read-only with `git show`. `tools/opcode_coverage.py`
checks against it (CI has no access to the firmware repository); refresh with
`python3 tools/opcode_coverage.py --refresh-firmware ~/prog/DSPi` on the host.

## Firmware cases (`wire/`, WP AN2)

What the core reports for each firmware the app has to tell apart, one small
`DspState` per file: only the fields the banner and read-only tests read, and
`bulkRaw` cut to the image's first byte (it says whether an image was decoded).
They are not on any classpath and not in the APK. Written and checked by
`crates/dspi-core/tests/wp_an2_read_only_firmware.rs`, which regenerates them
with `DSPI_WRITE_ANDROID_FIXTURE=1`.

The older images behind them are the V32 capture converted by inverting
`wire_compat`'s mapping, not captures from units on that firmware. `stock_final_v32`
and `k340_v32` are the capture with the final's version reply (1.1.6, beta 0);
the K340 case also answers the extension channel and has a crossfeed pair map.
