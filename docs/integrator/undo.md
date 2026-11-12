# Undo and redo

Every client has the same undo, because every change goes through one write
path in the core (WP P0W, PRO1 W1).

Reference: `crates/dspi-core/src/journal.rs` (its module documentation is the
specification), `crates/dspi-session/src/history.rs`.

## What is recorded

- Every typed setter records the change it made: the registry path, the value
  before and after, the time and the source. A change that moved nothing is not
  recorded. A slider still costs one transfer: recording adds no read.
- Calls: `undo`, `redo`, `can_undo`, `can_redo`, `history` (C: `dspi_undo` and
  so on; UniFFI and wasm the same). `NOTIFY_HISTORY` tells a client the history
  moved. `uconsole undo` and `uconsole redo` in the Terminal.
- Undo and redo replay an entry through the same setter, with its checks,
  clamps and device lock, and are recorded in turn (source `undo` or `redo`)
  without becoming a new step.

## What is stepped over

Only a parameter with no hazard is undone. These are recorded and stepped over:

- flash writes (saving a preset);
- audible ones: an output enable, the signal generator, a preset load;
- reconfigurations: pins, clocks, the input source, a bulk image;
- deferred control-surface writes, and anything irreversible.

A preset load, a bulk image (a document import) and a factory reset also end the
history before them.

## Connections and other writers

- The history belongs to the connection. A reconnect (an unplug, a DSPi Link
  re-attach, another device) starts with none. A replay that finds a different
  connection is refused before anything is sent. Nothing is ever replayed by
  itself.
- Over DSPi Link an undo is an ordinary write on the client's own session;
  other clients see it as a change from elsewhere. Each client's history holds
  its own changes only.
- When someone else has changed the value since (another Link client, a knob,
  the UART, the computer's volume), undo and redo leave their value alone, drop
  the entry and name the other writer in `HistoryMove.note`.

## To write

- The `HistoryEntry` and `HistoryMove` fields, as a table per binding.
- How a client should present a stepped-over or refused undo.
