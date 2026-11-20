# DSPi Console for Android

A native Android console for the [Weeb Labs DSPi](https://github.com/WeebLabs/DSPi) USB audio DSP
firmware (Raspberry Pi Pico / Pico 2). It aims to do everything the official desktop consoles do,
from a phone or tablet — including while that phone is the USB audio source playing into the DSPi.

The app is a faithful port of the official clients rather than a re-implementation:

- **Protocol and state:** the Rust core in this repository's [`crates/`](../../crates) (`dspi-core` and
  the crates under it), which comes from Weeb Labs' DSPi-Terminal and consoles and is shared with the
  other clients. The app opens the device from the file descriptor Android hands it, and uses the
  optional crossfeed pair map (see below).
- **Behaviour, wording and layout:** the macOS DSPi Console (1.1.6) is the reference, page by
  page; the Qt bridge (`DSPiBridge.cpp`) is ported 1:1 as `DspiBridge`. Every deliberate difference is
  listed in [`docs/UPSTREAM-DEVIATIONS.md`](docs/UPSTREAM-DEVIATIONS.md).

It is one of the clients in the `dspi-unified-console` repository; the root
[`README.md`](../../README.md) is the project's front page; the status of each client is in
[`docs/STATUS.md`](../../docs/STATUS.md) and the credits in [`docs/CREDITS.md`](../../docs/CREDITS.md).

