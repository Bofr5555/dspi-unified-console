# The event log

A persistent, bounded log per device of what happened to it (WP P0M, PRO1 M1,
the client half). It is written from calls a client makes anyway; nothing is
polled for it.

Reference: `crates/dspi-session/src/events.rs` (the file format, in its module
documentation), `crates/dspi-core/src/events.rs` (what the core records).

## Where it is

In memory (the last 1000 records) until the client names its data folder with
`event_log_set_dir(dir)` (C: `dspi_event_log_set_dir`). Then one file per
device:

- `events-s-<serial>.jsonl` when the serial is ASCII letters, digits, `-` and
  `_` only; otherwise `events-x-<the serial's UTF-8 bytes in hex>.jsonl`;
- rotated past the size limit: `.1.jsonl` is the newest older file; about 1 MiB
  per device in all with the defaults;
- on Unix the folder is created 0700 and the files 0600.

## A record

One JSON object per line, oldest first. Absent fields are omitted.

| Field | Type | Meaning |
|---|---|---|
| `v` | number | record version (1) |
| `id` | number | rises per device, across files and runs |
| `t` | number | ms since the Unix epoch |
| `kind` | string | see below; an unknown kind from a newer build reads as `unknown` |
| `serial` | string | the device |
| `text` | string | one readable line |
| `source` | string | who or what caused it: a notification's source (`HOST`, `PRESET`, `UART`, ...), a transport, a counter, a reset reason |
| `session` | number | the DSPi Link session that caused it, when another one did |
| `own` | bool | this client's own write coming back |
| `channel` | number | a wire channel (clips), or the preset slot (recalls) |
| `value` | number | °C for a temperature, the total for a counter |
| `fw` | string | the firmware version, on connects and firmware changes |
| `uptime_s` | number | the device's uptime, where it reports one |
| `lines` | array of strings | what a recall or an import changed |
| `count` | number | occurrences this record stands for (default 1) |

Kinds: `notification`, `connected`, `disconnected`, `clip`, `temperature`,
`counter`, `reset`, `firmware`, `import`, `recall`, `unknown`.

## Reading it

- `event_log_read(serial, after_id, limit)`: page with the last id seen; an
  empty serial is the open device. `event_log_export(serial)`: the whole log in
  the file format. `event_log_devices()`: the serials with a log.
- A line that is not a record (a torn write, a hand edit) is skipped and
  counted; nothing in a file makes a read fail.
- Repeats are coalesced: a slider dragged through fifty notifications is one
  record with a `count`.

## Writing it from another program

Append only under an exclusive advisory lock on `<stem>.lock` beside the files
(`flock`, `LockFileEx`), taking the id under that lock as one past the newest in
the files. Two writers of one serial in one folder (a hub and a Terminal on one
machine) then never write the same id. Readers take no lock.

## To write

- The hub's shared log (M1's hub half, `EventSink`), once it exists.
- Examples of each kind.
