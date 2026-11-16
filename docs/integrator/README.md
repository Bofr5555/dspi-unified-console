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
| [hub.md](hub.md) | The DSPi Link hub: install, service units, PIN pairing, roles, encryption (`--tls`, upstream compatibility, the key), the security deviations, the firmware rollout's switches (`--rollout`, `--allow-unverified-firmware`), network and firewall, its store | exists; page is an outline |
| [security.md](security.md) | Security: the threat model, TLS and key pinning on DSPi Link, pairing, the red "(unsecured)" bar, upstream compatibility mode, the browser, integrations (curl, Python, Home Assistant), what stays plaintext and why, downloads and firmware, the hub's firmware rollout (WP P3FRZ), secrets at rest (WP SECDOC) | built |
| [document-format.md](document-format.md) | `.dspipreset` (the whole-device document) and filter files: shape, import rules, the calls | exists; page is an outline |
| [undo.md](undo.md) | What undo and redo do, what they step over, writes from elsewhere | exists |
| [event-log.md](event-log.md) | The per-device event log: file names, record fields, kinds, rotation, several writers | exists |
| [diagnostics.md](diagnostics.md) | The diagnostics bundle: what is in it, what is left out | exists |
| [firmware-control.md](firmware-control.md) | Controlling the device over its UART and I2C, without a computer | exists in the firmware; page is an outline |
| [audio-link.md](audio-link.md) | The audio link's system model on the hub: topology, alignment, scenes at a frame, alarms, `system.*` messages | exists (host side; firmware partial) |
| [usb-direct-home-theater.md](usb-direct-home-theater.md) | A home theatre fed over USB from one computer: cables, multi-TT hubs, isolators, power, capacity, PipeWire (`uconsole usb-direct generate`), D, lock and lock error (WP AUSB3) | guide; bench not run yet |
| [hub-api.md](hub-api.md) | The hub API of PRO phases 2 and 3: encryption and pairing (`auth.rotate`, `key_next`, `tls_required`), inventory, Identify, health (with the health board's counters, clip rate and last change), the shared history, alarms, notifications out, parameters, presets and snapshots by name, schedules, MQTT and Home Assistant, the REST mirror and API keys, statistics; groups, group writes and bulk apply, and the firmware rollout (TLS only); the deviations from the DSPi Link spec (WP P2DOC; phase 3: WP P3DOC) | **v1 frozen 2026-11-16** (WP P3FRZ): additive only, a breaking change is `/api/v2/` and new message types; every section built |
| [parameter-api.md](parameter-api.md) | The parameter API for other programs (PRO1 I1): a summary of hub-api.md's parameter sections | draft (P2PA) |
| [control-protocols.md](control-protocols.md) | OSC, MIDI and MQTT control (PRO1 I-series) | placeholder |
| [limits-profiles.md](limits-profiles.md) | Limits profiles: ranges an installation locks down (PRO1 S1) | built (P1LM) |
| [installer-lock.md](installer-lock.md) | The installer's PIN, the locked-user state and the mode manifest (PRO1 S4, U7) | built (P1LK) |
| [read-only-mode.md](read-only-mode.md) | The client read-only mode: a diagnostic that refuses every write in the core, not a security boundary (WP ROMODE) | built |

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
| `enter_bootloader`, `firmware_flash` | `enter_bootloader_confirmed`; `flash_confirmed`, the one drive write, behind the firmware guard: a core method that needs no device open, so a board already in BOOTSEL is flashed too (pass `firmware_guard_board_in_bootloader()` and the phrase the verdict asks for; mint the token once the drive is there) |
| `config_import` | `config_import_confirmed` |
| `vreg_save` | `core_vreg_cal_commit_confirmed`, `core_vreg_manual_set_confirmed`, `core_vreg_op_confirmed` (a SET; trials, AUTO and CLEAR take no token) |
| the gated registry paths | `write_param_confirmed` (C `dspi_write_param_confirmed`; the Worker's `write` with `token`) |

C: `dspi_confirm_token(core, "<key>", buf, len)` and the `dspi_*_confirmed` functions
(`ci/generated/dspi_core.h`); Kotlin and Swift: `confirmToken(op)` and the
`*Confirmed` methods; the web Worker: `confirmToken {op}` and a `token` field on
`write`, `presetCopy`, `factoryReset` and `configImport`. A token is for one operation,
used once, valid for 10 s and only on the connection it was minted on. Without a good
token the call is refused with `confirmation_required` (the notice row of that name)
and nothing is sent. The `_confirmed` calls are the only forms: the calls without a
token (`save_preset`, `config_import`, `config_import_selected`, ...; C `dspi_save_preset`,
...), `core_vreg_op` and the free firmware writes (`flash_to_drive`,
`flash_to_drive_guarded`; C `dspi_flash_to_drive`, `dspi_flash_to_drive_guarded`) are
removed (WP P1TZ, P1TZF); there is no drive write without the firmware guard.
`write_param` keeps the gated registry paths (`preset.save`, `preset.delete`,
`dev.save`, `dev.save.io`) and refuses them with `confirmation_required`;
`write_param_confirmed` (C `dspi_write_param_confirmed`, UniFFI, and the Worker's
`write` with `token`) takes them. Raw requests (`raw_get`, `raw_set`) refuse the
opcodes of these operations (`invalid`, naming the `_confirmed` call), since they
carry no token; every other opcode goes out raw as before. There is no switch to turn tokens off. A token is a discipline,
not a security boundary: over DSPi Link the hub's roles stay the boundary.
Snapshot recall, A/B and a guarded apply take no token.

## To write

- A worked example per way in (C, Kotlin, Swift, shell, Link).
- The exit codes and `--json` shapes of `uconsole`, as a table.
- Versioning: what a client of the C ABI or of Link may rely on between releases
  (the ABI anchor, `crates/dspi-capi/tests/data/abi-anchor.txt`; the Link spec's
  version negotiation). The hub API's own rule is written: v1 is frozen and
  additive only ([hub-api.md](hub-api.md) section 1.2).
