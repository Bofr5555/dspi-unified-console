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
| Apply | `config_import_confirmed(text, options, eq_channels, token)` (a token is required, WP P1TZ) | `uconsole import FILE [--volume] [--hardware] [--yes]` |
| Diff two documents (P1DF) | `config_diff(before, after)` | `uconsole diff A.dspipreset B.dspipreset [--json]` |
| Diff a document against the device | `config_preview(text, options, eq_channels)` | `uconsole diff FILE`, `uconsole import FILE --preview [--only ...]` |
| Apply some blocks (P1DF) | `config_import_confirmed(text, IMPORT_BY_BLOCK \| blocks, eq_channels, token)` | `uconsole import FILE --only eq,crossover,routing,outputs,limiters,effects,volumes,hardware,names [--yes]` |
| Apply with a way back (P1SA) | `apply_guarded_import(text, options, eq_channels, timeout_s)`, then `apply_confirm`/`apply_revert`, `apply_poll` | `uconsole import FILE --guard 30 [--only ...]`: answer `keep` or `revert`; no answer goes back. Over DSPi Link guarded for 30 s by default; `--no-guard` opts out |
| Snapshots (P1SN) | `snapshot_capture`, `snapshot_list`, `snapshot_recall`, `snapshot_export`, `snapshot_import`, `snapshot_delete` | `uconsole snapshot save NAME [--note T]`, `list`, `recall N [--guard S \| --no-guard]`, `diff N [M]`, `export N FILE`, `import FILE`, `delete N` |
| A/B (P1AB) | `ab_start`, `ab_toggle`, `ab_diff`, `ab_keep` | `uconsole ab [A] B` (`t`, `d`, `a`, `b`, `q` on standard input), `--keep a\|b` |
| Edit a file with no device (P1OF) | `open_document(text, file_name)`, `save_document` | `uconsole open FILE` (the interface, "File:" in its title; asks to save); `--json` describes it |

A one-shot `uconsole` command is one session of the core. With `--json` each of
these prints the core's own record or reply (`SnapshotInfo`, `AbCompare`,
`GuardedApply`, the diff's `{rows, blocks, summary, notes}`). Exit codes: 0
done (a guard kept), 1 usage, 2 no device, 3 refused (not confirmed, a lock or a
limit, a guard reverted), 4 transport. An import is a gated call
(P1TK; always, P1TZ): on a terminal it asks, in a script it needs `--yes`, which mints the
confirmation token; a guarded import needs none.

**The guard's default (WP P1EFIX).** Over DSPi Link (`--link`) a snapshot recall
and an import of a whole-device document are guarded by default, for 30 s;
`--guard S` sets the time and `--no-guard` applies plainly. Locally (USB, MIDI,
the demo) nothing is guarded unless `--guard` is given, as before.

**A guard is for a person at the terminal (O2).** The countdown reads its answer
from standard input, and a piped answer counts at once: `echo keep | uconsole
import FILE --guard 30` keeps the change straight away, so the guard gives a
script no way back. A script that needs one waits itself and answers later, or
uses `snapshot save` first and `snapshot recall` to go back.

```sh
# Prepare at the desk, apply on site with a way back
uconsole open site.dspipreset                 # edit offline, save
uconsole snapshot save "Before site.dspipreset"
uconsole diff site.dspipreset                 # what it would change here
uconsole import site.dspipreset --only eq,crossover,outputs --guard 60   # type keep, or it goes back
uconsole ab "Before site.dspipreset" 2 --keep b
```

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
