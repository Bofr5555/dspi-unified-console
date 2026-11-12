# The parameter API (placeholder)

Planned: PRO1 I1, a stable, versioned API for other programs to read and write
parameters by name, over the hub. Not built yet; nothing on this page is a
promise.

## What exists today

- The registry of parameter paths (`vol.user`, `eq.freq`, ...), with types,
  ranges and hazards: `crates/dspi-proto/src/registry.rs`; `uconsole params`
  lists it.
- One write by path: `DspiCore::write_param` (C, UniFFI, wasm), the same path
  every typed setter and undo take.
- From a shell: `uconsole get PATH [i..]` and `uconsole set PATH [i..] VALUE`,
  with `--json` and exit codes; over the network with `--link` or `--hub`.
- On the network: DSPi Link's opcode-level commands, with the hub's roles
  ([hub.md](hub.md)).

## To decide and write

- Transport (HTTP and WebSocket on the hub, alongside Link), authentication
  (the hub's tokens and roles), versioning.
- Names, units and ranges from the registry; what a refusal looks like.
- How writes from the API appear in the event log and in other clients' undo.
