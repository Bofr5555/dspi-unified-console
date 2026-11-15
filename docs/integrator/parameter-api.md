# The parameter API

PRO1 I1: read and write parameters by name from another program, over the hub. The contract is
[hub-api.md](hub-api.md) (WP P2DOC): section 9 (parameters), 10 (presets), 11 (snapshots on the hub) and 14 (the REST
mirror and API keys), with the conventions of section 1. It is **v1, experimental** and **draft** until WP P2PA builds it.

## In short

- JSON messages on the hub's DSPi Link WebSocket (`/dspi/v1`): `param.list`, `param.get`, `param.get_many`,
  `param.set`, `preset.list`, `preset.load`, `snapshot.recall`, under the cap `param_api` (`hub_snapshots` for
  snapshots). Optional REST under `/api/v1/` with `--rest` and an API key.
- Addressed by serial; values are plain JSON, the registry's kind decides (hub-api.md 1.4).
- Every write runs through the same core code as a console's (`write_param`), as a session with the caller's role, so
  the roles, the limits profile, the installer lock and attribution are the same as in the consoles. A refusal is
  `denied`, `bad_request`, `limit_exceeded`, `installer_lock`, `image_refused` or `confirmation_required`
  (hub-api.md 1.6).
- A write by the API shows in other clients as a write from elsewhere (their undo says so) and in the hub's history
  with the caller's name in `by`.
- DSPi Link is not encrypted: read hub-api.md section 2 before putting a key in a script.

## Without a hub

- The registry of parameter paths (`vol.user`, `eq.freq`, ...), with types, ranges and hazards:
  `crates/dspi-proto/src/registry.rs`; `uconsole params` lists it.
- One write by path: `DspiCore::write_param` (C, UniFFI, wasm), the same path every typed setter and undo take.
- From a shell: `uconsole get PATH [i..]` and `uconsole set PATH [i..] VALUE`, with `--json` and exit codes; over the
  network with `--link` or `--hub`.
