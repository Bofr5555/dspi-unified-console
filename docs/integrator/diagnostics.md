# The diagnostics bundle

One zip a user saves and attaches to a support request themselves (WP P0E, PRO1
E2). Nothing is sent anywhere: the file is written where the user says.

Reference: `crates/dspi-core/src/diag.rs`.

## Making one

| From | Call |
|---|---|
| Terminal | `uconsole diag [FILE] [--store DIR] [--events DIR]` (any target: `--demo`, `--device`, `--midi`, `--link`, `--hub`) |
| C | `dspi_diagnostics_save(core, path, options_json, buf, len)`; `dspi_diagnostics_file_name` for a suggested name |
| Kotlin, Swift | `diagnosticsSave(path, DiagOptions)`; `diagnosticsFileName()` |
| Rust | `DspiCore::diagnostics_save`, or `diagnostics_zip` for the bytes |

`DiagOptions`: `app` (the client and its version), `hub_store_dir` (a hub store
to include; empty for none), `files` (the client's own files: a log, a report).
It reads the device and files, so call it off the UI thread.

## What is in it

| File | What |
|---|---|
| `README.txt` | what this is, what is in it, what was left out |
| `versions.json` | the core, the client, the OS, the device's firmware |
| `device/dump.json` | the open device: serial, transport, the probe's capabilities, extensions, the settings mirror |
| `device/stats.json` | status counters, buffer levels, ring health, clock diagnostics |
| `device/interrupts.txt` | the notification monitor's lines |
| `events/<serial>.jsonl` | each device's [event log](event-log.md) |
| `hub/hub-store.json` | the hub store, secrets removed |
| `system/os.json`, `system/usb.json` | OS facts; DSPis and bootloaders on the bus, by enumeration only |
| `client/<name>` | the client's own files (the Terminal adds `doctor.txt`) |
| `notes.txt` | what could not be read, and why |

A section that cannot be read (no device, a viewer refused a read) leaves a line
in `notes.txt`; the rest is still written. Nothing is written to the device and
nothing on the bus is opened.

## What is left out

Every value in the hub store whose key names a token, a hash, a secret, a PIN, a
password or a key reads `(removed)`; a store that is not JSON is left out
entirely. Device serials, device and client names and file paths stay, because a
supporter needs them.
