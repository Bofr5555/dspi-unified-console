# DSPi console apps: feature parity

Written 2026-11-06. This is a code-reading survey: nothing was built or run for it.

**Moved here from `dspi-mobile/docs/FEATURE-PARITY.md` by WP XC0.** Sections 1 to 4 are that survey,
written before the core merge. For Qt and the Terminal, `docs/PORTABILITY.md` section 11 says what the merge changed, and
`docs/core-merge/PLAN-CLIENTS.md` section 2 lists further corrections; only B2 and B23 are corrected in place below.
**The Qt column of sections 1 to 3 and 5 is current (WP POSTSWAP, 2026-11-13):** after the rebuilt Qt app replaced the old
one on `main`, every Qt cell was re-read against `apps/desktop` there (QP21's corrections, then QPI5, SWAPFIX and QP23).
**Section 5 is the macOS floor, audited on this tree: where it disagrees with sections 1 to 4, section 5 wins.**

The reference is the **macOS Swift DSPi Console**: upstream Weeb Labs v1.1.6 (`51fd1bb`) plus our stack, branch `ext-channel-116` (`48cf7c5`). Our stack adds FakeTransport, USB MIDI, the iOS target, the iOS Link client UI and the extension channel. iOS builds from the same sources.

Each other app is compared against that reference:

| App | Source | Version surveyed |
|---|---|---|
| **iOS** | the iOS target of the same Swift project (`ext-channel-116`) | same branch as the reference |
| **Qt desktop** | `dspi-unified-console/apps/desktop`: the rebuilt QML app on the core (C ABI) | `main` after the swap (382d7fe4) |
| **Terminal** | `apps/terminal` + `crates/dspi-{proto,transport,session,cmd,tui}` | before the merge |
| **Android** | `dspi-android` `main` | `63492af`, v0.10.1 |
| **Windows C#** | `DSPi-Console-Windows` | `4f54e36` |

Windows C# is being retired. Its column is for reference only and has no gap list.

Legend:

| Mark | Meaning |
|---|---|
| ✓ | at parity |
| ◐ | partial; the cell says what is missing or different |
| ✗ | absent |
| n.a. | not applicable; the cell gives the reason |
| *(ours)* | our addition on top of upstream v1.1.6 |

"Untested on HW" means the code exists but has not been run against a device.
Many Android packages, and the Terminal's control-surface and physical-control paths, are in that state. The Qt desktop
had its hardware pass on Linux and Windows (QP22, `docs/core-merge/HW-QT.md`); the firmware path (a flash) and a physical
replug on Linux are still owed.

---

## 1. Summary: biggest gaps per app, ranked by user impact

### iOS (shared Swift sources)
1. **No file I/O.** Filter-file import/export and `.dspipreset` config import/export are present in the menu but say "Not available yet". This blocks REW workflows and backups on iPhone/iPad.
2. **Firmware update is effectively dead.** The window opens but the bootloader locator never finds a board. The update itself is n.a. by owner decision (no DriverKit; USB MIDI is the only USB transport), but the dead window should be hidden.
3. **AutoEQ is partial:** no Update Database and no Favourites menu.
4. **PEQ graph editing is partial:** touch editor without marquee, pull-from-curve, shape chooser or value chip. There are no hardware-keyboard shortcuts.
5. **No hub/sharing.** This is by design (an iOS app is never a hub). Onboarding and the host-format monitor are also reduced.

### Qt desktop (Linux/Windows)
The rebuilt app (POSTSWAP). The earlier list (no extension channel, blank on older firmware, a beta4-only updater, missing
settings rows) is closed. Left:
1. **Hardware:** QP22 passed on the K340 on Linux and Windows; the firmware path (a flash), a physical replug on Linux and the
   desktop hand checks QP22 lists are owed (`docs/core-merge/HW-QT.md`).
2. **Orca and languages:** the screen-reader pass is open, and English is the only language.
3. **No USB MIDI**, by design: vendor USB works on Linux and Windows.
4. **No coach marks** (off in the Mac build too).
No macOS build: the Swift app is the Mac release.

### Terminal (TUI/CLI)
1. **No on-graph PEQ editing.** This is out of scope by design (decision 5); the `;` command bar replaces it.
2. **Extension channel is CLI-only.** `uconsole ext list | xfeed-map | vreg` work, but there is no TUI screen and no probe on connect.
3. **No automatic reconnect after unplug/replug.** You have to run `:reconnect`.
4. **USB vendor transport only.** No Link client (the `dspi-link` crate exists but isn't wired in), no MIDI, and the demo fixture answers zeros rather than simulating a device.
5. **Firmware update** only reboots into the bootloader and finds the drive; you copy the `.uf2` by hand.
6. **Older firmware is refused outright** (this is by design); there is no read-only mode.
7. **RTA has no graph overlay and no bar strip.**

### Android (Kotlin/Compose over the patched Linux core)
1. **Older firmware (V17–V30) can't be read at all** on `main` ("settings could not be read"). The read-only decoder exists only on the unmerged branch `wip/wire-compat`. The README overstates this.
2. **Control-surface editor is view/run only:** no binding editor, IR learn, display pages, or group/macro editors.
3. **No core-voltage status.** The ext channel is only probed through the pair map, and EXT_LIST is never sent.
4. **Thin accessibility:** about 13 `contentDescription`s, and the Canvas graph, matrix and meters are mute to TalkBack. **No localisation.**
5. **No onboarding / What's New**, no device-name edit, no RTA bar strip, no host audio-format monitor, and no AutoEQ "Rebuild from GitHub".
6. **Bundled firmware is 1.1.6-beta5, not final.**
7. **No USB MIDI.** This matters little on Android, because USB vendor works there.

### Windows C# (reference only)
It is frozen at wire **V26**. It has nothing from V27–V32 (limiter, subharmonic, tube, extension channel, core voltage), no RTA, no on-graph editing, no MIDI or Link, and no full config file. Firmware update is just "reboot to bootloader".

### Cross-cutting
- **Localisation:** no app is localised, the reference included.
- **Accessibility:** no app does deliberate accessibility work.
- **Room correction:** no app has it, so the row is n.a. everywhere.
- **Stale bundled firmware:** Android ships beta5, while the reference ships 1.1.6 final (the Qt desktop ships 1.1.6 final since the rebuild).

---

## 2. Matrix

### A. Connection and transport

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| A1 | USB vendor transport | ✓ IOKit | n.a. (owner decision: MIDI is the only USB transport on iPhone/iPad) | ✓ the core's `nusb` through `dspi_capi` (no USB code in the app) | ✓ nusb, `doctor`, `--install-udev` | ✓ UsbManager fd into the core (patch 0001); claims only the vendor interface | ✓ WinUSB |
| A2 | USB MIDI (SysEx) | ◐ dev flag `-LocalTransport midi` only *(ours)* | ✓ CoreMIDI, retries, relay port for the Simulator *(ours)* | ✗ | ✗ | ✗ (not needed: vendor works) | ✗ |
| A3 | DSPi Link client | ✓ Bonjour, typed address (lists the hub's devices), PIN pairing, tokens | ✓ Devices sheet, async PIN, local-network-denied state *(ours UI)* | ✓ Devices list, discovery (embedded mDNS on Linux; Windows' own DNS-SD on Windows, so browsing asks nothing of the firewall: DISC2), typed address, PIN pairing, tokens, reconnect with its state and attempt count, lock notice (`LinkController`, `qml/devices/DevicesPopup.qml`) | ✗ (`dspi-link` crate exists, not wired in; no mDNS) | ✓ with deviations: opt-in, NSD, Keystore tokens; viewer role makes the app read-only; no reconnect after 4000/4002 | ✗ |
| A4 | DSPi Link hub / sharing | ✓ Networking page: roles, pairing, revoke, prevent sleep | n.a. (iOS is never a hub) | ✓ Settings > Networking: sharing, name, access, port, pairing, roles, revoke; Windows Firewall and Public-network warnings (`HubController`, `FirewallController`); advertised through Windows' DNS-SD on Windows (DISC2) | ✗ (test fakehub only) | ✗ (owner decision D1/D5) | ✗ |
| A5 | Browser client served by the hub / ESP32 bridge | ✓ serves the web UI; ESP32 setup is external, Console sets the UART interface | ✗ (can be a client of a bridge) | ✗ | ✗ | ✗ (can be a client of a bridge) | ✗ |
| A6 | UART bridge as host transport | n.a. (reached through an ESP32 bridge hub) | n.a. (same) | ✗ | ✗ | ✗ (only via a Link bridge) | ✗ |
| A7 | Demo device | ◐ dev flag only *(ours)* | ✓ always listed; answers the ext channel; `-DemoPairMap k340` *(ours)* | ✓ always listed, demo banner, offered when no DSPi is attached (`DemoController`, `qml/banners/DemoBanner.qml`) | ◐ `--fixture`: render fixture, mock answers zeros | ✓ `FakeDspiCore` from a V32 capture; only while no DSPi is attached | ✗ |
| A8 | Multi-device, hot-plug, reconnect | ✓ | ✓ Devices sheet merges MIDI, demo and hubs; reconnect on foreground | ✓ the core's hot-plug events (`dspi_poll_hotplug`, every 500 ms), unsaved-changes guard on device switch and quit (`UnsavedGuard`) | ◐ picker and switch prompt; **no auto-reconnect** | ◐ USB / Link / Demo are separate stacks with no merged list; auto-launch on plug | ✓ |
| A9 | Wire format V32 | ✓ + per-version/caps gating | ✓ | ✓ the core's (generated header `dspi_core.h`); no wire constants in the app | ✓ (generated from `bofr-main` headers, 247-opcode coverage test) | ✓ | ✗ V26 (floor V16) |
| A10 | Older/newer firmware | ◐ banner and feature hiding, no read-only fallback; `-MIDIReadOnly` diagnostic *(ours)* | ◐ same | ✓ **read-only mode**: banner, every write refused with a notice, reads go on (`ReadOnlyController`, `qml/banners/ReadOnlyBanner.qml`); V17, V26, V31 and V33 fixtures in the smoke | ◐ refuses with a message (by design) | ◐ newer: read-only + banner (double write guard); older V17–V30 **unreadable** on `main` | ◐ legacy per-opcode fetch, no warning UI |
| A11 | Extension channel 0x00, EXT_LIST probe on connect | ✓ once per connect over every transport *(ours)* | ✓ *(ours)* | ✓ the core probes once per connect; the app shows the pair map and the core voltage (`ExtController`) | ◐ on demand via CLI, not on connect | ◐ probe is a pair-map GET; EXT_LIST defined but unused | ✗ |
| A12 | Live sync (notifications, other hosts) | ✓ EP 0x83 v1+v2, echo suppression, Link relay | ✓ MIDI notification frames *(ours)*, Link | ✓ the core's notifications (EP 0x83) with echo suppression; another console's writes show (QP22, Linux and Windows) | ✓ EP 0x83 v2, `uconsole watch` | ✓ USB + Link NOTIFY, 30 ms batching, 0 Hz poll when hidden | ✓ |

### B. DSP features

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| B1 | Master/user volume and modes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B2 | Input source and source names | ✓ | ✓ (long-press) | ✓ source names on Settings > Inputs (held for the save bar) and in the source menu (`qml/main/GlobalControls.qml`) | ✓ | ◐ no source names | ◐ no names |
| B3 | Device name edit | ✓ | ✓ | ✓ Settings > Global Parameters > Device Name (`NamesController`) | ✓ `dev.name` | ✗ (and no Link `device.rename`) | ✗ |
| B4 | Channel names | ✓ | ◐ long-press only | ✓ | ✓ | ✓ | ✓ |
| B5 | PEQ band list | ✓ 10 bands | ✓ | ✓ | ✓ | ✓ | ✓ (12 bands) |
| B6 | PEQ response graph | ✓ Metal, phase | ◐ SwiftUI path (Metal renderer excluded) | ✓ `BodePlot` (QPainter), phase, optional glow | ✓ text graph, phase, zoom | ✓ Canvas (blur only on API 31+) | ✓ Win2D |
| B7 | On-graph PEQ editing | ✓ full (drag, marquee, pull from curve, chip/HUD, menus, keys) | ◐ touch: tap/drag/pinch-Q/double-tap/long-press; no marquee, pull, chip | ✓ drag, marquee, pull from curve, shape chooser, the band bar docked under the graph, menus, keys (`src/render/PeqOverlay.cpp`, `qml/graph/`) | ✗ (by design; `;` command bar) | ✓ touch-mapped (marquee, curve pull, chip, pinch Q); modifiers need a hardware keyboard | ✗ |
| B8 | Crossover and Linkwitz | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B9 | Copy/paste channel | ✓ (Cmd+C/V) | ◐ menu only | ✓ | ✓ (logic sits in the TUI) | ✓ | ✓ |
| B10 | Matrix mixer | ✓ | ✓ (sheet) | ✓ | ✓ | ✓ (phone: one pager tab per output) | ✓ |
| B11 | Output delay/gain/mute/invert | ✓ | ✓ | ✓ | ✓ (+ output latency) | ✓ | ✓ |
| B12 | Output limiter | ✓ | ✓ | ✓ | ✓ | ✓ (gang groups mirrored in Kotlin) | ✗ |
| B13 | Crossfeed (basic) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B14 | Crossfeed pair map (ext 0x00) | ✓ Default / K340 / Custom; in presets, files and diff *(ours)* | ✓ one column on iPhone *(ours)* | ✓ Default / K340 / Custom editor, in presets, files and diff (`qml/tools/CrossfeedPairMap.qml`) | ◐ CLI only (`uconsole ext xfeed-map`) | ✓ (patch 0002; in presets and diff) | ◐ legacy 0xFC/0xFD pair mask |
| B15 | Loudness | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B16 | Volume leveller | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B17 | Psychoacoustic bass | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B18 | Subharmonic synth | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ |
| B19 | Tube modeller | ✓ Basic/Advanced | ✓ | ✓ | ◐ no Basic/Advanced split | ✓ | ✗ |
| B20 | Stereo upmixer | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B21 | Room correction | n.a. (doesn't exist) | n.a. | n.a. | n.a. | n.a. | n.a. |
| B22 | Core voltage (ext 0x0002) | ◐ read-only status on Settings > Overview *(ours)* | ◐ same *(ours)* | ✓ **above the Mac**: status on Overview and Stats, and the Core Voltage page under Settings > Advanced: set, trial, automatic raising, clear, calibration, the typed confirmation word (`CoreVoltageController`, `qml/settings/pages/CoreVoltagePage.qml`) | ◐ CLI only, but **full** read and set (set, trial, auto, clear) | ✗ (constant only) | ✗ |
| B23 | Output latency target (0x03) | ✓ | ✓ | ✓ Settings > Global Parameters > Latency (`OutputLatencyController`) | ✓ `out.latency` | ✗ | ✗ |

### C. Analysis

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| C1 | RTA / spectrum (overlay, bar strip, window, shared over Link) | ✓ | ✓ | ✓ overlay, bar strip, window; shared over Link with "following another console" and Take control (`RtaController`) | ◐ panel only; no overlay, no bar strip | ◐ overlay and window; no bar strip, no "following another console" | ✗ |
| C2 | System stats (clock/MS diagnostics) | ✓ | ✓ | ✓ clock diagnostics when the firmware has them, ring health, watermarks and their reset (`qml/tools/StatsWindow.qml`) | ✓ `diag.clock/usb/buffers` | ✓ | ✓ |
| C3 | Interrupt monitor | ✓ | ✓ | ✓ (+ Copy) | ✓ | ✓ (+ Copy) | ◐ bulk-endpoint debug window |
| C4 | Signal generator | ✓ 15 signals, Space key | ✓ (no Space key) | ✓ | ✓ | ◐ stops when the page leaves the screen | ✓ |
| C5 | CPU meters, level meters, dashboard | ✓ C0/C1 | ✓ | ✓ C0 and C1 bars (`qml/main/Sidebar.qml`) | ✓ | ✓ | ✓ |

### D. Presets and files

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| D1 | Preset slots (save, rename, clear, copy, default) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| D2 | Commit, revert, factory reset | ✓ (+ Edit > Undo, Redo, Cmd-Z / Shift-Cmd-Z: the core's journal, `Core/CoreUndo.swift`, SWMAC1; see section 4) | ✓ (+ Undo and Redo toolbar buttons, SWMAC1) | ✓ (+ undo, redo and History: the core's journal, `HistoryController`, `qml/tools/HistoryWindow.qml`) | ✓ (+ undo journal) | ✓ (+ confirmation tokens) | ✓ |
| D3 | Unsaved-changes marker and diff | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (+ IO undo) |
| D4 | Filter files (REW / AutoEQ / Console) | ✓ | ✗ "Not available yet" | ✓ | ✓ (+ Windows dialect) | ✓ SAF | ✓ |
| D5 | `.dspipreset` config document | ✓ | ✗ "Not available yet" | ✓ | ✓ (`--dry-run`) | ✓ (+ share sheet) | ✗ |
| D6 | AutoEQ (browse, favourites, update DB) | ✓ | ◐ browser only | ✓ browse, favourites, update dialog, rebuild from GitHub, import, Reset to Built-in (`AutoEqController`) | ✓ (update runs `curl`) | ◐ no rebuild from GitHub | ✓ |

### E. Settings and hardware

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| E1 | Pin overview | ✓ | ✓ | ✓ | ✓ | ✓ (tap a pin to open its page) | ✓ |
| E2 | Inputs (S/PDIF, LG, I2S, ADAT) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E3 | Outputs and I2S configuration | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E4 | Global parameters (startup, DAC mute, modes) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E5 | Control interfaces (UART/I2C) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E6 | Control surfaces (bindings, IR learn, display, groups, macros, aux) | ✓ | ✓ (small screens unverified) | ✓ cards, binding editor, IR learn, display pages, groups, macros, aux outputs over `dspi_cs_*` (`CsController`) | ✓ (editing model sits in the TUI) | ◐ **view/run only** | ✓ (display/macros not verified) |
| E7 | Graphing/spectrum settings and app preferences | ✓ | ✓ | ✓ searchable; the core's preferences map (`PrefsMap`) | ✓ (TOML; glow and speed settings have no effect) | ◐ no developer onboarding section (+ light theme) | ✓ |

### F. Firmware and app updates

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| F1 | Firmware update (UF2) | ✓ bundled 1.1.6 final, wizard, downgrade | n.a. (no DriverKit, MIDI only), **but the window still shows and waits forever** | ✓ bundled **1.1.6 final** for RP2040 and RP2350, verify, downgrade note, enter bootloader, export the configuration first, custom `.uf2`; hidden on STM32H723; no download, as on the Mac (`FirmwareController`, `qml/tools/FirmwareWindow.qml`) | ◐ finds the drive; you copy by hand | ✓ PICOBOOT or SAF copy, bundled **beta5** or local `.uf2` | ◐ reboot to bootloader only |
| F2 | Enter bootloader | ✓ | ◐ request works but nothing can flash afterwards | ✓ | ✓ | ✓ | ✓ |
| F3 | App self-update | ✓ GitHub, signed zip | n.a. (App Store) | ✓ opt-in GitHub release feed over https, privacy line, update banner (Settings > Updates, `AppUpdater`); only in the project's own Windows files, AppImage and tar.gz, never in the `.deb`, `.rpm` or AUR package | ✗ | n.a. (Play / sideload) | ✗ |

### G. Platform integration

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| G1 | Menus, menu-bar mode, tray | ✓ menus + menu-bar gateway mode, start at login | ◐ toolbar Devices / Tools / Settings | ✓ in-window menu bar under the OS frame; tray with status, pairing code, share, allow a new client, quick volume and quit; start minimised (`TrayController`). No macOS build: the Swift app is the Mac release | n.a. (Ctrl-P palette instead) | n.a. (NavHost + service notification) | ◐ flyout menu, no tray |
| G2 | Keyboard shortcuts | ✓ about 30 + graph keys | ✗ | ✓ the Mac's plus ours (Ctrl+1–9 presets, Ctrl+S commit, Ctrl+Shift+R revert, Ctrl+M mute, Ctrl+B bypass, undo, redo, History), from one table (`qml/main/MenuTable.qml`, `ShortcutController`), checked by the smoke | ✓ Console mnemonics | ◐ graph editor only | ✓ |
| G3 | Media keys, OS volume, notification controls | ◐ user volume follows the macOS volume (UAC); no media keys | n.a. | ✗ no host-volume code in the app | ◐ shows OS-slider changes | ◐ notification Vol −/+, Mute, Next preset; no MediaSession or volume keys | ◐ UAC1 reflected |
| G4 | Background behaviour | ✓ keeps sharing from the menu bar, prevents sleep | ◐ no background; reconnects on foreground *(ours)* | n.a. (desktop app) | n.a. | ✓ foreground service keeps USB or Link open | n.a. |
| G5 | Onboarding, What's New, coach marks | ✓ | ◐ wizard (firmware step can't work), What's New; no coach marks | ◐ Getting Started + What's New; no coach marks (off in the Mac build) | ✗ (by design) | ✗ | ✗ |
| G6 | Accessibility | ◐ SwiftUI defaults + `.help` | ◐ Dynamic Type capped at xxxLarge, 44 pt targets | ✓ `Accessible.*` on every custom control, a Tab walk through every window in the smoke; Orca pass pending | ◐ `--lite`, ASCII fallback, no screen reader | ◐ thin TalkBack | ✗ |
| G7 | Localisation | ✗ | ✗ | ◐ every string through `qsTr`/`tr`, the `.ts` checked in CI, a pseudo-locale pass in the smoke with a clipping check; English only | ✗ | ✗ | ✗ |
| G8 | Pop-out windows, multi-window, large screens | ✓ tool windows, pop-out graph | ◐ one sheet at a time; iPad split view/slide-over widths | ✓ tool windows and the pop-out graph | ◐ in-terminal graph pop-out | ◐ routes, 600/840 dp breakpoints, no foldable posture handling | ✓ |
| G9 | Host audio-format monitor | ✓ CoreAudio channel count | ✗ (stub) | ✓ the host's channel count drives the strips (`HostAudioFormat`: PipeWire and Windows backends); shown in Stats | ✓ INPUT_FORMAT notifications | ✗ | ◐ channel count only |

### H. Extras not in the reference (or beyond it)

| Extra | Where |
|---|---|
| Scripting / JSON / dry-run / raw opcodes / `uconsole screenshot` | Terminal |
| Undo/redo journal | Terminal (full); Qt desktop (Edit menu, History window); Swift macOS (Edit menu) and iOS (toolbar), on the core's device only (SWMAC1), undoing volume, EQ, crossover, output and matrix edits since SW9; web (Undo and Redo beside the screen tabs, Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z, Ctrl+Y; drags marked; WB8u); Windows (IO only) |
| Event log (P0M: clips, audio link, resets, connections; a hub's alarms) | Web (Events screen, Advanced and Installer: filter by unit, Copy as Text; WB8u) |
| Full core-voltage control (set/trial/auto) | Terminal CLI; Qt desktop (Core Voltage page, with calibration) |
| Strict read-only double write guard; confirmation tokens on flash writes; PICOBOOT flashing without a drive; config share sheet; light theme; auto-launch on plug | Android |
| Searchable settings; Updates page; read-only mode; delay-refusal banner; licences screen; diagnostics bundle (Help > Save Diagnostics; also the web's Events screen, Save Diagnostics… as a download, Advanced and up, WB8u); the demo entry; Linux packaging (AppImage, tar.gz, deb, rpm, AUR) and Windows zip and installer | Qt desktop |
| Bulk-endpoint debug window; 12-band PEQ | Windows C# |

### I. The audio link installation on a hub (Pro layer)

*WP LKUI1, 2026-11-13. Beyond the macOS floor: the hub's system model (SW2, `system.*`, `docs/integrator/audio-link.md`).
The web reads it through its own generic hub call (SW3); Qt and Android through the core's typed calls
`link_system_view`, `link_system_save`, `link_system_align` and `link_system_recall` (dspi-core `link/system.rs`; C ABI
`dspi_link_system_*`; UniFFI), which turn the hub's answers into the rows the screens show.*

| # | Feature | Web | Qt desktop | Android | Swift (macOS, iOS) | Terminal |
|---|---|---|---|---|---|---|
| I1 | System view: every element with its role (Sender, Fan-out, Repeater, Receiver) and link state (Locked, Holdover, Searching, Off, pending link delay), Link delay in frames and ms, Link delay range, Hop offset, phase error | ✓ SW3 (rooms, zones, topology tree, link slots) | ◐ Window > System… (`SystemController`, `qml/system/SystemWindow.qml`): a units table; no rooms-and-zones grouping, topology tree or link slots | ◐ System screen (`feature-home/system/SystemScreen.kt`), touch rows; no rooms-and-zones grouping, topology tree or link slots | **pending** (Mac VM): the binding has the calls (`apps/apple/.../Generated`); no screen yet | ✗ |
| I2 | Clock and links: System delay (automatic or frames), each room's lip sync, each unit's measured latency; Save guarded by the revision it was edited on (stale: Save withdrawn, Reload offered; a stale Save is the hub's `conflict`); write the link delays (asks first) | ✓ SW3 (also the slot matrix, link stats and the trim scan; Installer mode) | ◐ Advanced (the Mode menu stays hidden); no slot matrix, link stats or trim scan | ◐ folded under "Clock and links"; no slot matrix, link stats or trim scan | **pending** | ✗ |
| I3 | Scene bar: recall; a refused recall in words, naming the frame the sender holds; a refusal by role with the hub's words | ✓ SW3 | ✓ | ✓ | **pending** | ✗ |
| I4 | Compact status: "Audio link: 4 of 4 locked", the last scene, the system mute, alarms | ✓ status rail (with the system mute toggle and alarm acknowledge) | ◐ a line under the menu bar (`SystemStatusStrip.qml`); no mute toggle or acknowledge | ◐ Home's chip under the health line (phone) or the graph header (tablet); no mute toggle or acknowledge | **pending** | ✗ |
| I5 | Roles and modes: Viewer gets nothing that writes, Control recalls but does not edit, Admin edits; Simple shows the status only; no PIN, so nothing is locked | ✓ | ✓ (the hub's role from every view; Simple hides the scene bar and editor) | ✓ roles, and G1's read-only state turns the writes off too; Android has no mode switch yet: Home's chip is the front page's view, the System screen is the Advanced layer | **pending** | ✗ |

Tests: dspi-hub `tests/wp_lkui1_system_view.rs` (the core's calls against a real hub with the audio link fakes, Admin,
Control and Viewer); Qt smoke section `lkui1-system` against `uconsole --demo hub --audio-link`; Android
`SystemScreenModelWpLkui1Test`, `SystemScreenWpLkui1Test` (Robolectric) on the fake core's `FakeAudioLinkSystem`.

---

## 3. Gap list per app

Effort: **S** is under a day, **M** is a few days, **L** is a week or more.

The "Close in" column says where the fix belongs:
- **Core** means the shared Rust core now being built in `dspi-unified-console` from the Terminal crates (`dspi-proto` / `dspi-session`, plus `dspi-link`). The fix is written once and serves the Qt desktop, Android and later iOS.
- **UI** means that app's own UI layer.

The core itself still needs an FFI surface (C ABI or UniFFI) over `dspi-session` before any GUI can use it. That is a prerequisite for most "Core" rows, so it is counted once below.

### 3.0 Shared core prerequisites (benefit every GUI app)

| Gap | Effort | Notes |
|---|---|---|
| FFI over `dspi-session` (C ABI for Qt, UniFFI/JNI for Android, Swift later) with a callback/async notification surface | L | There is no FFI today. `dspi-core-linux` is a parallel stack that would be retired (RUST-CORE-CONSOLIDATION). |
| Move the TUI-resident logic that GUIs need into session | M | The control-surface editing model (`cs_model.rs`, about 3.8k lines), the channel clipboard, the bootloader watch, poll scheduling and curve sampling. |
| Read-only decoder for older wire formats (V17–V31) | M | Port Android's `wip/wire-compat` patch 0003 into proto/session. That gives the Qt, Terminal and Android apps the same behaviour. |
| Probe EXT_LIST on connect; put the pair map and core-voltage status in `DeviceState` | S | The ext code exists in `session/ext.rs`; only the connect probe and the state model are missing. |
| `LinkClient` → `Transport` adapter, plus mDNS browse | M | Then Link works in every core-based app. |
| USB MIDI SysEx transport (codec in core, port per OS) | M | Needed if the core is to serve iOS; optional for the others. |
| UF2 validation / PICOBOOT writer in core | M | Android has this in Kotlin. A core version would let the Qt and Terminal apps flash directly. |
| AutoEQ update without shelling out to `curl` | S | Make the HTTP fetch injectable from the host app. |
| Stateful demo device (simulates writes, answers the ext channel) | M | Model it on Swift `FakeTransport` and Android `FakeDspiCore`. |

### 3.1 iOS

| Gap | Effort | Close in |
|---|---|---|
| Filter file and `.dspipreset` import/export (UIDocumentPicker / share sheet) | M | UI. The Swift model code already exists; only the iOS file plumbing is missing. |
| Hide or explain the firmware-update window and wizard step on iOS (n.a. by owner decision) | S | UI |
| AutoEQ Update Database and Favourites | S | UI |
| Touch graph: marquee, pull-from-curve, shape chooser, value chip | M | UI (compile `PeqBandHUD` for iOS or write a touch version) |
| Hardware-keyboard shortcuts (`.commands`, graph keys on iPad) | S–M | UI |
| Host audio-format monitor (AVAudioSession route channel count) | S | UI |
| Core-voltage write controls (if wanted; macOS lacks them too) | S | UI (shared Swift) |
| Hub/sharing | n.a. | By design (never a hub) |

### 3.2 Qt desktop

| Gap | Effort | Close in |
|---|---|---|
| Extension channel: pair-map editor and core-voltage status | M | Done (`ExtController`, `CoreVoltageController`) |
| Input-source names, device name, output latency, clock diagnostics (0x54–0x57, 0x03, 0x82) | S each in UI | Done (`NamesController`, `OutputLatencyController`, `StatsController`) |
| Read-only mode for older firmware instead of a blank app | M | Done (`ReadOnlyController`) |
| DSPi Link client | M | Done (`LinkController`) |
| DSPi Link hub/sharing | L | Done (`HubController`, `FirewallController`) |
| Demo device | S | Done (`DemoController`) |
| Firmware updater: 1.1.6 final images, custom `.uf2` | S | Done (`FirmwareController`); no download, as on the Mac |
| Tray / menu-bar mode (keep running, quick volume) | M | Done, with quick volume (`TrayController`, QP14) |
| Accessibility (`Accessible.name/role` on custom controls) | M | Done; the Orca pass is left |
| Localisation (`qsTr` + `.ts` files) | M | Plumbing done; languages beyond English are left |
| Separate C0/C1 CPU bars | S | Done |
| Hardware validation, especially Windows USB | M | QP22 (Linux and Windows) done; the firmware path and a physical replug on Linux are left |

### 3.3 Terminal

| Gap | Effort | Close in |
|---|---|---|
| TUI screens for the pair map and core voltage, plus probe on connect | S | Core (probe/state) + UI (crossfeed Pairs row, Overview line) |
| Auto-reconnect after unplug/replug | S | Core (session reconnect policy) |
| RTA overlay on the response graph and a bar strip | M | UI |
| Link client | M | Core (adapter) + UI (device list, PIN prompt) |
| Firmware flashing (PICOBOOT or drive copy, bundled/selected `.uf2`) | M | Core |
| Read-only fallback for older firmware (currently refuses by design) | M | Core |
| Tube Basic/Advanced split | S | UI |
| On-graph editing | n.a. | By design |
| Onboarding | n.a. | By design |

### 3.4 Android

| Gap | Effort | Close in |
|---|---|---|
| Merge the older-firmware read-only decoder (`wip/wire-compat`); fix the README claim | M | Core (better in shared session than as patch 0003) |
| Control-surface editor: bindings, IR learn, display pages, group/macro editors | L | Core (move `cs_model` out of the TUI) + UI |
| Core-voltage status (and EXT_LIST probe) | S | Core + UI |
| Device-name edit (and Link `device.rename`) | S | Core already has it in the Terminal crates; UI row |
| Accessibility: TalkBack semantics for the graph, matrix and meters | M | UI |
| Localisation (move strings into `strings.xml`) | M | UI |
| Onboarding / What's New | M | UI |
| RTA bar strip; analyser "following another console" | S–M | UI |
| AutoEQ rebuild from GitHub | S | Core (injectable fetch) |
| Bundle 1.1.6 final firmware | S | Build |
| Host audio-format monitor (AudioManager device channel count) | S | UI |
| Signal generator keeps playing off-screen (matches macOS) | S | UI |
| MediaSession / hardware volume-key link | M | UI (optional) |
| USB MIDI | n.a. / low | Vendor USB works on Android |
| Hub/sharing | n.a. | Owner decision D1/D5 |

### 3.5 Windows C#

No gap list: it is being retired in favour of the Qt desktop. To reach parity it would need V27–V32 (limiter, subharmonic, tube, ext channel), RTA, on-graph editing, config files and a real firmware flasher. Before it is retired, check two things in the Qt app:
- its undo/redo and settings depth match what Windows users have now;
- its crossfeed pair mask handles the legacy 0xFC/0xFD opcodes the same way.

---

## 4. Notes and caveats

- **How cells were judged.** Cells come from code reading. Upstream manuals and READMEs were used only to find features, and stale docs were overruled by code. Known stale docs:
  - Windows `README.md` / `CHANGELOG.md`;
  - Android README on old-firmware read-only.
- **Partly unverified.** Some cells were confirmed by reading files rather than a full grep, because a scratch volume ran out of quota during the survey:
  - iOS: B3, B6, B9, F2, E6 on small screens;
  - Android: B22.
- **Qt core vs Terminal crates** (before the merge). The Qt desktop's `dspi-core-linux` had constants hand-copied at 1.1.6 beta4. The Terminal crates are generated from `bofr-main` `af001ec` (1.1.6 final + ext channel). Moving the desktop onto the Terminal crates was to close several Qt rows at once: B2, B3, B23, C2 and A11/B14/B22 (core side). Done: the merge put the Qt app on the core, and the rebuilt app has every one of those rows.
- **Hub discovery per platform (WP DISC2).** Each client uses its OS's DNS-SD service where there is one: Bonjour (Swift), NsdManager (Android), Windows' DNS Client (`DnsServiceBrowse`/`DnsServiceRegister`) for the Qt app, the Terminal and `uconsole hub` on Windows. Linux keeps the embedded mDNS daemon (mdns-sd). Shared in the core: TXT parsing, address ranking, own-hub filter, resolve by hub id, `verify()`, and a 3 s removal grace (`REMOVAL_GRACE`; Swift and Android wait the same). On Windows:
  - browsing opens no socket, so Windows' firewall asks nothing until sharing starts; the Firewall and Public rows then deal with a Cancel, as before;
  - a hub whose resolve gives only an IPv6 link-local address is listed under its `.local` name, which Windows resolves when dialling (upstream's app and the Mac dial by name too);
  - a TXT change is a goodbye and a new registration, which the grace hides;
  - one difference stays: a hub on a dead adapter stays listed (as with Bonjour and NsdManager) until a dial fails and `verify()` removes it, where Linux's embedded browser drops it within seconds.
- **Swift on the core (WP SWMAC1, 2026-11-13).** The Swift app builds and passes its tests on `main` after RN1 (the kept Swift bundle ids and Xcode names unchanged). Undo and Redo use the core's journal, with slider and graph drags marked by `setDragging`, but only while the device is on the core (behind `-CoreTransport YES`: `CoreBackedTransport`; without it the demo device is Swift's `FakeTransport` and there is no journal), and the Swift domains still send raw SETs, which the core journals as history only (`raw_set`). So Undo stays disabled for the app's own edits until SW9–SW14 move the domains to the typed setters. The AutoEQ requests send the core's User-Agent (`uConsole-for-DSPi/<core version>`). Not in Swift yet: the Pro layer (System view, scene bar, status rail, History window) and the audio-link screens (Clock and links); they follow LKUI1.
- **Swift undo, where it works (WP SW9, 2026-11-13).** Volume (master, user, input preamps), EQ bands and their bypass, crossover bands, the channel delays, EQ bypass, outputs (gain, mute, delay, enable, latency) and the matrix now go through the core's typed setters (the Core 1 mode and the latency table are read through the core's typed reads) whenever a core is attached, so Edit > Undo / Redo (macOS) and the toolbar's Undo / Redo (iOS) undo them. A slider drag and a PEQ graph drag are one step each, also a graph drag whose motion changes kind mid-way (gain only, then frequency and gain: the core's P0W2); a drag cut off by its view leaving the screen (a sheet dismissed, a tool window closed, an iOS gesture cancelled) ends its step there (review lk1 S1). The output enable and the latency are recorded and stepped over (an audible change and a flash write, by the registry), as in Qt. Where: **macOS** with `-CoreTransport YES`, on the core's demo device (`-LocalTransport demo`) and on the USB device through the in-process hub's session 1 (not yet checked on hardware); **iOS** with `-CoreTransport YES`, on the core's demo device. Not yet: without the flag (the default on both: Swift's `FakeTransport` demo and USB), over USB MIDI (SW5) and as a Link client (SW6); there the domain's writes stay raw SETs and Undo stays disabled. The other domains (effects, pins and inputs, control surfaces, presets, analyser; SW10–SW14) still send raw SETs, which the core records as history only. Through a core the master volume moves in the core's 0.5 dB steps, and a band's gain and a crosspoint gain in 0.1 dB, the values Qt and Android show (fixup sw9). A compound edit (a linked input pair's preamp, Clear All, direct routing) is one step per write, as in Qt: the core has no grouping.
- **Reference version.** The macOS reference is `ext-channel-116`. Our additions there (MIDI, demo, ext channel, pair map, core-voltage status, iOS UI) are not in upstream Weeb Labs v1.1.6.

---

## 5. The macOS feature floor (XC0 audit)

Audited 2026-11-06 by reading code; nothing was built or run. The Mac has every row, so it has no column.
`PLAN-CLIENTS.md` section 3 is a view of this table.

**Sources:**

| Client | Where | Path prefix below |
|---|---|---|
| macOS, iOS | `~/.cache/dspi-build/work/parity-ref` (export of `ext-channel-116`), not the stale `_ref/DSPi-Console`. iOS is this tree minus `project.pbxproj:56-79` | `DSPi Console/` |
| Qt | this tree, `main` after the swap (re-read by WP POSTSWAP, 2026-11-13) | `apps/desktop/qt/` |
| Terminal | this tree, `ab95c7f` | `crates/dspi-tui/src/` |
| Android | `~/prog/dspi-mobile/dspi-android` `63492af`, v0.10.1 on its old core (moves to `apps/android` in MR1) | the module path, `feature-home/` and so on; Kotlin package directories are left out, so `feature-home/ContentView.kt` is `feature-home/src/main/kotlin/se/bofr/dspi/feature/home/ContentView.kt` |

**Cells:**

| Mark | Meaning |
|---|---|
| **have** | read in that client's code, and does what the Mac does |
| **partial** | read; the cell says what differs |
| **missing** | looked for and absent |
| **n.a.**, **skip**, **later** | not applicable; decided out (Terminal: no graphical UI); deferred by the owner |
| a WP id | not there yet; that WP adds it |

**Depth.** Rows M41 to M53 and M55 were read for all four clients. In other rows, a cell was re-read only where this
audit names a file. The rest are carried over from the plan's own reading: M1 to M40, M54 and M56.

### 5.1 Floor table

**Link hub** (mobile: owner decided)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M1 | Share devices: server, hub name, open or pairing, port | have: Settings > Networking (`HubController`, `qml/settings/pages/NetworkingPage.qml`) | TM11 | no (Play); sideload later | no |
| M2 | Paired clients: roles, revoke, pairing window | have: the same page | TM11 (CLI) | no | no |
| M4 | Web UI served by the hub | **missing**: the app's hub (the C ABI's `dspi_hub_*`) serves no web pages; `uconsole hub --web` does (HB5) | TM11 | no | no |
| M5 | Bonjour advertisement | have: embedded mDNS on Linux; on Windows Windows' own DNS-SD (DISC2) | TM11; the same | no | no |
| M6 | Menu-bar gateway (`MenuBarMode.swift:104-126`: status, address, pairing code, Open, Share on This Network, Allow a New Client, Quit) | have: the tray menu has the Mac's items (`TrayController`) | n.a.: no GUI; a service | n.a.: service notification | n.a.: no such surface |
| M7 | Open at login, start minimised | have (`SystemIntegration`) | n.a.: a service unit | n.a. | n.a. |
| M8 | Keep the machine awake while sharing | have (`SystemIntegration`; QP22 checked it on Windows) | TM11 | n.a.: no hub | n.a.: no hub |

M3 and M14 (`fw.install`) are deleted: no hub implements them (`PLAN-CLIENTS.md` section 2, row 2).

**Link client**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M9 | Find hubs, add by address | have (`LinkController`, `qml/devices/DevicesPopup.qml`) | CG2, TM9 | have; AN4 re-resolves | have; kept in SW6 |
| M10 | One merged device list | have (`qml/devices/DevicesPopup.qml`) | have (R1) | AN7 | have |
| M11 | Non-blocking PIN prompt, clear errors | have (`qml/devices/PairingDialog.qml`) | have (R1) | AN5 | have; re-pointed in SW6 |
| M12 | Lost-connection banner (owner's rule) | have (`qml/link/LinkBanner.qml`) | TM8 | AN5 | SW7 |
| M13 | Analyser "following another console", "Take control" (`SpectrumAnalyserView.swift:1046`) | have (`RtaController`) | CG5, TM5 | CG5, AN17 | have; re-pointed in SW14 |

**App and firmware**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M15 | App self-update | have: opt-in, Settings > Updates (`AppUpdater`); only in the project's own Windows files, AppImage and tar.gz | n.a.: package manager | n.a.: Play | n.a.: App Store |
| M16 | Firmware wizard: bundled image, verify, downgrade, enter bootloader without installing, export configuration before flashing (`FirmwareUpdateView.swift:384-385`); hidden on STM32H723 (`DSPi_ConsoleApp.swift:11974`) | have (`FirmwareController`, `qml/tools/FirmwareWindow.qml`) | TM3 (non-graphical) | have most; AN18 | n.a.: no USB bootloader (owner); SW18 hides the dead window |
| M41 | Firmware mismatch banner: any version other than the expected one (`FirmwareUpdateView.swift:481-526`, `DSPViewModel.swift:2099-2106`) | **have**: `qml/banners/FirmwareBanner.qml`, `FirmwareController` (expected version: the bundled images', 1.1.6) | TM4 | **partial**: shown only for an incompatible wire format, never for another version on the same format (`feature-home/HomeModel.kt:89-97`, `feature-home/FirmwareMismatchBanner.kt:35-42`). **AN18**, with the updated images | **partial**: shown, but Update opens the dead firmware window (`ContentView.swift:1075-1077`). **SW18** |

**Onboarding** (Terminal: decided, skips)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M17 | Getting Started wizard (`DSPi_ConsoleApp.swift:12054`) | have | skip | AN19 | have; SW18 drops the firmware step |
| M20 | What's New | have | skip | AN19 | have |
| M18, M19 | Coach-mark tour, hints: **off in the shipped Mac build** | later | skip | later | later |

**Main window and presets**

| # | Feature | Qt | Terminal | Android | iOS | Web |
|---|---|---|---|---|---|---|
| M42 | Unsaved-changes guard on preset switch, Copy to, device switch and quit (`DSPi_ConsoleApp.swift:11604`, `ContentView.swift:241,701`, `DSPViewModel.swift:3175,3231`, `PresetSnapshot.swift:692-735`). In this repository, while the device is on the core (`CoreBackedTransport`: the demo device, `-CoreTransport`, the Mac's hub session under the flag) the flag and the lines are the core's (CG7, `presetChanges`, via `DSPViewModel.corePresetChanges`, SWF6), and each prompt first settles an edit sent in the last 100 ms (`settleUnsavedChanges`, SWF7; at most 100 ms on main, not over the hub session, whose echoes come through main, SWF8); `PresetSnapshot` decides for USB without the flag, MIDI and the Link client | **have**: device switch, quit and closing the main window (`UnsavedGuard`); preset switch and Copy to (`SidebarController`, `qml/main/GlobalControls.qml`). The lines are the core's (CG7) | **have**: `live.rs:1555-1566,1763,1869,2151,2263` | **partial**: preset switch and Copy to (`feature-presets/PresetMenuActions.kt:54,85,167`). No user-facing device switch yet, so no guard there (`bridge/DspiBridgeImpl.kt:771,1686,1803,1819` select by themselves). No quit on Android. **AN7** | **have**: `ContentView.swift:250,701-758`, `DSPViewModel.swift:3231-3236`. No quit on iOS. Through the core the lines are CG7's (`presetChanges`, SWF6), settled before each prompt (SWF7); otherwise `PresetSnapshot`'s | **have** (WB6a): preset switch and Copy to (`screens/main/Presets.tsx`); closing a session (Disconnect) asks, as the device switch does; quit is the browser's own leave-page question while something is unsaved (WB6b: one per console, `ui/unsaved.ts`; a host page may take it over, `leavePrompt: false`, `unsaved()`). The lines are the core's (CG7, `presetChanges`). Switching to another open session needs no guard: sessions stay open side by side |
| M43 | Preset slot menu: Save, Rename, Set as Default, Copy to, Clear, Clear All (`ContentView.swift:1546-1566`) | **have**: `qml/main/GlobalControls.qml` (the preset popup) | **have**: `screens/presets.rs:65-110`, `live.rs:1697` | **have**: `feature-home/PresetPickerRow.kt:152-175`, `feature-presets/PresetsScreen.kt:326-364` | **have**: shared view | **have** (WB6a): `screens/main/Presets.tsx`, the core's `preset.*` writes and `presetCopy` |
| M44 | Commit, Revert, Factory Reset (`DSPi_ConsoleApp.swift:11959-11969`) | **have**: the Device menu (`qml/main/MenuTable.qml`; Ctrl+S, Ctrl+Shift+R) | **have**: palette and Ctrl-S, `live.rs:192,1342-1346,2063-2078`, `actions.rs:1346-1373` | **have**: `feature-presets/ToolsMenuActions.kt:21,49,79`, `feature-presets/PresetsScreen.kt:375-377` | **have**: `iOS/IOSApp.swift:215-217`, `DSPi_ConsoleApp.swift:11300-11373` | **have** (WB6a): in the preset menu (`Presets.tsx`), each confirmed as on the Mac; `presetRevert`, `factoryReset` (`crates/dspi-wasm/src/presets.rs`) |
| M45 | Channel clipboard: context menu and Cmd+C/V (`DSPi_ConsoleApp.swift:11706-11731`, `ContentView.swift:480-483,523-526`) | **have**: `qml/main/ChannelRow.qml`, `qml/tools/MatrixWindow.qml`; Ctrl+C/V | **have**: `y` and `Y`, `shell/mod.rs:228,763-764`, `live.rs:1826-1831`, `screens/clipboard.rs` | **have**: menu, `feature-home/ContentView.kt`, `feature-matrix/MatrixMixerView.kt:229`; Ctrl/Meta+C and +V on the selected channel, `app/ConsoleKeyMap.kt` (AN35) | **partial**: menu only; the commands are macOS-only (`DSPi_ConsoleApp.swift:11706`). Keys: SW20 | WB6g (inputs and channels) |
| M46 | Identify output: channel-ID blips, -12 dB, two passes (`ContentView.swift:518`, `Commands.swift:3688-3704`) | **have**: `qml/main/OutputCard.qml`, `qml/tools/MatrixWindow.qml`, `SiggenController` | **partial**: at -20 dB, one pass, walk flag, no clamp to the valid-channel mask (`screens/mod.rs:416-423`; keys `shell/mod.rs:229,767`, `screens/matrix.rs:1170`). **TM18** | **have**: `feature-home/ContentView.kt:341-343`, `feature-matrix/MatrixMixerView.kt:215-219`, `bridge/ControllersImpl.kt:466-471` | **have**: shared view | WB6j (analyser, generator and Stats) |
| M47 | Sidebar volume mode, remembered (`Components.swift:579`, `ContentView.swift:879-886`, `DSPi_ConsoleApp.swift:271`) | **have**: `qml/main/GlobalControls.qml` | **partial**: Enter toggles it, but the choice is never saved; the config value is only read (`shell/mod.rs:856`, `live.rs:653,1734-1740`, `settings/config.rs`). **TM18** | **have**: `feature-home/components/VolumeMode.kt:34-49`, `feature-home/ContentView.kt:421` | **have**: shared view | **have** (WB6a): User or Master volume, kept for this viewer in the page's storage (`screens/main/Volume.tsx`) |
| M48 | L/R link of an input pair, with which side to keep. **The plan's citation, `Components.swift:1520`, is `PreampControlView`, which nothing instantiates.** The live one is `InputChannelHeader`, `Components.swift:1201-1228,1284,1441-1481` | **have**: `qml/main/InputDetail.qml` | **have**: `screens/input.rs:35,393-449` | **have**: `feature-home/components/InputChannelHeader.kt:85-111,165-169` | **have**: shared view | WB6g (inputs and channels) |
| M49 | Linkwitz Transform panel with implied boost (`Components.swift:2357`) | **have**: `qml/main/Linkwitz.qml`, `qml/main/FilterList.qml` | **have**: `screens/linkwitz.rs`, `screens/filters.rs:150,221` | **have**: `feature-home/components/FilterRow.kt`, `feature-home/ChannelDetail.kt:33-34` | **have**: shared view, as a popover | **have** (WB6c): offered in an output's band type menu from wire V22 (not on inputs, as the Console's `includeLinkwitz`); entering it seeds a neutral one (fp = f0, Qp = Q0, the core's `registry::retyped`); the panel (`screens/eq/Linkwitz.tsx`, from the band list) edits f0, Q0, fp and Qp as a draft with Apply and Revert, "Not applied yet" / "Applied", and shows the DC boost (40 log10(f0/fp)) with the Console's warning above +15 dB; writes `eq.freq`, `eq.q`, `eq.fp`, `eq.qp` (`bandWrites`), read back |
| M50 | Bypass Crossovers with warning, Clear All Bands (`Components.swift:1839,1870`) | **have**: `qml/main/FilterList.qml` | **have**: through the `;` bar, no direct keys, same dialogs (`screens/filters.rs:742-775`, `screens/output.rs:428,438`, `screens/input.rs:214,225`) | **have**: `feature-home/components/FilterList.kt:140-184` | **have**: shared view, `PlatformAlert` branch | **have**: Clear All Bands, asked as on the Mac, and Enable All / Bypass All (WB6b, `screens/eq/BandList.tsx`); the XO tab's Bypass All asks "Bypass this output's crossovers?" with the Console's warning, Enable All does not, Clear All asks (WB6c, `screens/outputs/Crossover.tsx`). Global bypass: **have** (WB6a), `Volume.tsx` |
| M51 | Slider-reset and rename gestures (`ContentView.swift:1411,463`) | **have**: right-click reset (`qml/components/CustomSlider.qml`, `qml/main/ChannelValueField.qml`); Alt+click rename (`qml/main/ChannelRow.qml`) | n.a.: keys, no gestures. Backspace resets and `r` renames (`shell/mod.rs:227,237`) | **have**: **double-tap** resets `ui-common/CustomSlider.kt:166-167`; rename from the long-press menu `feature-home/ContentView.kt:273,301,347` | **have**: long-press resets `Components.swift:91-93`; rename from the context menu `ContentView.swift:478,521`, since option-click is an empty stub `Components.swift:95` | **partial** (WB6a): a double or secondary click puts the volume and the trims back at 0 dB; rename by clicking a channel's name (no Option-click on the web). The other sliders come with their slices |
| M52 | Limiter batch actions (`OutputLimiterView.swift:277-285`) | **have**: `qml/tools/LimiterSettings.qml` | **have**: `screens/limiter.rs:44-51,212-230,521-545` | **have**: `feature-tools/OutputLimiterView.kt:183-192` | **have**: shared view | **have** (WB6c): "Copy to all outputs" and the "All outputs" menu (Link all stereo pairs, Unlink all outputs, Switch every limiter off) in the limiter's settings (`screens/outputs/Limiter.tsx`), `limit.*` writes to output 255 or per output; the link groups are the core's |
| M57 | Force a reconnect (`Components.swift:538`: right-click the device picker). **New row** | have: right-click the device name (`qml/main/DeviceStatus.qml`), and Device > Reconnect | have: `live.rs:196,1315` | have: long-press `feature-home/components/ConnectionStatusIndicator.kt:61` | have: long press, shared view | later: a session's reconnect is the core's (Link); no manual one yet |
| M58 | Curve visibility pills (`ContentView.swift:461`, `GraphView.swift:635`). **New row** | have: `qml/main/ChannelRow.qml` (`GraphController.visibleChannels`) | **skip**: depends on the graph, which draws one channel and its partner (`live.rs:822-838`). The coordinator is raising it with the owner | have: `feature-home/components/SidebarRows.kt:55` | have: shared view | **have** (WB6b): a pill per channel beside its chooser on the EQ screen, this viewer's (`screens/eq/EqScreen.tsx`) |
| M59 | Dashboard cards per row (`DashboardView.swift:511-538`). **New row** | have: `qml/main/DashboardLayoutPopup.qml` | have: `settings/graphing.rs:284` | have: `feature-home/DashboardView.kt:259-287` | have: shared view | WB6j (analyser, generator and Stats) |
| M60 | Quick-strip toggles; secondary click opens the tool (`ContentView.swift:575-602`). **New row** | have: `qml/main/QuickStrip.qml` | have: `shell/mod.rs:823` | have: `feature-home/ContentView.kt:401-436`; a long press opens that tool's own page, and Home's top-left menu button opens the Tools list, the one tap-only way to Tube Modeller, Subharmonic and Upmixer (AN31, `ToolsEntryAn31Test`) | have: shared view, long press | WB6l (control surfaces and shortcuts) |
| M61 | Help links, Reset Channel Names (`DSPi_ConsoleApp.swift:12080-12086,2391`). **New row** | have: the Help rows of `qml/main/MenuTable.qml`; Reset Channel Names in `qml/settings/pages/AdvancedPage.qml` | have: `settings/about.rs:16`, `settings/advanced.rs:38-42` | have: `feature-settings/GeneralSettingsTab.kt:30`, `feature-settings/AdvancedSettingsTab.kt:22` | have: shared view | WB6i (settings) |
| M62 | Show Debug Information (`DSPi_ConsoleApp.swift:2409`). **New row** | **have**: "Show Debug Information" in `qml/settings/pages/AdvancedPage.qml` reveals the onboarding developer section | have: `settings/advanced.rs` | have: `feature-settings/AdvancedSettingsTab.kt:28` | have: shared view | WB6i (settings) |

**Graph and analysis** (Terminal: decided, skips on-graph editing; the `;` bar is the equivalent)

| # | Feature | Qt | Terminal | Android | iOS | Web |
|---|---|---|---|---|---|---|
| M21 | HUD chip, marquee, pull from curve, shape menu | have: `src/render/PeqOverlay.cpp`, `qml/graph/` (the band bar docked under the graph, `PeqBandBar.qml`) | skip | have | SW19 | **have** (WB6b): drag (frequency and gain, Q for 2nd-order cuts), Ctrl/Cmd-drag Q, Shift fine, Alt axis lock, Alt-click bypass, marquee, pull from the curve (a shelf near either end), double click adds a bell, Ctrl/Cmd-click shape card, band and graph menus (slope, bypass, invert gain, delete, select all), the chip (shape, slope, bypass, typed values with "2k" and note names; docked under the graph, not floating), the shape card asks a shape's slope ("6 dB" / "12 dB", "180°" / "360°") as the Console's chooser (WB6c), wheel Q and Ctrl-wheel gain; touch: drag, double tap, pinch Q, long press for the menus, Q steppers (`screens/eq/EqGraph.tsx`, `Chip.tsx`); curves the core's (`curves`). Not yet: the hover ghost dot and readouts, the animated band moves |
| M22 | Analyser bar strip | have (`qml/main/SpectrumStrip.qml`) | TM6 | have: AN17 strip, placed by AN34 on Home and channel pages (gear: RTA Bars) | have | WB6j (analyser, generator and Stats) |
| M23 | Analyser overlay on the graph | have (`qml/graph/RtaGraphOverlay.qml`) | TM5 | have | have | WB6j (analyser, generator and Stats) |
| M24 | About 30 shortcuts, Space for the generator, graph keys | have: the Mac's plus ours, Space for the generator (`qml/main/MenuTable.qml`, `ShortcutController`) | have (own mnemonics) | have: `app/ConsoleKeyMap.kt` (AN35; Ctrl or Meta for Command, windows open as routes, Space on the analysis screen only), graph keys `feature-graph/PeqKeyMap.kt` (AN18) | SW20 | **partial** (WB6b): the graph keys (Tab, arrows, Alt-arrows Q, Delete, Escape, Ctrl/Cmd-A, Enter, Shift-F10); the app shortcuts in WB6l |
| M25 | Host audio format (idle channel count, `DSPViewModel.swift:1947-1951`) | have (`HostAudioFormat`: PipeWire and Windows) | skip: the device report collapses when idle, and it has no host API | have: `feature-home/HostChannelCount.kt` (AN35) over AN18's `HostAudioFormat.kt`, refreshed on `AudioDeviceCallback` and handed to the bridge (`setHostInputChannels`), whose `liveInputCount` is `effectiveInputChannels` as macOS `numMatrixInputs`: the strips, the pair links and their mirror use that one count | SW20 | later: a page cannot see the host's audio format |
| M26 | C0 and C1 CPU bars | have (`qml/main/Sidebar.qml`) | have | have | have | WB6f (status rail) |
| M27 | Pop-out tool windows | have: tool windows, the pop-out graph | have (graph) | n.a.: routes | n.a.: sheets | n.a.: a session's screens are tabs in its view |
| M53 | Stats for nerbs, Reset Watermarks (`StatsView.swift:918-1187`), **including Output Ring Health (`:206-223,478-504,1080-1106`) and fill thresholds taken from it (`:1222`)** | **have**: every section with Output Ring Health and its thresholds, and Reset Watermarks (`qml/tools/StatsWindow.qml`, `StatsController`) | **partial**: same (`screens/stats.rs:199-510`, reset `:31,358`). **CG12, TM16** | **partial**: same (`feature-analysis/StatsView.kt:274-654`, reset `:431`). **CG12, AN25** | **have**: shared view | WB6j (analyser, generator and Stats) |
| M54 | Interrupt Monitor (`InterruptMonitor.swift:1001-1010`) | have (`qml/tools/MonitorWindow.qml`) | have | have | have | WB6j (analyser, generator and Stats) |

Web slices: the letters in the Web column are `PLAN-CLIENTS.md` section 5.5's WB6 slice table, whose order puts the System view
(WB6d), the scene bar (WB6e) and the status rail (WB6f, with M26) before the remaining tool screens (UIM1, `docs/UI-MODES.md`).

Web, the B rows of section 2: B5 (band list) and B6 (response graph) **have** (WB6b, `screens/eq/BandList.tsx`, `draw.ts`); B7 is M21.
WB6c: **B8** crossover and Linkwitz **have**: an output's XO tab beside PEQ (family Off / Linkwitz-Riley / Butterworth / Bessel, Low or High Pass, the family's slopes, frequency, bypass; `screens/outputs/Crossover.tsx`), the type through the binding's `crossoverType` (the core's `set_crossover`: the registry's `eq.type` takes PEQ types only), frequency and bypass by `eq.freq` / `eq.bypass` on bands 20..23; the Transform is M49. **B10** matrix mixer **have**: a Matrix screen (`screens/matrix/MatrixScreen.tsx`): live inputs by every output, a crosspoint per cell (connect, level -60..+12 dB, INV) through the binding's `crosspoint` (the registry's `mix`), per output ENABLE (with the Console's PDM / Core 1 questions and switch-over), GAIN, DELAY, MUTE, Direct 1:1 and Clear with more than two inputs, rename from a column's menu; on a phone the grid scrolls inside its card. Not on the web yet: the column menu's Identify (the generator slice, M46) and Copy / Paste Parameters (the channels slice, M45), the per-input trims of the 8-channel matrix (the inputs slice), the upmixer's C / Ls / Rs row names. **B11** output delay, gain, mute and polarity **have**: an output's settings card atop its page as on the Console (`screens/outputs/OutputCard.tsx`): the first two inputs' routing (connect, level, INV: polarity is per crosspoint, as on the Console), GAIN, DELAY (owner rule SL1, `Level.tsx`: a drag sends whole ms and snaps, typed values and the device's are kept and shown to 0.0001 ms, the wheel and the keys go to the next whole ms, keys and assistive changes commit), mute, and the delay's limit is the device's (`outputLimits`: 21 or 42 ms); output enable is in the matrix, as on the Console. **B12** output limiter **have** (wire V32): a toggle on the card and its settings (`Limiter.tsx`): on/off, threshold -30..0 dBFS, release 10..1000 ms, link group Off / 1-4 with the Console's summary, and M52's batch actions.

**Files**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M28 | Filter import with channel pickers, export | have | have; TM10 adds JSON | have | SW17 |
| M29 | `.dspipreset` with apply options and report | have | TM10 | have | SW17 |
| M30 | Save Master Volume, Save Output Configuration | have | **have**: `live.rs:1349-1352`, `actions.rs:1393,1412`. TM10's check is answered | have | have |
| M31 | AutoEQ favourites | have | have | have | SW18 |
| M32 | AutoEQ update, rebuild from GitHub, import, Reset to Built-in | have (`AutoEqController`) | CG6, TM12 | CG6, AN18 | CG6, SW18 |

**Device settings**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M33 | Input-source names | have: Settings > Inputs, and the source menu | **partial**: command only, `in.name` (`crates/dspi-proto/src/registry.rs:2274`); no screen uses it. **TM17** | AN6 | have |
| M34 | Device name (`DSPi_ConsoleApp.swift:1740`) | have: Settings > Global Parameters (`NamesController`) | **partial**: command only, `dev.name` (`registry.rs:2879`); not on Global Parameters. **TM17** | AN6 | have |
| M35 | Output latency (`DSPi_ConsoleApp.swift:1912`) | have: Settings > Global Parameters (`OutputLatencyController`) | **partial**: command only, `out.latency` (`registry.rs:2897`); not on Global Parameters. **TM17** | AN6 | have |
| M36 | Core-voltage status | have: Settings > Overview and Stats | TM2 | AN6 | have |
| M37 | Pair-map editor, K340 preset, in files and diff | have (`qml/tools/CrossfeedPairMap.qml`) | TM1, TM10 | have; CG7 | have |
| M38 | Tube Basic and Advanced | have (`qml/tools/TubeWindow.qml`) | TM7 | have | have |
| M39 | Demo device (dev flag on Mac) | have (`DemoController`) | have (R1) | have | have |
| M40 | USB MIDI (dev flag on Mac) | n.a.: vendor USB works on Linux and Windows; no MIDI in the app | have (R1) | n.a.: vendor USB works | have; SW5 |
| M56 | Core-voltage writes and calibration under Advanced. **Above the floor:** owner's addition; the Mac has status only today | **have** (QP23): the Core Voltage page under Settings > Advanced (`CoreVoltageController`) | have (CLI: set, trial, auto, clear) | AN23 | SW23 (both targets) |
| M55 | Settings: Spectrum, Graphing, Global Parameters (`DSPi_ConsoleApp.swift:1707-2381`) | **have**: Spectrum and Graphing complete; Global has Device Name and Latency | **partial**: Spectrum and Graphing complete; Glow has no terminal meaning (`settings/config.rs:21-31`). Global lacks Device Name and Output Latency. **TM17** | **partial**: Spectrum and Graphing complete. Global lacks Device Name and Output Latency (AN6) | **have**: `iOS/IOSApp.swift:306` wraps the same `SettingsView` |

### 5.2 The Mac's menus and settings pages, walked

**Menus** (`DSPi_ConsoleApp.swift:11838-12092`, `MenuBarMode.swift:104-126`):

| Mac menu | Items | Rows |
|---|---|---|
| App | About; Check for Updates; Settings | M15; section 2 |
| File | Import and Export Filters; Import and Export Device Configuration; Save Master Volume; Save Output Configuration | M28, M29, M30 |
| Edit | Cut, Copy, Paste, Delete, Select All, with the channel clipboard | M45 |
| AutoEQ | Browse Profiles; Favorite Profiles; Clear Favorites; Update Database | M31, M32 |
| Tools | Commit, Revert, Factory Reset; Firmware Update; the eight effect and matrix windows; Signal Generator; Spectrum Analyser; Stats; Interrupt Monitor | M44; M16; section 2 B and C; M53, M54 |
| Window | Minimise to Menu Bar | M6 |
| Help | Getting Started; Replay the Basics Tour (hidden); What's New; two GitHub links | M17, M18, M20, M61 |
| Menu bar item | Status, address, pairing code; Show or Open; Share on This Network; Allow a New Client; Quit | M6 |

**Settings pages** (`SettingsCategory`, `DSPi_ConsoleApp.swift:339-445`):

| Mac page | Qt | Terminal | Android | iOS |
|---|---|---|---|---|
| About | `AboutPage.qml` | `settings/about.rs` | `GeneralSettingsTab.kt` | shared |
| Software Update | `UpdatesPage.qml` ("Updates") | n.a. | n.a.: Play | n.a.: App Store (`:736`) |
| Advanced | `AdvancedPage.qml`, with the Core Voltage page (`CoreVoltagePage.qml`) | `settings/advanced.rs` | `AdvancedSettingsTab.kt` | shared |
| Networking | `NetworkingPage.qml` | TM11 (CLI) | `NetworkingSettingsTab.kt`, client half | n.a.: excluded from the target; the Devices sheet instead (`iOS/IOSApp.swift:380-414`) |
| Graphing | `GraphingPage.qml` | `settings/graphing.rs` | `GraphingSettingsTab.kt` | shared |
| Spectrum Analyser | `SpectrumPage.qml` | `settings/spectrum.rs` | `SpectrumSettingsTab.kt` | shared |
| Overview | `OverviewPage.qml` | `settings/overview.rs` | `PinOverviewTab.kt` | shared |
| Inputs | `InputsPage.qml` | `settings/inputs.rs` | `HardwareSettingsTab.kt` | shared |
| Outputs | `OutputsPage.qml` | `settings/outputs.rs` | `HardwareSettingsTab.kt` | shared |
| I2S Configuration | `I2sPage.qml` | `settings/i2s.rs` | `HardwareSettingsTab.kt` | shared |
| Global Parameters | `GlobalPage.qml` | `settings/global.rs`; M55 | `GlobalSettingsTab.kt`; M55 | shared |
| Control Surfaces | `cs/SurfacesPage.qml` | `settings/surfaces.rs` | `ControlSurfacesSettingsTab.kt`, view and run; AN9–AN11 | shared |
| Control Interfaces | `pages/InterfacesPage.qml` | `settings/interfaces.rs` | `ControlInterfacesSettingsTab.kt` | shared |
| Channel Groups, Macros, Auxiliary Outputs | `cs/GroupsPage.qml`, `cs/MacrosPage.qml`, `cs/AuxPage.qml` | `settings/groups.rs`, `settings/macros.rs`, `settings/surfaces/aux_outputs.rs` | `ControlSurfacesSettingsTab.kt`, view and run (`SettingsScreen.kt:225-227`); AN12 | shared |

Pages were checked for existence, except Global Parameters, Graphing and Spectrum Analyser (M55), whose rows were compared one by one.
Qt pages are under `apps/desktop/qt/qml/settings/`: `pages/` unless named with `cs/`.
Android's one Hardware tab serves Inputs, Outputs and I2S as three pages (`feature-settings/SettingsScreen.kt:220-222`).

**One setting is not a floor item.** `graphAnimationSpeed` has a slider and storage on the Mac and nothing reads it
(`DSPi_ConsoleApp.swift:160,2285-2287` are its only uses), so its absence from Qt needs no WP.

### 5.3 WPs this audit added to the plan

| WP | What | Rows |
|---|---|---|
| CG12 | Output ring health in the session, exported to C and UniFFI | M53 |
| QD29, TM16, AN25 | The ring health section and ring-scaled thresholds in each client | M53 |
| TM17 | Device Name and Output Latency on Global Parameters, source names on Inputs | M33, M34, M35, M55 |
| TM18 | Volume mode saved; Identify at the Mac's level, passes and mask | M46, M47 |

`PLAN-CLIENTS.md` has their scope, and these changes to WPs it already had: AN18 (banner compares versions, M41), AN7 (device
switch guarded, M42), SW18 (banner button and Tools entry, M41), QD12 and QD14 (the Mac's menu-bar items, M6), QD4 and QD5 (Device Name and Output Latency on Global Parameters).

---

## 6. Retired documents

### 6.1 The Qt app's parity plan

`apps/desktop/Documentation/parity-plan.md` was retired into this file by WP HO1a (2026-11-07). It was the Linux
console's own phase plan against the macOS and Windows consoles and firmware 1.1.6 (wire format V32), written on the
old core before the merge. Every phase, 0 to 10, was marked done and most "untested on hardware". To read it:
`git show 2f1e95f3:apps/desktop/Documentation/parity-plan.md`.

Where its content went:

| What | Now |
|---|---|
| Phase status | Section 5 (the Qt column) and the QD track in `docs/core-merge/PLAN-CLIENTS.md` |
| "Not done by choice: separate C0/C1 CPU bars" | Since done by QD5 (row M26) |
| Per-input delay: the firmware has one per channel, inputs included; the plan recorded that neither the macOS nor the Windows console exposed it, so the Qt app did not either | Unchanged in the Qt app; not a floor row in section 5 |
| Notes for users: the PipeWire card profile for system volume, KDE effects on Wayland, Qt logging to the journal | The old app's README Troubleshooting section, gone with the swap; no home in the rebuilt app's docs yet (`apps/desktop/README.md` covers building and the code) |
| Channel ids: the Qt bridge's app ids (0-1 inputs 1-2, 2-10 outputs 1-9, 11-16 inputs 3-8) against the core's wire indices | Gone with the old app's bridge: the rebuilt app uses the core's indices and labels (`dspi_output_labels`) |
| Notes on the old core's bulk image (`bulk_raw`, `encode_bulk`, DAC mute and bulk SET) | Dropped: they describe the old core, which the merge replaced (`docs/core-merge/PLAN.md`) |
