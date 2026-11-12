# The document format

Two kinds of file carry a tuning between devices and programs. Both are read and
written by the core, so every client agrees on them.

Reference: `crates/dspi-core/src/files.rs`, `crates/dspi-session/src/preset_file.rs`,
and the full key list in [`docs/terminal/preset-format.md`](../terminal/preset-format.md).

## `.dspipreset`: the whole device

- JSON, `schemaVersion` 1; a larger number is refused. Top-level keys:
  `schemaVersion`, `meta`, `global`, `loudness`, `crossfeed`, `leveller`,
  `psybass`, `upmix`, `subharm`, `tube`, `channels`, `matrix`, `io`.
- Values are the firmware's raw wire numbers (filter types, masks, modes, pins),
  not names.
- An export writes only what the device's wire format carries
  (`dspi_proto::wire_compat::features`): an older device's document has no key
  its firmware lacks. A feature block that is absent means the source device
  lacked it, which is different from present and off.
- An import changes only what the file holds a readable value for; everything
  else keeps the device's value, and the import report names what was skipped.
  The master and user volume and the hardware block (pins, interfaces) are
  applied only when asked (`IMPORT_VOLUMES`, `IMPORT_HARDWARE`).
- An import is one bulk write and is live, not stored: save it to a preset slot
  to keep it.
- Files over 4 MB are refused before they are parsed.

| Do | Core call | `uconsole` |
|---|---|---|
| Export | `config_export(name, app_version, links)` | `uconsole export FILE.dspipreset` |
| Describe before applying | `config_inspect(text)` | `uconsole import FILE --dry-run` |
| Apply | `config_import(text, options)` | `uconsole import FILE [--volume] [--hardware]` |

## Filter files: EQ and crossovers

Text. This build writes the macOS console's format 2 (`# Format: 2`) and reads
that, the Windows console's name-keyed sections, and REW or AutoEQ filter lists.
Core calls: `filters_export`, `filters_inspect`, `filters_import`; `uconsole
export FILE` with any other extension.

## To write

- An annotated example document, generated from the demo device.
- The compatibility promise: which keys a later schema may add, and how a
  reader treats a key it does not know.
- The shared fixture set every client tests against.
