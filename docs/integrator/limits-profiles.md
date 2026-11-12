# Limits profiles (placeholder)

Planned: PRO1 S1, a profile an installer sets so that users cannot push the
system past safe ranges (maximum volume, protected outputs, locked parameters),
enforced in the core and the hub. Not built yet; nothing on this page is a
promise.

## What exists today

- The hub's roles: a `viewer` cannot change anything, a `control` client cannot
  change the device configuration ([hub.md](hub.md)).
- The core-voltage rule: no client on the network raises the core voltage above
  1.20 V.
- Read-only connections: firmware of another wire format is shown, never
  written.
- Each parameter's own range and hazard in the registry
  (`crates/dspi-proto/src/registry.rs`).

## To decide and write

- The profile's format and where it lives (the document, the hub store, the
  device).
- Which ranges it can narrow, and who may change it.
- How a client shows a limit, and what a refused write says.
