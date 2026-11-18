<!-- SPDX-License-Identifier: GPL-3.0-only -->
# uConsole for DSPi — copyright, licence and additional terms

> **DRAFT.** Not final legal text: pending the owner's approval (and a lawyer's reading). `assets/about.json` is
> marked `"draft": true` until then, and the release gate fails while it is.

<!-- WP LIC1, CLEAN1: the owner's decisions of 2026-11-08 and 2026-11-11. The same words are in assets/about.json
(`licence`), which every client shows in its About, in apps/android/NOTICE.md and in
apps/desktop/packaging/licences/TERMS.txt; a test in crates/dspi-session (about.rs) holds them equal. -->

- The unified console, the core merge, the Android console and the Rust Link client: Copyright (C) 2026 Bofr5555 (https://github.com/Bofr5555).
- DSPi, the DSPi firmware, DSPi Console for macOS, DSPi Console for Windows, DSPi-Terminal and DSPi-Link: Copyright (C) Weeb Labs (https://github.com/WeebLabs).

"DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no rights to them.

uConsole for DSPi (the Qt console for Linux, Windows and macOS, the Android console, the Swift console for macOS and iOS,
the DSPi Terminal, the web console and the hub) combines code under different terms. Bofr5555's own contributions are
free software: you can redistribute them and/or modify them under the terms of the GNU General Public License as
published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version, with
the additional terms below. The parts by Weeb Labs are under the GNU GPL version 3 (the DSPi firmware, DSPi Console for
macOS, DSPi Console for Windows) or version 3 or later (DSPi-Terminal), and DSPi-Link under the MIT licence. The program
as a whole is therefore distributed under the GNU GPL version 3. It is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See
the GNU General Public License for details.
[`LICENSE`](LICENSE) holds the GPL version 3 text. The Cargo workspace's field, `GPL-3.0-or-later`, is the licence of
Bofr5555's own contributions; the binaries built from the workspace carry the parts listed below and are distributed
under the GPL version 3.

## Additional terms (GNU GPL v3, section 7)

These terms apply to the material Bofr5555 added to this work (section 7, paragraph 2). They do not restrict the rights
Weeb Labs' GPL-3.0 code carries on its own.

1. **Attributions (7b).** Keep the author attributions — "DSPi and its firmware by Weeb Labs", "Unified console by Bofr5555" — and the licence
   notice and licences view in each client's About, or in an equivalent place a user can reach from the client.
2. **Modified versions are marked (7c).** A distributed modified version must be clearly marked as different from the original. It must not be presented as one
   of these clients or as an official Weeb Labs app: it uses its own application ID, its own app name and its own
   icon.
3. **No trademark rights (7e).** This licence grants no rights under trademark law to the names "DSPi" and "DSPi Console" (Weeb Labs) or to the
   clients' icons and store graphics.

Everything else the GPL-3.0 allows stays allowed: you may study, change, rebuild and share the clients, including for a
fee, as long as you pass the same freedoms on and publish your source.

Each client shows this notice and these terms where it shows licences: the Qt console's Settings › About ›
Licences, the Android console's Settings › About › Licences, the Swift console's Settings › About (macOS and iOS),
the web console's About, the DSPi Terminal's Settings › About and `uconsole --license` (which the hub shares, as part
of `uconsole`). The desktop packages carry it in `NOTICE.txt` and `LICENSES/TERMS.txt` (AppImage, Windows, .dmg), in
`copyright` (.deb) and in `share/licenses/uconsole-for-dspi/` (.rpm and a plain install).

## Source code

The complete source code of each release is published with the release, as its -source.tar.gz archive. Whoever passes
this program on, a hub that serves the web console included, passes on your right to that source. `ci/release-source.sh`
makes it: a `git archive` of the exact commit the release was built from (the Qt desktop app's `apps/desktop` left out,
apart from `apps/desktop/packaging/licences/`, which builds the Swift app's notices), named
`uConsole-for-DSPi-<version>-source.tar.gz`. Every non-Qt artifact of a release (the Terminal archives, the web console,
the Android packages, the Swift app) is published with it, and their notices point at it rather than at this repository.
The Qt packages are published with `uConsole-for-DSPi-<version>-desktop-source.tar.gz` (`--with-desktop`: all of
`apps/desktop` included), which their notices name.

## Who made what

The facts behind this file, the iOS build first, are in [`docs/licensing/IOS-INVENTORY.md`](docs/licensing/IOS-INVENTORY.md);
the README has the per-directory table.

- **DSPi, the DSPi firmware, DSPi Console for macOS, DSPi Console for Windows, DSPi-Terminal and DSPi-Link:** Weeb Labs
  (https://github.com/WeebLabs).
- **The core merge, the unified console, the Android console and the Rust Link client:** Bofr5555 (https://github.com/Bofr5555).

## Material under the GNU GPL version 3 only

<!-- One place for this list (WP CLEAN1): later work may replace these parts. -->

These parts come from Weeb Labs' work under the GNU GPL version 3, without "or later". Changes Bofr5555 made to them are
offered under the same terms.

- **Ported from DSPi Console for Windows** (https://github.com/WeebLabs/DSPi-Console-Windows; its README states GPL v3),
  the Android console's PEQ graph editor, pin overview and hardware-change order and a throttle test, in these 13 files
  of `apps/android/`:
  - `bridge/src/test/kotlin/se/bofr/dspi/bridge/ThrottleTest.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqEditorTypes.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqGraphEditor.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqGraphEditorView.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqGraphModel.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqGraphRenderer.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqGraphSupport.kt`
  - `feature-graph/src/main/kotlin/se/bofr/dspi/feature/graph/PeqShapeChooserState.kt`
  - `feature-graph/src/test/kotlin/se/bofr/dspi/feature/graph/PeqGraphEditorTest.kt`
  - `feature-settings/src/main/kotlin/se/bofr/dspi/feature/settings/HardwareChange.kt`
  - `feature-settings/src/main/kotlin/se/bofr/dspi/feature/settings/PinAssignments.kt`
  - `feature-settings/src/main/kotlin/se/bofr/dspi/feature/settings/PinOverviewTab.kt`
  - `feature-settings/src/test/kotlin/se/bofr/dspi/feature/settings/HardwareChangeOrderTest.kt`
- **From DSPi Console for macOS** (`apps/apple/`, https://github.com/WeebLabs/DSPi-Console; its `LICENSE` says GPL-3.0),
  and the UI, behaviour and texts the Qt, Android, web and Terminal clients port from it.
- **The DSPi firmware** (`crates/dspi-proto/firmware/` headers and its `protocol/` registry and legacy map; the bundled `.uf2` images, below).

## Parts with their own words

| Part | What it says | Where |
|---|---|---|
| `tests/fixtures/link/` | MIT, Copyright (c) 2026 Weeb Labs | `tests/fixtures/link/LICENSE.DSPi-Link` |
| `crates/dspi-hub/assets/web/hub/` | DSPi-Link's web page (`js/apps/web`), built unmodified; MIT, Copyright (c) 2026 Weeb Labs. `uconsole hub --web` serves it at `/hub/` and its licence at `/THIRD-PARTY-NOTICES.txt` | `crates/dspi-hub/assets/web/hub/LICENSE`, `PROVENANCE.json` |
| `tools/esp-rig/firmware/net/net_qemu.c` | Three functions copied from DSPi-Link's `bridge/esp32/components/net/net.c`, MIT, Copyright (c) 2026 Weeb Labs (named in the file). The rig's flash image (upstream's MIT firmware with ESP-IDF's Apache-2.0 components, run in Espressif's GPL-2.0 QEMU as a tool) is built and used locally only and never distributed | `tools/esp-rig/firmware/net/LICENSE.DSPi-Link` |
| `crates/dspi-link/` | The owner's, from `dspi-android`; follows Weeb Labs' MIT DSPi-Link spec and policy | its `Cargo.toml` |
| `apps/apple/`, and the UI, behaviour and texts the Qt, Android, web and Terminal clients port from it | Weeb Labs' DSPi Console for macOS: its `LICENSE` says GPL-3.0 (version 3, no "or later"); see the section above | `apps/apple/LICENSE`, https://github.com/WeebLabs/DSPi-Console |
| `crates/dspi-proto/firmware/` | Vendored DSPi firmware headers, GPL-3.0 | `PROVENANCE.toml` there |
| `crates/dspi-proto/firmware/protocol/` | <!-- WP PGPROTO --> Weeb Labs' `dspi_commands.json` and `legacy_map.json` from the DSPi firmware repository (`Documentation/protocol/`, `release/v1.1.7` at `103ba37`), copied unmodified: data, GPL-3.0 like that repository; `dspi-proto` generates its paged-protocol tables from them at build time. Our workspace's own code stays GPL-3.0-or-later; the binaries carrying the generated tables are distributed under the GPL version 3, as above | `PROVENANCE.toml` `[protocol]` there |
| `apps/desktop/qt/firmware/*.uf2`, `apps/android/feature-firmware/src/main/assets/firmware/*.uf2`, `apps/apple/DSPi Console/Firmware/*.uf2` | Weeb Labs' DSPi firmware release v1.1.6, unmodified; GPL-3.0, with third-party parts (Raspberry Pi's pico-sdk and pico-extras, BSD-3-Clause; TinyUSB, MIT; LUFA, Dean Camera; Elehobica's S/PDIF receiver, BSD-2-Clause; Alex Stanoev's original code, BSD-3-Clause): their notices are in `assets/licences/firmware-THIRD-PARTY-NOTICES.txt`, which every package that bundles the images carries | https://github.com/WeebLabs/DSPi/tree/v1.1.6 (commit 9ffce34), mirrored as tag `v1.1.6` in https://github.com/Bofr5555/DSPi |
| Icon glyphs (`make_graphics.py`'s app icon) | Outlines of Noto Sans Mono glyphs; the font is under the SIL Open Font License 1.1, Copyright 2022 The Noto Project Authors | https://github.com/notofonts/latin-greek-cyrillic |
| `crates/dspi-session/autoeq/` | AutoEq data. AutoEq's `LICENSE` is MIT, Copyright (c) 2018-2022 Jaakko Pasanen; the measurement sources it credits are not covered by anything read so far | https://github.com/jaakkopasanen/AutoEq |

## Third-party Rust crates

`Cargo.lock` lists them; each carries its own licence, and `deny.toml` lists those allowed. They are MIT, Apache-2.0,
BSD, Zlib, Unicode or Unlicense, plus MPL-2.0 for `uniffi*`, `cbindgen` and `option-ext`. The Terminal alone also carries ISC (`ring`, `rustls-webpki`,
`untrusted`) and CDLA-Permissive-2.0 (`webpki-roots`, Mozilla's root list). The Qt app links Qt under the LGPL-3.

Each package carries the licence texts of the crates compiled into it: the Terminal archives `THIRD-PARTY-NOTICES.txt`
(`apps/terminal/notices/rust-crates.txt`, which `uconsole --third-party-licences` also prints), the desktop packages
`LICENSES/rust-crates.txt`, the Android app `feature-settings/src/main/assets/licenses/rust-crates.txt`, the web console
`THIRD-PARTY-NOTICES.txt` (`apps/web/notices/rust-crates.txt`) and the Swift app `apps/apple/licences/rust-crates.txt`.

## The web console's JavaScript

<!-- WP WB7 -->
`apps/web` bundles Preact (MIT, Copyright (c) 2015-present Jason Miller) as its one runtime package; the lockfile pins it and
`ci/licence-inventory.py` records it. A built console carries `THIRD-PARTY-NOTICES.txt` with Preact's licence and the Rust
crates of its core (`apps/web/notices/rust-crates.txt`).
