# The Terminal on the K340: hardware record (TM13)

TM13's record (`PLAN-CLIENTS.md`, **TM13**). Done when: USB replug and real MIDI ports end to end pass.
Facts are from the run reports named in each section; nothing here was re-run for this record.

## Runs

| run | date | commit | where | report |
|---|---|---|---|---|
| hw-1 | 2026-11-07/08 | `main` @ 2f1e95f3 | Linux host, podman containers | `~/.cache/dspi-build/work/hw-1/REPORT.md` |
| hw-win (the `dspi.exe` part) | 2026-11-08 | `main` @ 2f1e95f3 | `winvm` (`dspi-win11`, Windows 11 Pro 26300) | `~/.cache/dspi-build/work/hw-win/REPORT.md` |

Device in both: PCB unit BB66A44F2AC1E7B6, RP2350, firmware `v1.1.6-20-g04122d3` (bofr-main @ 04122d3, 2026-11-06),
wire V32, 17 channels, core 1.20 V, USB 2e8b:feaa. Both runs were scripted by an agent; no owner hands.

## Setup and safety

- **hw-1:** toolchains in podman (`localhost/dspi-coremerge:4`). Device runs in a container with only
  `/dev/bus/usb/001/110` (plus `/dev/snd/seq` for MIDI), real sysfs, `--network=none`, no keep-id. The script refused to run
  if a bootloader device (2e8a) was present.
- **hw-win:** `dspi.exe` cross-built in containers (MSVC with `cargo xwin`, and GNU), copied to the VM. The DSPi passed with a
  stock `usb-host` hostdev (`virsh attach-device --live`), detached at the end.
- Both: nothing flashed, bootloader never entered, VREG read only, no OVERVOLT, no preset save, load or delete, signal
  generator never started. Only audio: digital silence through `flash-tools/guarded-silence.sh`, sink at 40 %.
- Backup before any write (`backup-inner.sh`, read-only GETs: whole-device export, filter file, `cs export`, raw 0xA0 image,
  raw names, preset directory, `ext vreg`, `ext xfeed-map`). `compare.sh` ignores timestamps and `uptime_s` only.

## Checks: Linux (hw-1, `tm-pass.sh`, `tm-pass.log`)

