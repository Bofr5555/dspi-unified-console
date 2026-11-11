# DSPi Console for Android — copyright, licence and additional terms

> **DRAFT.** Not final legal text: pending the owner's approval.

<!-- WP LIC1, CLEAN1: the project's statement and terms (root NOTICE.md, assets/about.json `licence`),
pending the owner's approval. A test in crates/dspi-session holds the copies equal. -->

The unified console, the core merge, the Android console and the Rust Link client: Copyright (C)
2026 Bofr5555 (https://github.com/Bofr5555).
DSPi, the DSPi firmware, DSPi Console for macOS, DSPi Console for Windows, DSPi-Terminal and
DSPi-Link: Copyright (C) Weeb Labs (https://github.com/WeebLabs).

DSPi uConsole (the Qt console for Linux, Windows and macOS, the Android console, the Swift
console for macOS and iOS, the DSPi Terminal, the web console and the hub) combines code under
different terms. Bofr5555's own contributions are free software: you can redistribute them
and/or modify them under the terms of the GNU General Public License as published by the Free
Software Foundation, either version 3 of the License, or (at your option) any later version,
with the additional terms below. The parts by Weeb Labs are under the GNU GPL version 3 (the
DSPi firmware, DSPi Console for macOS, DSPi Console for Windows) or version 3 or later
(DSPi-Terminal), and DSPi-Link under the MIT licence. The program as a whole is therefore
distributed under the GNU GPL version 3. It is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for details.
See [`LICENSE`](LICENSE) (the GPL version 3 text) and the project's [`NOTICE.md`](../../NOTICE.md),
which lists the material under the GPL version 3 only, among it the 12 files of this app ported
from DSPi Console for Windows.

The complete source code of each release is published with it, as the file ending in
-source.tar.gz beside its downloads. The third-party parts and their licences are listed in
NOTICE.md in that source, and in each package's notices.

## Additional terms (GNU GPL v3, section 7)

These terms apply to the material Bofr5555 added to this work (section 7, paragraph 2). They do
not restrict the rights Weeb Labs' GPL-3.0 code carries on its own.

1. **Attributions (7b).** Keep the author attributions — "DSPi and its firmware by Weeb Labs",
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
| DSPi core (the monorepo's `crates/`: Weeb Labs' DSPi-Terminal, extended by Bofr5555; firmware headers vendored in `crates/dspi-proto/firmware/`) | Bofr5555's parts GPL-3.0-or-later with the additional terms above; DSPi-Terminal GPL-3.0-or-later; firmware headers GPL-3.0 | the release's source archive (see above); https://github.com/WeebLabs/DSPi-Terminal |
| DSPi Console for macOS — UI, behaviour and texts ported from it; AutoEQ database | GPL-3.0 (its `LICENSE`: version 3, without "or later") | https://github.com/WeebLabs/DSPi-Console |
| DSPi Console for Windows — the 12 files the project's `NOTICE.md` lists (PEQ graph editor, pin overview, hardware-change order) | GPL-3.0 (its README: "GPL v3") | https://github.com/WeebLabs/DSPi-Console-Windows |
| Noto Sans Mono — glyph outlines in the app icon | SIL Open Font License 1.1 | https://github.com/notofonts/latin-greek-cyrillic |
| DSPi Link protocol spec, policy table and test fixtures (`fixtures/link/`, tests only); the Link client follows the macOS Console's `Link/` | MIT | https://github.com/WeebLabs/DSPi-Link |
| DSPi firmware images (`feature-firmware` assets: Weeb Labs' release v1.1.6, unmodified), with their third-party parts (pico-sdk and pico-extras, BSD-3-Clause; TinyUSB, MIT; LUFA; Elehobica's S/PDIF receiver, BSD-2-Clause; Alex Stanoev, BSD-3-Clause), whose notices ship as `firmware-THIRD-PARTY-NOTICES.txt` | GPL-3.0 | https://github.com/WeebLabs/DSPi/tree/v1.1.6 (commit 9ffce34), mirrored as tag `v1.1.6` in https://github.com/Bofr5555/DSPi |
| AutoEq headphone measurements (in the AutoEQ database) | MIT | https://github.com/jaakkopasanen/AutoEq |
| UniFFI | MPL-2.0 | https://github.com/mozilla/uniffi-rs |
| Other Rust crates | MIT / Apache-2.0 / Unlicense (as chosen) | listed with their licence files in `feature-settings/src/main/assets/licenses/rust-crates.txt` |
| JNA | Apache-2.0 (dual-licensed with LGPL-2.1) | https://github.com/java-native-access/jna |
| AndroidX, Jetpack Compose, Kotlin, kotlinx, Guava listenablefuture, Okio, JSpecify | Apache-2.0 | https://developer.android.com/jetpack/androidx |

The full licence texts ship in the app (Settings › About › Licences) and live in
`feature-settings/src/main/assets/licenses/`. Regenerate the Rust list with
`tools/dev.sh python3 tools/gen_third_party_licenses.py`.
