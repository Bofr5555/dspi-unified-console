## 8. Notes read by hand

*DRAFT where it states licences (`NOTICE.md` is the statement). Written by the LC1 agent and included as is by the generator. Facts with where they came from; not legal text. Dated 2026-11-06: re-read them when the numbers above change.*

### What cannot go in on a GPL grant alone

The plan's reading (0.4) is that GPL terms and App Store usage rules conflict; the owner and Weeb Labs settle that. On that reading, these are the parts that need attention, largest first:

1. **All first-party code**, Rust and Swift, until the owner and Weeb Labs grant their parts.
2. **The two `.uf2` images.** Confirmed from `project.pbxproj`: the iOS target's folder exceptions name no `.uf2`, so both are members. iOS has no USB bootloader path, and SW18 removes them. Doing that also removes the firmware's other authors and its third-party parts from the question.
3. **`dspi-proto`'s generated constants. The plan's list does not cover this.** `build.rs` turns 13 vendored GPL firmware headers into Rust constants, so taking out the images does not take the firmware out of the binary entirely. Section 3 has the authors. Whether numeric protocol constants carry anything to license is for them to judge. Only Jonathan Rosser's 10 lines in `config.h` are outside Weeb Labs and the owner: comments, `Filter` struct lines and filter-type enum values. The firmware struct is not the wire format.

### Jonathan Rosser

The one person outside Weeb Labs and the owner whose lines reach the iOS binary.

- **Swift, 19 lines in 4 files,** from two commits of 2026-04-20 ("Add allpass filter type", "Add notch filter"): the `.notch` and `.allPass` cases, their display names, the numerator coefficients in `DSPMath.swift`, and the firmware checks that hide them. `Commands.swift` holds none, so its removal by SW does not remove them.
- **Firmware headers, 10 lines,** as above.
- **`.uf2` images, 1117 lines,** only until SW18.
- **Rust core: none by git.** `dspi-proto`'s `FILTER_TYPE` table was written by others. Whether it follows their Swift or firmware work was not traced.

The other firmware authors, Tim Prangenberg (172 lines) and Jeremy Blencowe (no surviving line), matter only for the images. Neither appears in the Swift or the Rust.

### AutoEQ data (`autoeq_database.json`, 5.4 MB)

- It is the only large third-party data in the target, and it holds computed EQ settings, 3 828 entries: crinacle 1 583, innerfidelity 767, oratory1990 736, rtings 605, headphone.com 137.
- **Upstream read 2026-11-06,** `jaakkopasanen/AutoEq` at `7ae0f56`: `LICENSE` is MIT, "Copyright (c) 2018-2022 Jaakko Pasanen". Its README credits those five sources as the origin of its measurements and has no licence section. No `README` exists under `measurements/` at the paths tried.
- **So MIT is certain for AutoEq's own work and uncertain for the sources' data.** Android's `NOTICE.md` and this repo's README record plain MIT; that is a simplification.
- The Rust copy (`crates/dspi-session/autoeq/`) is **not** in the iOS binary: `bundled-autoeq` is off in `dspi-core`'s dependency, and dep-info confirms it. `AutoEQManager.swift` also fetches updates from AutoEq's GitHub at run time.

### Other trees, and where the words disagree

Read 2026-11-06 at the commits named. Authors are the full history of those clones, except the three marked shallow.

| Tree | What it says | Authors | Reaches iOS? |
|---|---|---|---|
| DSPi-Console (`release/v1.1.6`, fork `48cf7c5`) | **`LICENSE` is GPL-3.0; `README.md:5` badge and `:2066` say MIT.** `LICENSE` changed in `5c823ee`, 2026-02-05, by Weeb Labs: "License updated to GPL v3"; the README was not. Weeb Labs told the owner MIT | Weeb Labs, Jonathan Rosser (2), owner on the fork (31) | **Yes, the Swift half** |
| DSPi firmware (`v1.1.6` = `9ffce34`) | GPL-3.0 (`LICENSE`, README); LUFA and pico-extras have their own files, pico-sdk is a submodule | Weeb Labs, Jonathan Rosser (17), Jeremy Blencowe (1), Tim Prangenberg (1); on `bofr-main` also the owner (20) | Images until SW18; headers as constants |
| DSPi-Terminal (`8389b20`) | `GPL-3.0-or-later` in `Cargo.toml` and README; **no `LICENSE` file** | Weeb Labs only | **Yes:** `dspi-proto`, `-session`, `-transport` |
| DSPi-Console-Windows (`9f65fd0`) | **No `LICENSE` file;** README says GPL-3.0 | Weeb Labs only | No |
| DSPi-Link (`05949b8`, shallow) | MIT (`LICENSE`, Copyright (c) 2026 Weeb Labs) | Weeb Labs at the tip | Yes: spec behind `dspi-link`, `commands.json`, web bundle |
| `dspi-android` (`63492af`) | GPL-3.0 with §7 terms by Bofr5555 (`LICENSE`, `NOTICE.md`). Its Rust workspace field says **`UNLICENSED`** | Owner only | Yes: `dspi-link` came from here. Its Kotlin does not |
| `dspi-unified-console` | Root `LICENSE` GPL-3.0. DRAFT statement (`NOTICE.md`, WP CLEAN1): the program as a whole is distributed under GPL-3.0; Bofr5555's own additions are GPL-3.0-or-later with three §7 terms, which is what the workspace field `GPL-3.0-or-later` describes | Weeb Labs, owner | **Yes** |
| sigman78 web console, dspictl (shallow) | MIT | Tips by SiGMan, Steffen Uhlig | No |
| DSPi-ESP32-Front-Panel, DSPiCliRemote (shallow) | No licence | Not checked beyond the tip | No: do not copy |

**Weeb Labs' agreement.** On 2026-11-02 Weeb Labs agreed to the Android app being published. The project relies on licence files only: the macOS Console's `LICENSE` (GPL-3.0), the firmware's (GPL-3.0), DSPi-Link's (MIT) and DSPi-Terminal's manifests (GPL-3.0-or-later). The project's own statement is `NOTICE.md` (DRAFT).

**The metadata fix for `dspi-link`.** Its manifest now inherits the workspace's `GPL-3.0-or-later`, which is what this repo says and what the plan's entry asks for. Android's words are "version 3" with §7 terms and that crate's old workspace said `UNLICENSED`. The owner is its only author by git, and the coordinator relays the owner's confirmation of the field, 2026-11-06: "gpl3 seems to only be an issue on ios". Nothing relayed covers the absence of the §7 terms here. Its MIT origin (spec, policy, fixtures) is recorded in the manifest.

### Limits of this inventory

- **Blame shows who last changed a line, not who owns it.** `-C` follows moved code within a commit; rewrites and ports are not followed, hence `CRATE_ORIGINS` in the script.
- **Today's iOS app does not link the Rust core yet.** Both halves are listed because the target state has both; SW removes much of the Swift, starting with `Commands.swift`. The Swift `#if os(macOS)` code in member files is counted though it does not compile for iOS.
- **The crate list and the file list are both the real iOS target's:** `cargo tree` and `cargo check --target aarch64-apple-ios`, without `dspi-ffi`'s default `usb`, so `nusb` is out of both (WP CG3). The script fails loudly on a platform-gated `mod`; code gated inside a file is counted.
- **The `.uf2` images were not rebuilt and compared** with the tag they are named after.
- **Crate licences are the manifests' words.** `cargo-deny` checks them; nobody read each crate's files. Apple's SDK modules are not listed.

