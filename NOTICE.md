<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Notices

**A record of what the parts of this repository say, not a licence and not final.** The repository is private. The
terms are open between the owner and Weeb Labs (`docs/core-merge/PLAN-CLIENTS.md`, section 0.4). The facts behind this
file, the iOS build first, are in [`docs/licensing/IOS-INVENTORY.md`](docs/licensing/IOS-INVENTORY.md); the README has
the per-directory table.

- **DSPi, the DSPi firmware, the DSPi Consoles and DSPi-Terminal:** Weeb Labs (https://github.com/WeebLabs).
  "DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no rights to them.
- **The core merge, the Android console and the Rust Link client:** Bofr5555 (https://github.com/Bofr5555).
- **The repository as a whole:** `GPL-3.0-or-later` (the owner's decision, 2026-11-08). Root `LICENSE` holds the GPL
  version 3 text, as usual for "or later"; the Cargo workspace's field is `GPL-3.0-or-later`.

## Parts with their own words

| Part | What it says | Where |
|---|---|---|
| `tests/fixtures/link/` | MIT, Copyright (c) 2026 Weeb Labs | `tests/fixtures/link/LICENSE.DSPi-Link` |
| `crates/dspi-hub/assets/web/hub/` | DSPi-Link's web page (`js/apps/web`), built unmodified; MIT, Copyright (c) 2026 Weeb Labs. `dspi hub --web` serves it at `/hub/` and its licence at `/THIRD-PARTY-NOTICES.txt` | `crates/dspi-hub/assets/web/hub/LICENSE`, `PROVENANCE.json` |
| `crates/dspi-link/` | The owner's, from `dspi-android`; follows Weeb Labs' MIT DSPi-Link spec and policy | its `Cargo.toml` |
| `crates/dspi-proto/firmware/` | Vendored DSPi firmware headers, GPL-3.0 | `PROVENANCE.toml` there |
| `apps/desktop/qt/firmware/*.uf2` | DSPi firmware images, GPL-3.0, with third-party parts | https://github.com/WeebLabs/DSPi |
| `crates/dspi-session/autoeq/`, `apps/desktop/qt/autoeq/` | AutoEq data. AutoEq's `LICENSE` is MIT, Copyright (c) 2018-2022 Jaakko Pasanen; the measurement sources it credits are not covered by anything read so far | https://github.com/jaakkopasanen/AutoEq |

## Third-party Rust crates

`Cargo.lock` lists them; each carries its own licence, and `deny.toml` lists those allowed. They are MIT, Apache-2.0,
BSD, Zlib, Unicode or Unlicense, plus MPL-2.0 for `uniffi*`, `cbindgen` and `option-ext`. The Terminal alone also carries ISC (`ring`, `rustls-webpki`,
`untrusted`) and CDLA-Permissive-2.0 (`webpki-roots`, Mozilla's root list). The Qt app links Qt under the LGPL-3.

## The web console's JavaScript

<!-- WP WB7 -->
`apps/web` bundles Preact (MIT, Copyright (c) 2015-present Jason Miller) as its one runtime package; the lockfile pins it and
`ci/licence-inventory.py` records it. A built console carries `THIRD-PARTY-NOTICES.txt` with Preact's licence and the Rust
crates of its core (`apps/web/notices/rust-crates.txt`).
