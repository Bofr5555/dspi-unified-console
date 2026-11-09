<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# DSPi Console — copyright, licence and additional terms

<!-- WP LIC1: the owner's decisions of 2026-11-08, in the wording of apps/android/NOTICE.md adapted to the whole
project. Pending the owner's approval. The same words are in assets/about.json (`licence`), which every client
shows in its About, in apps/android/NOTICE.md and in apps/desktop/packaging/licences/TERMS.txt; a test in
crates/dspi-session (about.rs) holds them equal. -->

- The unified console, the core merge, the Android console and the Rust Link client: Copyright (C) 2026 Bofr5555 (https://github.com/Bofr5555).
- DSPi, the DSPi firmware, the DSPi Consoles and DSPi-Terminal: Copyright (C) Weeb Labs (https://github.com/WeebLabs).

"DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no rights to them.

DSPi Console (the Qt console for Linux, Windows and macOS, the Android console, the Swift console for macOS and iOS, the
DSPi Terminal, the web console and the hub) is free software: you can redistribute it and/or modify it under the terms
of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at
your option) any later version, with the additional terms below. It is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See
the GNU General Public License for details.
[`LICENSE`](LICENSE) holds the GPL version 3 text, as usual for "or later"; the Cargo workspace's field is
`GPL-3.0-or-later`.

## Additional terms (GNU GPL v3, section 7)

These terms apply to the material Bofr5555 added to this work (section 7, paragraph 2). They do not restrict the rights
Weeb Labs' GPL-3.0 code carries on its own.

1. **Attributions (7b).** Keep the author attributions — "Made with love by Weeb Labs", "Unified console by Bofr5555" —
   and the licence notice and licences view in each client's About, or in an equivalent place a user can reach from the
   client.
2. **Modified versions are marked (7c).** A distributed modified version must be clearly marked as different from the
   original. It must not be presented as one of these clients or as an official Weeb Labs app: it uses its own
   application ID, its own app name and its own icon.
3. **No trademark rights (7e).** This licence grants no rights under trademark law to the names "DSPi" and "DSPi
   Console" (Weeb Labs) or to the clients' icons and store graphics.

Everything else the GPL-3.0 allows stays allowed: you may study, change, rebuild and share the clients, including for a
fee, as long as you pass the same freedoms on and publish your source.

Each client shows this notice and these terms where it shows licences: the Qt console's Settings › About ›
Licences, the Android console's Settings › About › Licences, the Swift console's Settings › About (macOS and iOS),
the web console's About, the DSPi Terminal's Settings › About and `dspi --license` (which the hub shares, as part of
`dspi`). The desktop packages carry it in `NOTICE.txt` and `LICENSES/TERMS.txt` (AppImage, Windows, .dmg), in
`copyright` (.deb) and in `share/licenses/dspi-console/` (.rpm and a plain install).

**Open point** (for the owner and Weeb Labs, not settled here): parts of the clients are ported from Weeb Labs'
DSPi Console for macOS, whose `LICENSE` says GPL-3.0 without "or later". README's Licensing section lists it with
the other open points.

## Who made what

The facts behind this file, the iOS build first, are in [`docs/licensing/IOS-INVENTORY.md`](docs/licensing/IOS-INVENTORY.md);
the README has the per-directory table.

- **DSPi, the DSPi firmware, the DSPi Consoles and DSPi-Terminal:** Weeb Labs (https://github.com/WeebLabs).
- **The core merge, the unified console, the Android console and the Rust Link client:** Bofr5555 (https://github.com/Bofr5555).

## Parts with their own words

| Part | What it says | Where |
|---|---|---|
| `tests/fixtures/link/` | MIT, Copyright (c) 2026 Weeb Labs | `tests/fixtures/link/LICENSE.DSPi-Link` |
| `crates/dspi-hub/assets/web/hub/` | DSPi-Link's web page (`js/apps/web`), built unmodified; MIT, Copyright (c) 2026 Weeb Labs. `dspi hub --web` serves it at `/hub/` and its licence at `/THIRD-PARTY-NOTICES.txt` | `crates/dspi-hub/assets/web/hub/LICENSE`, `PROVENANCE.json` |
| `tools/esp-rig/firmware/net/net_qemu.c` | Three functions copied from DSPi-Link's `bridge/esp32/components/net/net.c`, MIT, Copyright (c) 2026 Weeb Labs (named in the file). The rig's flash image (upstream's MIT firmware with ESP-IDF's Apache-2.0 components, run in Espressif's GPL-2.0 QEMU as a tool) is built and used locally only and never distributed | `tools/esp-rig/firmware/net/LICENSE.DSPi-Link` |
| `crates/dspi-link/` | The owner's, from `dspi-android`; follows Weeb Labs' MIT DSPi-Link spec and policy | its `Cargo.toml` |
| `apps/apple/`, and the UI, behaviour and texts the Qt, Android, web and Terminal clients port from it | Weeb Labs' DSPi Console for macOS: its `LICENSE` says GPL-3.0 (version 3, no "or later"). Open point, see above | `apps/apple/LICENSE`, https://github.com/WeebLabs/DSPi-Console |
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
