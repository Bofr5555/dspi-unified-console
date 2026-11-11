# DSPi Console for Android — copyright, licence and additional terms

<!-- WP LIC1: the project's statement and terms (root NOTICE.md, assets/about.json `licence`),
pending the owner's approval. A test in crates/dspi-session holds the copies equal. -->

The unified console, the core merge, the Android console and the Rust Link client: Copyright
(C) 2026 Bofr5555 (https://github.com/Bofr5555).
DSPi, the DSPi firmware, the DSPi Consoles and DSPi-Terminal: Copyright (C) Weeb Labs
(https://github.com/WeebLabs).

DSPi uConsole (the Qt console for Linux, Windows and macOS, the Android console, the Swift
console for macOS and iOS, the DSPi Terminal, the web console and the hub) is free software:
you can redistribute it and/or modify it under the terms of the GNU General Public License as
published by the Free Software Foundation, either version 3 of the License, or (at your option)
any later version, with the additional terms below. It is distributed in the hope that it will
be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for details.
See [`LICENSE`](LICENSE) (the GPL version 3 text, as usual for "or later") and the project's
[`NOTICE.md`](../../NOTICE.md).

## Additional terms (GNU GPL v3, section 7)

These terms apply to the material Bofr5555 added to this work (section 7, paragraph 2). They do
not restrict the rights Weeb Labs' GPL-3.0 code carries on its own.

1. **Attributions (7b).** Keep the author attributions — "Made with love by Weeb Labs",
   "Unified console by Bofr5555" — and the licence notice and licences view in each client's
   About, or in an equivalent place a user can reach from the client.
2. **Modified versions are marked (7c).** A distributed modified version must be clearly marked
   as different from the original. It must not be presented as one of these clients or as an
   official Weeb Labs app: it uses its own application ID, its own app name and its own icon.
3. **No trademark rights (7e).** This licence grants no rights under trademark law to the names
   "DSPi" and "DSPi Console" (Weeb Labs) or to the clients' icons and store graphics.

Everything else the GPL-3.0 allows stays allowed: you may study, change, rebuild and share the
clients, including for a fee, as long as you pass the same freedoms on and publish your source.

The Android console shows this notice and these terms in Settings › About › Licences.

## Third-party components

The app includes or links:

| Component | Licence | Source |
|---|---|---|
| DSPi core (the monorepo's `crates/`: Weeb Labs' DSPi-Terminal and consoles, extended by Bofr5555; firmware headers vendored in `crates/dspi-proto/firmware/`) | GPL-3.0-or-later with the additional terms above (the owner's decision, 2026-11-08; the project's `NOTICE.md`) | https://github.com/WeebLabs/DSPi-Terminal |
| DSPi Console for macOS — UI, behaviour and texts ported from it; AutoEQ database | GPL-3.0 (its `LICENSE`: version 3, without "or later"; an open point for the owner and Weeb Labs, see the project's README) | https://github.com/WeebLabs/DSPi-Console |
| DSPi Console for Windows — the graph editor's logic and the hardware pin and change-order rules ported from it (`feature-graph`: `PeqEditorTypes.kt`, `PeqGraphEditor.kt`, `PeqGraphEditorView.kt`, `PeqGraphModel.kt`, `PeqGraphRenderer.kt`, `PeqGraphSupport.kt`, `PeqShapeChooserState.kt`, test `PeqGraphEditorTest.kt`; `feature-settings`: `HardwareChange.kt`, `PinAssignments.kt`, `PinOverviewTab.kt`, test `HardwareChangeOrderTest.kt`; `bridge` test `ThrottleTest.kt`) | GPL-3.0 (the owner's decision) | DSPi-Console-Windows (Weeb Labs) |
| DSPi Link protocol spec, policy table and test fixtures (`fixtures/link/`, tests only); the Link client follows the macOS Console's `Link/` | MIT | https://github.com/WeebLabs/DSPi-Link |
| DSPi firmware images (`feature-firmware` assets, v1.1.6-beta5) | GPL-3.0 | https://github.com/WeebLabs/DSPi/tree/v1.1.6-beta5 |
| AutoEq headphone measurements (in the AutoEQ database) | MIT | https://github.com/jaakkopasanen/AutoEq |
| UniFFI | MPL-2.0 | https://github.com/mozilla/uniffi-rs |
| Other Rust crates | MIT / Apache-2.0 / Unlicense (as chosen) | listed with their licence files in `feature-settings/src/main/assets/licenses/rust-crates.txt` |
| JNA | Apache-2.0 (dual-licensed with LGPL-2.1) | https://github.com/java-native-access/jna |
| AndroidX, Jetpack Compose, Kotlin, kotlinx, Guava listenablefuture, Okio, JSpecify | Apache-2.0 | https://developer.android.com/jetpack/androidx |

The full licence texts ship in the app (Settings › About › Licences) and live in
`feature-settings/src/main/assets/licenses/`. Regenerate the Rust list with
`tools/dev.sh python3 tools/gen_third_party_licenses.py`.