> Status: version **0.13.0** (versionCode 13) is distributed through Google Play (app ID
> `se.bofr.dspi_console_android`), in closed testing; 0.12.0 (tag `android-v0.12.0`) was the first release on the
> core. *(2026-11-20: `main` builds 0.14.0, versionCode 14, for the next release; what it adds is in
> [`docs/STATUS.md`](../../docs/STATUS.md) and the changelog. Installing: [`docs/INSTALL.md`](../../docs/INSTALL.md#android);
> what works where, and which features need the DSPi fork firmware (†):
> [`docs/FEATURES.md`](../../docs/FEATURES.md).)*

What you need: an Android phone or tablet with Android 8.0 or newer. **No DSPi required**: the app has a
Demo mode, so every screen can be tried without hardware. With a DSPi: firmware **v1.1.6** for full
features (see [Requirements](#requirements) for older firmware) and a USB-C/OTG cable or adapter.

## Features

Home and channel pages with live meters · response graph with on-graph PEQ editing (touch gestures) ·
crossovers, delays, gains, input routing · matrix mixer · loudness, crossfeed, volume leveller,
psychoacoustic bass, subharmonic synth, tube modeller, upmixer, output limiters · presets with the
official Unsaved Changes flow, import/export, filter files, AutoEQ · settings with the Save / Revert
bar (pins, outputs, I2S, inputs, global parameters, control interfaces, control surfaces) · test
signals, spectrum analyser, statistics, interrupt monitor · firmware installer (the bundled 1.1.6
images or a local `.uf2`) · demo mode without a device.

DSPi Link (optional, off by default; Settings › Networking): control a DSPi through a DSPi Link hub
on the local network, such as DSPi Console on a Mac, this repository's `uconsole hub` or Qt app, or a DSPi
ESP32 bridge — hub discovery or address entry, PIN pairing, reconnect. *(Until 0.12.0 the protocol was
plaintext. On `main` since 2026-11-16, WP SECAN:)* the connection to this project's hubs is TLS, with the
hub's key pinned at pairing (compare the fingerprint the hub shows with its PIN); a hub without TLS,
such as an upstream one, shows as unsecured
([`docs/integrator/security.md`](../../docs/integrator/security.md)).

Since 0.12.0, all through the core: the PRO phase 1 screens (snapshots,
A/B, safe apply, limits profiles, the installer lock and PIN, offline documents and templates, document
diff and partial import, AutoEQ preview), the hub's limits, PIN and pinned modes (HUBADMIN); shared
snapshots, the hub's history, the alarm banner and Identify † (P2AN); the device info with the firmware
generation, the update check before the bootloader entry and the core's write gate for firmware 1.1.6
and 1.1.7 (PGAND); the audio link's System screen † with S/PDIF units, view only; and the alink sender's own
page †, where the write keys do nothing (QTSENDER). Measurement and auto-tune come in a
later batch (`docs/core-merge/PLAN-P5.md`).

Android specifics: auto-launch on plug-in, a foreground-service notification with volume / mute /
next preset, and the app claims only the DSPi's vendor control interface, so audio keeps playing
(Android's own USB audio or a player such as USB Audio Player PRO).

## Requirements

- Android 8.0 (API 26) or newer with USB host (OTG) support; arm64, armv7 and x86_64 builds.
- **DSPi firmware on bulk wire format V32:** v1.1.6, or its betas 4 and 5. This is full control, and
  no firmware banner shows, whichever of these a device runs, the K340 fork below included.
- On any other wire format the app connects **read-only**: it shows a banner, disables every control
  and never writes to the device, and the firmware updater stays available. What it shows depends on
  the format:
  - **V17–V31** (v1.1.5-beta3 to v1.1.6-beta3): that firmware's settings. Tools and rows it does not
    have are hidden, and so, for now, are the hardware options under the channel and settings pages.
  - **Newer than V32:** the settings this version of the app understands.
- *(On `main`, 2026-11-19.)* Upstream firmware **1.1.7**, which speaks the newer paged protocol with the
  same V32 settings image, is full control through the core's adapter; firmware with a newer protocol
  major is refused with a notice ([`docs/manual/notices.md`](../../docs/manual/notices.md)).
  - **Older, or a format the app cannot map:** the settings cannot be read, and the banner says the
    values on screen are defaults.
- † **Needs the DSPi fork firmware** on the device ([Bofr5555/DSPi](https://github.com/Bofr5555/DSPi), branch
  `bofr-main`): the crossfeed pair map, the core voltage, Identify and the audio link (the System screen,
  USB-direct and S/PDIF units, the alink sender). Stock firmware has none of these, and the app does not offer
  them for it ([`docs/FEATURES.md`](../../docs/FEATURES.md#firmware-stock-vs-fork)).

## Installing without Google Play

A universal `.apk` (arm64, armv7, x86_64) can be made from a release bundle with `bundletool
build-apks --mode=universal` ([`docs/RELEASE.md`](docs/RELEASE.md), section 4). It is signed with the
developer's upload key, not Google Play's key, so it cannot update a Play install and Play cannot update
it: to switch between the two, uninstall first.

## Fork-only extra: crossfeed output pairs

DSPi crossfeed runs on fixed output pairs (1/2, 3/4, …). Boards that wire **one DAC per ear** (the
K340 amplifier this was built for: out1 R woofer, out2 R tweeter, out3 L woofer, out4 L tweeter) need
crossfeed between outputs 3↔1 and 4↔2 instead. The firmware branch
[`Bofr5555/DSPi` → `bofr-main`](https://github.com/Bofr5555/DSPi/tree/bofr-main) makes the pairs
configurable (default unchanged, stored in the preset without a format change; set and read through
the firmware's extension channel, opcode 0x00); the app then shows a
**Pairs** row in the crossfeed tool. On official firmware the row is hidden and nothing changes.

## Building

All toolchains run in a podman container (JDK 21, Android SDK/NDK, Rust + cargo-ndk, UniFFI), built
from [`container/Containerfile`](container/Containerfile) with every download pinned and checksum-
verified ([`docs/SUPPLY-CHAIN.md`](docs/SUPPLY-CHAIN.md)). Nothing is installed on the host.

From the repository root, `ci/check.sh --step android` builds the core libraries, `--step
android-tests` runs the app's tests and `--step android-aab` builds a bundle (see the root `README.md`).
By hand:

```bash
# from a checkout of this repository
cd apps/android
tools/debug-keystore.sh                     # once: persistent debug signing key (gitignored)
tools/ci.sh all                             # every package's checks, in the container
tools/dev.sh ./gradlew :app:assembleDebug -Pdspi.rustProfile=release
```

The APK lands in `app/build/outputs/apk/debug/`. `tools/dev.sh <cmd>` runs any command in the
container; the container never gets USB or device access.

## Publishing

Release builds are signed Android App Bundles for Google Play (target API 36):
`tools/upload-key.sh` once (keep the key safe), then `tools/dev.sh ./gradlew :app:bundleRelease`;
`tools/ci.sh release` validates a bundle with a throwaway key. The full checklist — Play Console,
app ID, Play App Signing, testing tracks, store listing, privacy and licensing — is in
[`docs/RELEASE.md`](docs/RELEASE.md).

## Project layout

| Path | What |
|---|---|
| `../../crates/` | the shared Rust core; `crates/dspi-ffi` is its UniFFI library (Kotlin contract generated by `tools/gen_kotlin_contract.py`) |
| `core-api`, `ffi` | Kotlin contract + generated bindings / native libraries |
| `bridge` | `DspiBridge`: port of the Qt bridge (polling, notifications, throttled live edits, confirmation gate for flash writes, read-only guard) |
| `fake-core` | stateful fake device for tests and previews |
| `usb` | USB permission / attach / foreground service / bootloader locator |
| `ui-common`, `feature-*` | Compose UI (home, graph, matrix, tools, presets, settings, analysis, firmware) |
| `app` | wiring, navigation, demo mode |
| `docs/` | package map, macOS→Android port map, deviations, supply chain |

## Credits and licensing

DSPi firmware and the official consoles are by **Weeb Labs** — all protocol knowledge, the core and
the UI design come from their work. Android port by **Bofr5555**.

This app is free software under the **GNU GPL v3.0** ([`LICENSE`](LICENSE)) with additional terms
under section 7 ([`NOTICE.md`](NOTICE.md)): keep the attributions, mark modified versions and give
them their own name, app ID and icon, and no trademark rights to "DSPi" or the icon. Third-party
components (the Weeb Labs core, firmware and AutoEQ database, libusb, UniFFI, Rust crates, AndroidX…)
and their licences are listed in `NOTICE.md` and in the app under Settings › About › Licences.

Privacy: the app collects no data. It uses the network only for the optional DSPi Link feature (off
by default), to talk to a hub you choose on your local network — see [`PRIVACY.md`](PRIVACY.md).
