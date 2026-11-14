# Integrator guide

For people who build DSPi processors into an installation, a product or another
program: running the hub on a machine that stays on, reading and writing the
document format, following the event log, and controlling the device from a
microcontroller. The user-facing manual is separate (PRO1 E6, not written yet).

This is a skeleton (WP P0E, PRO1 E7). Each page says what exists today, with the
source that is the reference, and marks what is planned. Where a page and the
code disagree, the code is right and the page is the bug.

Licensing of the software described here: see [`NOTICE.md`](../../NOTICE.md).

## Contents

| Page | Covers | State |
|---|---|---|
| [hub.md](hub.md) | The DSPi Link hub: install, service units, PIN pairing, roles, network and firewall, its store | exists; page is an outline |
| [document-format.md](document-format.md) | `.dspipreset` (the whole-device document) and filter files: shape, import rules, the calls | exists; page is an outline |
| [undo.md](undo.md) | What undo and redo do, what they step over, writes from elsewhere | exists |
| [event-log.md](event-log.md) | The per-device event log: file names, record fields, kinds, rotation, several writers | exists |
| [diagnostics.md](diagnostics.md) | The diagnostics bundle: what is in it, what is left out | exists |
| [firmware-control.md](firmware-control.md) | Controlling the device over its UART and I2C, without a computer | exists in the firmware; page is an outline |
| [audio-link.md](audio-link.md) | The audio link's system model on the hub: topology, alignment, scenes at a frame, alarms, `system.*` messages | exists (host side; firmware partial) |
| [usb-direct-home-theater.md](usb-direct-home-theater.md) | A home theatre fed over USB from one computer: cables, multi-TT hubs, isolators, power, capacity, PipeWire (`uconsole usb-direct generate`), D, lock and lock error (WP AUSB3) | guide; bench not run yet |
| [parameter-api.md](parameter-api.md) | A stable parameter API for other programs (PRO1 I1) | placeholder |
| [control-protocols.md](control-protocols.md) | OSC, MIDI and MQTT control (PRO1 I-series) | placeholder |
| [limits-profiles.md](limits-profiles.md) | Limits profiles: ranges an installation locks down (PRO1 S1) | built (P1LM) |
| [installer-lock.md](installer-lock.md) | The installer's PIN, the locked-user state and the mode manifest (PRO1 S4, U7) | built (P1LK) |

## Ways in, today

| From | Use | Reference |
|---|---|---|
| A program in C or C++ | the C ABI, `ci/generated/dspi_core.h` (library `dspi_capi`) | `crates/dspi-capi` |
| Kotlin or Swift | the UniFFI library `dspi_ffi`, namespace `dspi_core` | `crates/dspi-ffi`, `ci/generated/kotlin`, `ci/generated/swift` |
| A browser | the web console's Worker over the wasm binding | `crates/dspi-wasm`, `apps/web` |
| A shell script | `uconsole` one-shot commands, `--json`, exit codes | `uconsole --help`, `docs/terminal/` |
| Another machine on the network | the DSPi Link protocol, through a hub | [hub.md](hub.md); the spec is DSPi-Link's `spec/dspi_link_protocol_spec.md` |
| A microcontroller | the firmware's UART or I2C control | [firmware-control.md](firmware-control.md) |

Every binding is the same core (`crates/dspi-core`), so a rule stated here holds
for all of them. Calls that touch the device block their thread: never call them
on a UI thread.

## Confirmation tokens

<!-- -- WP P1TZ -- -->
Dangerous operations always need a confirmation token (OD-P1f; a behaviour change of
the C ABI and the bindings, see `CHANGELOG.md`). After the person has confirmed one,
mint a token for it and pass it to the operation's `_confirmed` call:

| Operation (key) | Call that takes the token |
|---|---|
| `save_preset`, `copy_preset`, `delete_preset` | `save_preset_confirmed`, `copy_preset_confirmed`, `delete_preset_confirmed` |
| `save_params`, `factory_reset`, `save_output_config` | `save_params_confirmed`, `factory_reset_confirmed`, `save_output_config_confirmed` |
| `enter_bootloader`, `firmware_flash` | `enter_bootloader_confirmed`, `flash_confirmed` (mint it in the bootloader wait) |
| `config_import` | `config_import_confirmed` |
| `vreg_save` | `core_vreg_cal_commit_confirmed`, `core_vreg_manual_set_confirmed` |

C: `dspi_confirm_token(core, "<key>", buf, len)` and the `dspi_*_confirmed` functions
(`ci/generated/dspi_core.h`); Kotlin and Swift: `confirmToken(op)` and the
`*Confirmed` methods; the web Worker: `confirmToken {op}` and a `token` field on
`write`, `presetCopy`, `factoryReset` and `configImport`. A token is for one operation,
used once, valid for 10 s and only on the connection it was minted on. Without a good
token the call is refused with `confirmation_required` (the notice row of that name)
and nothing is sent. The `_confirmed` calls are the only forms: the calls without a
token (`save_preset`, `config_import`, `config_import_selected`, ...; C `dspi_save_preset`,
...) are removed (WP P1TZ). `write_param` keeps the gated registry paths (`preset.save`,
`preset.delete`, `dev.save`, `dev.save.io`) and refuses them with
`confirmation_required`; `write_param_confirmed` (and the Worker's `write` with
`token`) takes them. There is no switch to turn tokens off. A token is a discipline,
not a security boundary: over DSPi Link the hub's roles stay the boundary.
Snapshot recall, A/B and a guarded apply take no token.

## To write

- A worked example per way in (C, Kotlin, Swift, shell, Link).
- The exit codes and `--json` shapes of `uconsole`, as a table.
- Versioning: what a client of the C ABI or of Link may rely on between releases
  (the ABI anchor, `crates/dspi-capi/tests/data/abi-anchor.txt`; the Link spec's
  version negotiation).
