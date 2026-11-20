# Licences

The licence of the program is the GNU GPL version 3: [`LICENSE`](../LICENSE) holds its text, and
[`NOTICE.md`](../NOTICE.md) is the statement (copyright, the additional terms, the source code offer, who made what,
and the third-party notices). This page summarises them. The iOS licence inventory is in [`licensing/`](licensing/)
([`IOS-INVENTORY.md`](licensing/IOS-INVENTORY.md)). Where the work comes from: [`CREDITS.md`](CREDITS.md).

> **DRAFT.** Not legal text: the statement is [`NOTICE.md`](../NOTICE.md), pending the
> owner's approval. This section only summarises it.

The program as a whole is distributed under the **GNU GPL version 3**: it combines
Weeb Labs' parts under GPL-3.0 (the DSPi firmware, DSPi Console for macOS, DSPi
Console for Windows) or GPL-3.0-or-later (DSPi-Terminal), DSPi-Link under MIT, and
Bofr5555's own additions, which are licensed **GPL-3.0-or-later with three
additional terms** under GPL version 3 section 7 (attributions kept, modified
versions marked, no trademark rights). `LICENSE` holds the GPL version 3 text; the
workspace's licence field, `GPL-3.0-or-later`, is the licence of Bofr5555's own
additions. Every client shows the notice and the terms in its About, and every
release is published with the source archive of the commit it was built from
(`ci/release-source.sh`).

| Component | What applies |
|---|---|
| Bofr5555's additions: the core merge (`crates/dspi-core`, `dspi-capi`, `dspi-ffi`, `dspi-hub`, `dspi-measure`, `dspi-wasm`), the Android console, the Rust Link client (`crates/dspi-link`), the web console | GPL-3.0-or-later with the section 7 terms |
| DSPi-Terminal parts and what grew from them (`apps/terminal/`, `crates/dspi-proto`, `dspi-transport`, `dspi-session`, `dspi-cmd`, `dspi-tui`, `xtask/`) | GPL-3.0-or-later, as upstream's manifests and README state |
| `apps/apple/`, and the UI, behaviour and texts other clients follow from it | GPL-3.0 (DSPi Console for macOS; its `LICENSE` says version 3) |
| The Android files ported from DSPi Console for Windows (listed in `NOTICE.md`) | GPL-3.0 (its README says GPL v3) |
| Vendored firmware headers (`crates/dspi-proto/firmware/`) and bundled firmware images | GPL-3.0 (DSPi firmware v1.1.6), with the third-party parts in `assets/licences/firmware-THIRD-PARTY-NOTICES.txt` |
| `tests/fixtures/link/`, the hub's DSPi-Link web page | MIT, Weeb Labs |
| AutoEQ database (`crates/dspi-session/autoeq/`) | AutoEq data, MIT (`assets/licences/AutoEq-MIT.txt`); the measurement sources' own terms are an open point |
| `apps/desktop/` (the Qt app) | Rebuilt on the core (2026-11-13), first released with 0.14.0; its packages are published with the `…-desktop-source.tar.gz` archive (`NOTICE.md`, Source code) |

A GPL core conflicts with App Store terms, which matters for the iOS build. The
owner's direction is in [`core-merge/PLAN-CLIENTS.md`](core-merge/PLAN-CLIENTS.md), section 0.4. The
inventory is [`licensing/IOS-INVENTORY.md`](licensing/IOS-INVENTORY.md); the licence statement and the third-party
notices are in `NOTICE.md`.

"DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no
rights to them.