| check | result |
|---|---|
| doctor, list, `--version`, dump, `dump --json`, `ext list`, `ext vreg` (read), `ext xfeed-map` (read) | pass |
| interface screenshot on the real device; `export - --json` parses | pass |
| read sweep: `get` of every non-action parameter (index 0) | pass: 158 read. 10 refused: 6 need two indices (the sweep's fault), `in.spdif.enable 0` invalid on this board, `cs.aux 0` and `cs.aux.level 0` stall (no aux in slot 0), `rta.control` (D2) |
| `set ch.delay i2s.1.l` 0 → 1 → 0, each confirmed by `get` | pass |
| undo and redo in one script (`-f`) | pass. As separate processes `dspi undo` says "Nothing to undo", by design (the journal lives with the connection) |
| TM20 latency confirm: `out.latency` 4 → 3 → 4 ms | pass |
| script `-f`; out-of-range value rejected (exit 1), value unchanged | pass |
| `import --dry-run` of the live export writes nothing | pass |
| **USB MIDI:** `--midi list` (port "Weeb Labs DSPi DSPi Control" 16:0), `--midi dump`, `--midi get`, `--midi set` delay 1 → 0 | **pass** |
| **notifications:** `watch --json` over USB while writing over MIDI | **pass**: both seen, "changed over MIDI" |
| `--dry-run set …` with the flag before the command | **fail: D1** |
| USB replug | **not run**: needs the owner's hands |

## Checks: Windows (hw-win, `tm-win.ps1`, `tm-win-msvc.log`, `tm-win-gnu.log`)

| check | MSVC | GNU |
|---|---|---|
| doctor (interface claimed, firmware 1.1.6, wire V32, MIDI port "DSPi Control"), list, `--version` | pass | pass |
| dump, `dump --json`, `ext list`, `ext vreg` (read), `ext xfeed-map` (read), screenshot 120×40 | pass | pass |
| `export - --json` parses; export to file; `import --dry-run` of it | pass | pass |
| read sweep | pass, the same 158 and 10 refusals as Linux | same |
| `set ch.delay i2s.1.l` 0 → 1 → 0 confirmed | pass | pass |
| undo and redo in one script | pass | pass |
| out-of-range rejected, value unchanged | pass | pass |
| **USB MIDI (WinMM):** `--midi list`, `dump`, `get`, `set` 1 → 0 | **pass** | **pass** |
| notifications: `watch --json` over USB while writing over MIDI | pass | pass |
| **reconnect after a USB detach and re-attach** (`virsh detach-device` / `attach-device`, not a physical unplug) | **pass**: `watch` printed `dspi: device disconnected` 2.0 s after the detach and exited 4; `get` while detached: "no DSPi device found", exit 2; after re-attach `get` 0.0 ms and `doctor` all ok (`replug-cli.log`, `replug-watch2.log`) | not run |

The WinUSB 4 KB control-transfer cap did not bite (export, dump and the bulk read work).

## Checks: `dspi hub` with Terminal clients

| check | result |
|---|---|
| hw-1: `dspi hub run --loopback --open`; `--link` list, dump, export, `ext vreg` (read), get, `set ch.delay` 0 → 1 → 0 | pass (after the hub reported the device attached; M2) |
| hw-1: `--link watch` in a second client sees the first client's writes | pass, "changed by another Link client" |
| hw-1: `real_hub` harness safe set (16 tests), open hub | ok; scenarios 1_idle, 3_dark 5/20/32/60 s, 3_dark_60s_idle, 7_attach_gets, smoke passed with the 0xA0 image compared before and after |
| hw-1: PIN hub (`--pair`): setup_pair, smoke, 7_attach_gets, 8_pair_cut_at_ok, 8_pair_late_reply, 2_wrong_pin_keeps_token, 4_slots_full | pass, settings compared |
| hw-1: teardown_revoke | test ok, but the hub refused revoke to the viewer session, so the harness skipped it |
| hw-1: no PIN or token in the hub's stderr | pass (0 lines) |
| hw-win: `dspi.exe hub run --open --loopback` in the VM, Linux `dspi` through an ssh tunnel: list, dump, export, get, `ext vreg`, `set` 0 → 1 → 0, notifications to a second client, `real_hub` safe set | 11/11 pass. Not reachable directly: the VM forwards only ssh |
| hw-win: TM11 scheduled task import | **fail: F2** (the documented command); with the declaration changed in memory the task imports and starts. Logon trigger not exercised |
| not run | `6_board_flap` (someone cuts board power), `6_no_board_pairing` (board absent), the PIN hub on Windows |

## Device restored

- **hw-1:** final export against the first backup: **empty diff** (`backup/final-diff.txt`, rc 0). Mid-run copy also
  identical. No restore needed. Uptime 12494 s → 13925 s without a break, `hang_count 0`.
- **hw-win:** first backup byte-identical to hw-1's final export. One difference after the VM: `i2sInputRateHz` 48000 → 96000,
  set by Windows' USB audio driver; `dspi set in.rate 48000` was refused ("device kept 96000 Hz"). **Restored** by 6 s of
  guarded 48 kHz digital silence on the host; then **empty diff** (`backup/final-diff.txt`, rc 0). Uptime 14422 s → 16318 s,
  `hang_count 0`.

## Defects and observations, and where they went

| id | what | fixed |
|---|---|---|
| D1 | a boolean global flag before the command (`--json`, `--quiet`, `--dry-run`) read as "no command", opened the full-screen interface and panicked without a tty (exit 101); the write did not happen | TM26 (2df60015) |
| D2 | `get rta.control` says "read-only" for a write-only action | TM26 |
| M1 | `cs.aux 0` / `cs.aux.level 0` report "stalled" on a slot with no aux | TM26 |
| M2 | the hub accepts connections ~0.3 s before it has attached the device | TM26 |
| M3 | `RUST_LOG=info` still prints `[DEBUG tungstenite::protocol]` lines (72 on Linux, 70 on Windows); no PIN or token in them | TM26 |
| M4 | `dspi hub` reads `pair`/`quit` only from a terminal; scripts restart with `--pair` | by design |
| M5 | `systemd-inhibit` refused in the container | expected there |
| F2 (hw-win) | `dspi-hub-task.xml` declared `encoding="UTF-8"`; `Register-ScheduledTask -Xml` fails "unable to switch the encoding" | CG31 (0f163dfa) |
| — | first start of `dspi.exe` in a new path took ~10–30 s to listen (hw-win O1) | cause not found; may be the VM |
| — | Windows Firewall prompts on the hub's first start; Cancel creates two Block rules | not filed; the report suggests a README line |

The fixes were made after the runs; they have not been re-run on the K340.

## Deviations

- hw-1 used its own `ci/run.sh` clone pinned to its own target dir (the shared `target/HEAD` was in use), and reproduced
  `run-hw.sh`'s command rather than running it.
- hw-win reached the Windows hub through an ssh tunnel; it did not change the VM's network. It removed
  `%LOCALAPPDATA%\dspi` whole after the TM11 check (a test VM).
- The replug was a VM detach and re-attach on Windows only. Whether that counts as TM13's "USB replug" is the coordinator's
  call (unclear from the plan).

## Still owed

- **USB replug on Linux** with a physical unplug (`apps/desktop/qt/smoke/run-replug.sh`, the owner's hands), or the
  coordinator accepting hw-win's VM detach as the replug.
- A re-run of D1, D2, M1–M3 and F2 after their fixes (TM26, CG31) on the K340: not required by TM13, not done.
- `real_hub` `6_board_flap` and `6_no_board_pairing` (need a person or the board absent).
