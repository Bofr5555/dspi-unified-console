# DSPi console apps: feature parity

Written 2026-11-06. This is a code-reading survey: nothing was built or run for it.

**Moved here from `dspi-mobile/docs/FEATURE-PARITY.md` by WP XC0.** Sections 1 to 4 are that survey,
written before the core merge. For Qt and the Terminal, `docs/PORTABILITY.md` section 11 says what the merge changed, and
`docs/core-merge/PLAN-CLIENTS.md` section 2 lists further corrections; only B2 and B23 are corrected in place below.
**Section 5 is the macOS floor, audited on this tree: where it disagrees with sections 1 to 4, section 5 wins.**

The reference is the **macOS Swift DSPi Console**: upstream Weeb Labs v1.1.6 (`51fd1bb`) plus our stack, branch `ext-channel-116` (`48cf7c5`). Our stack adds FakeTransport, USB MIDI, the iOS target, the iOS Link client UI and the extension channel. iOS builds from the same sources.

Each other app is compared against that reference:

| App | Source | Version surveyed |
|---|---|---|
| **iOS** | the iOS target of the same Swift project (`ext-channel-116`) | same branch as the reference |
| **Qt desktop** | `dspi-unified-console/apps/desktop` on `crates/dspi-core-linux` | before the merge |
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
Most of the Qt desktop, many Android packages, and the Terminal's control-surface and physical-control paths are in that state.

---

## 1. Summary: biggest gaps per app, ranked by user impact

### iOS (shared Swift sources)
1. **No file I/O.** Filter-file import/export and `.dspipreset` config import/export are present in the menu but say "Not available yet". This blocks REW workflows and backups on iPhone/iPad.
2. **Firmware update is effectively dead.** The window opens but the bootloader locator never finds a board. The update itself is n.a. by owner decision (no DriverKit; USB MIDI is the only USB transport), but the dead window should be hidden.
3. **AutoEQ is partial:** no Update Database and no Favourites menu.
4. **PEQ graph editing is partial:** touch editor without marquee, pull-from-curve, shape chooser or value chip. There are no hardware-keyboard shortcuts.
5. **No hub/sharing.** This is by design (an iOS app is never a hub). Onboarding and the host-format monitor are also reduced.

### Qt desktop (Linux/Windows/macOS)
1. **No extension channel.** So there is no crossfeed pair map (the K340's "one DAC per ear") and no core-voltage status. This is the biggest gap for K340 owners.
2. **No DSPi Link client or hub, no USB MIDI, no demo device.**
3. **Older firmware leaves the app blank, not read-only.** It shows a banner, and every write and every parameter read is skipped.
4. **Firmware updater only flashes the bundled 1.1.6-beta4 images.** That is older than 1.1.6 final, and there is no custom `.uf2` and no download.
5. **Missing settings:** no input-source names and no device-name edit (opcodes 0x54–0x57 are not in `dspi-core-linux`), and no clock/output-latency diagnostics.
6. **Accessibility is done (QD22):** names and roles on every custom control, a keyboard walk through every window; the screen-reader (Orca) pass is still to do. **No localisation** (no `qsTr`), and no tray or menu-bar mode.
7. Almost everything is marked "Done, untested on hardware"; USB has never run on Windows.

### Terminal (TUI/CLI)
1. **No on-graph PEQ editing.** This is out of scope by design (decision 5); the `;` command bar replaces it.
2. **Extension channel is CLI-only.** `dspi ext list | xfeed-map | vreg` work, but there is no TUI screen and no probe on connect.
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
- **Stale bundled firmware:** the Qt desktop ships beta4 and Android beta5, while the reference ships 1.1.6 final.

---

## 2. Matrix

### A. Connection and transport

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| A1 | USB vendor transport | ✓ IOKit | n.a. (owner decision: MIDI is the only USB transport on iPhone/iPad) | ✓ rusb/libusb, WinUSB wIndex fix | ✓ nusb, `doctor`, `--install-udev` | ✓ UsbManager fd into the core (patch 0001); claims only the vendor interface | ✓ WinUSB |
| A2 | USB MIDI (SysEx) | ◐ dev flag `-LocalTransport midi` only *(ours)* | ✓ CoreMIDI, retries, relay port for the Simulator *(ours)* | ✗ | ✗ | ✗ (not needed: vendor works) | ✗ |
| A3 | DSPi Link client | ✓ Bonjour, typed address (lists the hub's devices), PIN pairing, tokens | ✓ Devices sheet, async PIN, local-network-denied state *(ours UI)* | ✗ | ✗ (`dspi-link` crate exists, not wired in; no mDNS) | ✓ with deviations: opt-in, NSD, Keystore tokens; viewer role makes the app read-only; no reconnect after 4000/4002 | ✗ |
| A4 | DSPi Link hub / sharing | ✓ Networking page: roles, pairing, revoke, prevent sleep | n.a. (iOS is never a hub) | ✗ | ✗ (test fakehub only) | ✗ (owner decision D1/D5) | ✗ |
| A5 | Browser client served by the hub / ESP32 bridge | ✓ serves the web UI; ESP32 setup is external, Console sets the UART interface | ✗ (can be a client of a bridge) | ✗ | ✗ | ✗ (can be a client of a bridge) | ✗ |
| A6 | UART bridge as host transport | n.a. (reached through an ESP32 bridge hub) | n.a. (same) | ✗ | ✗ | ✗ (only via a Link bridge) | ✗ |
| A7 | Demo device | ◐ dev flag only *(ours)* | ✓ always listed; answers the ext channel; `-DemoPairMap k340` *(ours)* | ✗ | ◐ `--fixture`: render fixture, mock answers zeros | ✓ `FakeDspiCore` from a V32 capture; only while no DSPi is attached | ✗ |
| A8 | Multi-device, hot-plug, reconnect | ✓ | ✓ Devices sheet merges MIDI, demo and hubs; reconnect on foreground | ✓ 500 ms hot-plug poll, unsaved-changes prompt on switch | ◐ picker and switch prompt; **no auto-reconnect** | ◐ USB / Link / Demo are separate stacks with no merged list; auto-launch on plug | ✓ |
| A9 | Wire format V32 | ✓ + per-version/caps gating | ✓ | ✓ (constants hand-copied at beta4) | ✓ (generated from `bofr-main` headers, 247-opcode coverage test) | ✓ | ✗ V26 (floor V16) |
| A10 | Older/newer firmware | ◐ banner and feature hiding, no read-only fallback; `-MIDIReadOnly` diagnostic *(ours)* | ◐ same | ◐ banner, then blank (no reads, no writes) | ◐ refuses with a message (by design) | ◐ newer: read-only + banner (double write guard); older V17–V30 **unreadable** on `main` | ◐ legacy per-opcode fetch, no warning UI |
| A11 | Extension channel 0x00, EXT_LIST probe on connect | ✓ once per connect over every transport *(ours)* | ✓ *(ours)* | ✗ | ◐ on demand via CLI, not on connect | ◐ probe is a pair-map GET; EXT_LIST defined but unused | ✗ |
| A12 | Live sync (notifications, other hosts) | ✓ EP 0x83 v1+v2, echo suppression, Link relay | ✓ MIDI notification frames *(ours)*, Link | ✓ EP 0x83 (other hosts' EP0 writes are dropped as echoes) | ✓ EP 0x83 v2, `dspi watch` | ✓ USB + Link NOTIFY, 30 ms batching, 0 Hz poll when hidden | ✓ |

### B. DSP features

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| B1 | Master/user volume and modes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B2 | Input source and source names | ✓ | ✓ (long-press) | ◐ no source names | ✓ | ◐ no source names | ◐ no names |
| B3 | Device name edit | ✓ | ✓ | ✗ | ✓ `dev.name` | ✗ (and no Link `device.rename`) | ✗ |
| B4 | Channel names | ✓ | ◐ long-press only | ✓ | ✓ | ✓ | ✓ |
| B5 | PEQ band list | ✓ 10 bands | ✓ | ✓ | ✓ | ✓ | ✓ (12 bands) |
| B6 | PEQ response graph | ✓ Metal, phase | ◐ SwiftUI path (Metal renderer excluded) | ✓ | ✓ text graph, phase, zoom | ✓ Canvas (blur only on API 31+) | ✓ Win2D |
| B7 | On-graph PEQ editing | ✓ full (drag, marquee, pull from curve, chip/HUD, menus, keys) | ◐ touch: tap/drag/pinch-Q/double-tap/long-press; no marquee, pull, chip | ✓ | ✗ (by design; `;` command bar) | ✓ touch-mapped (marquee, curve pull, chip, pinch Q); modifiers need a hardware keyboard | ✗ |
| B8 | Crossover and Linkwitz | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B9 | Copy/paste channel | ✓ (Cmd+C/V) | ◐ menu only | ✓ | ✓ (logic sits in the TUI) | ✓ | ✓ |
| B10 | Matrix mixer | ✓ | ✓ (sheet) | ✓ | ✓ | ✓ (phone: one pager tab per output) | ✓ |
| B11 | Output delay/gain/mute/invert | ✓ | ✓ | ✓ | ✓ (+ output latency) | ✓ | ✓ |
| B12 | Output limiter | ✓ | ✓ | ✓ | ✓ | ✓ (gang groups mirrored in Kotlin) | ✗ |
| B13 | Crossfeed (basic) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B14 | Crossfeed pair map (ext 0x00) | ✓ Default / K340 / Custom; in presets, files and diff *(ours)* | ✓ one column on iPhone *(ours)* | ✗ (fixed-pair mask only) | ◐ CLI only (`dspi ext xfeed-map`) | ✓ (patch 0002; in presets and diff) | ◐ legacy 0xFC/0xFD pair mask |
| B15 | Loudness | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B16 | Volume leveller | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B17 | Psychoacoustic bass | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B18 | Subharmonic synth | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ |
| B19 | Tube modeller | ✓ Basic/Advanced | ✓ | ✓ | ◐ no Basic/Advanced split | ✓ | ✗ |
| B20 | Stereo upmixer | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| B21 | Room correction | n.a. (doesn't exist) | n.a. | n.a. | n.a. | n.a. | n.a. |
| B22 | Core voltage (ext 0x0002) | ◐ read-only status on Settings > Overview *(ours)* | ◐ same *(ours)* | ◐ core mV from the status block only, no ext data | ◐ CLI only, but **full** read and set (set, trial, auto, clear) | ✗ (constant only) | ✗ |
| B23 | Output latency target (0x03) | ✓ | ✓ | ✗ | ✓ `out.latency` | ✗ | ✗ |

### C. Analysis

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| C1 | RTA / spectrum (overlay, bar strip, window, shared over Link) | ✓ | ✓ | ✓ | ◐ panel only; no overlay, no bar strip | ◐ overlay and window; no bar strip, no "following another console" | ✗ |
| C2 | System stats (clock/MS diagnostics) | ✓ | ✓ | ◐ no 0x82 clock diagnostics, no latency | ✓ `diag.clock/usb/buffers` | ✓ | ✓ |
| C3 | Interrupt monitor | ✓ | ✓ | ✓ (+ Copy) | ✓ | ✓ (+ Copy) | ◐ bulk-endpoint debug window |
| C4 | Signal generator | ✓ 15 signals, Space key | ✓ (no Space key) | ✓ | ✓ | ◐ stops when the page leaves the screen | ✓ |
| C5 | CPU meters, level meters, dashboard | ✓ C0/C1 | ✓ | ◐ one CPU bar (deliberate) | ✓ | ✓ | ✓ |

### D. Presets and files

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| D1 | Preset slots (save, rename, clear, copy, default) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| D2 | Commit, revert, factory reset | ✓ | ✓ | ✓ | ✓ (+ undo journal) | ✓ (+ confirmation tokens) | ✓ |
| D3 | Unsaved-changes marker and diff | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (+ IO undo) |
| D4 | Filter files (REW / AutoEQ / Console) | ✓ | ✗ "Not available yet" | ✓ | ✓ (+ Windows dialect) | ✓ SAF | ✓ |
| D5 | `.dspipreset` config document | ✓ | ✗ "Not available yet" | ✓ | ✓ (`--dry-run`) | ✓ (+ share sheet) | ✗ |
| D6 | AutoEQ (browse, favourites, update DB) | ✓ | ◐ browser only | ✓ | ✓ (update runs `curl`) | ◐ no rebuild from GitHub | ✓ |

### E. Settings and hardware

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| E1 | Pin overview | ✓ | ✓ | ✓ | ✓ | ✓ (tap a pin to open its page) | ✓ |
| E2 | Inputs (S/PDIF, LG, I2S, ADAT) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E3 | Outputs and I2S configuration | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E4 | Global parameters (startup, DAC mute, modes) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E5 | Control interfaces (UART/I2C) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| E6 | Control surfaces (bindings, IR learn, display, groups, macros, aux) | ✓ | ✓ (small screens unverified) | ✓ | ✓ (editing model sits in the TUI) | ◐ **view/run only** | ✓ (display/macros not verified) |
| E7 | Graphing/spectrum settings and app preferences | ✓ | ✓ | ✓ (searchable) | ✓ (TOML; glow and speed settings have no effect) | ◐ no developer onboarding section (+ light theme) | ✓ |

### F. Firmware and app updates

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| F1 | Firmware update (UF2) | ✓ bundled 1.1.6 final, wizard, downgrade | n.a. (no DriverKit, MIDI only), **but the window still shows and waits forever** | ◐ bundled **beta4** only, no custom file | ◐ finds the drive; you copy by hand | ✓ PICOBOOT or SAF copy, bundled **beta5** or local `.uf2` | ◐ reboot to bootloader only |
| F2 | Enter bootloader | ✓ | ◐ request works but nothing can flash afterwards | ✓ | ✓ | ✓ | ✓ |
| F3 | App self-update | ✓ GitHub, signed zip | n.a. (App Store) | ✗ | ✗ | n.a. (Play / sideload) | ✗ |

### G. Platform integration

| # | Feature | macOS (ref) | iOS | Qt desktop | Terminal | Android | Windows C# |
|---|---|---|---|---|---|---|---|
| G1 | Menus, menu-bar mode, tray | ✓ menus + menu-bar gateway mode, start at login | ◐ toolbar Devices / Tools / Settings | ◐ in-window menu, native menu on macOS; **no tray** | n.a. (Ctrl-P palette instead) | n.a. (NavHost + service notification) | ◐ flyout menu, no tray |
| G2 | Keyboard shortcuts | ✓ about 30 + graph keys | ✗ | ✓ about 18 | ✓ Console mnemonics | ◐ graph editor only | ✓ |
| G3 | Media keys, OS volume, notification controls | ◐ user volume follows the macOS volume (UAC); no media keys | n.a. | ◐ OS volume via UAC1 | ◐ shows OS-slider changes | ◐ notification Vol −/+, Mute, Next preset; no MediaSession or volume keys | ◐ UAC1 reflected |
| G4 | Background behaviour | ✓ keeps sharing from the menu bar, prevents sleep | ◐ no background; reconnects on foreground *(ours)* | n.a. (desktop app) | n.a. | ✓ foreground service keeps USB or Link open | n.a. |
| G5 | Onboarding, What's New, coach marks | ✓ | ◐ wizard (firmware step can't work), What's New; no coach marks | ◐ Getting Started + What's New; no coach marks | ✗ (by design) | ✗ | ✗ |
| G6 | Accessibility | ◐ SwiftUI defaults + `.help` | ◐ Dynamic Type capped at xxxLarge, 44 pt targets | ✓ `Accessible.*` on every custom control, Tab through every window (QD22); Orca pass pending | ◐ `--lite`, ASCII fallback, no screen reader | ◐ thin TalkBack | ✗ |
| G7 | Localisation | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| G8 | Pop-out windows, multi-window, large screens | ✓ tool windows, pop-out graph | ◐ one sheet at a time; iPad split view/slide-over widths | ✓ | ◐ in-terminal graph pop-out | ◐ routes, 600/840 dp breakpoints, no foldable posture handling | ✓ |
| G9 | Host audio-format monitor | ✓ CoreAudio channel count | ✗ (stub) | ◐ shown in Stats only | ✓ INPUT_FORMAT notifications | ✗ | ◐ channel count only |

### H. Extras not in the reference (or beyond it)

| Extra | Where |
|---|---|
| Scripting / JSON / dry-run / raw opcodes / `dspi screenshot` | Terminal |
| Undo/redo journal | Terminal (full); Windows (IO only) |
| Full core-voltage control (set/trial/auto) | Terminal CLI only |
| Strict read-only double write guard; confirmation tokens on flash writes; PICOBOOT flashing without a drive; config share sheet; light theme; auto-launch on plug | Android |
| Searchable settings; per-OS window chrome; Linux packaging (AppImage, deb, rpm, AUR) | Qt desktop |
| Bulk-endpoint debug window; 12-band PEQ | Windows C# |

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
| Extension channel: pair-map editor and core-voltage status | M | Core (move to `dspi-session`) + UI (crossfeed Pairs row, Overview status) |
| Input-source names, device name, output latency, clock diagnostics (0x54–0x57, 0x03, 0x82) | S each in UI | Core: already in the Terminal crates; arrives with the core switch |
| Read-only mode for older firmware instead of a blank app | M | Core + small UI |
| DSPi Link client | M | Core (`dspi-link` adapter) + UI (devices list, pairing prompt) |
| DSPi Link hub/sharing | L | Core (server side doesn't exist anywhere in Rust) + UI |
| Demo device | S | Core |
| Firmware updater: 1.1.6 final images, custom `.uf2`, optional download | S | UI (bundle) / Core (writer) |
| Tray / menu-bar mode (keep running, quick volume) | M | UI (`QSystemTrayIcon`) |
| Accessibility (`Accessible.name/role` on custom controls) | M | UI: done in QD22; the Orca pass is left |
| Localisation (`qsTr` + `.ts` files) | M | UI |
| Separate C0/C1 CPU bars | S | UI (declined on purpose; optional) |
| Hardware validation, especially Windows USB | M | Testing |

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
  - Qt: whether Stats shows core voltage;
  - Android: B22.
- **Qt core vs Terminal crates.** The Qt desktop's `dspi-core-linux` has constants hand-copied at 1.1.6 beta4. The Terminal crates are generated from `bofr-main` `af001ec` (1.1.6 final + ext channel). Moving the desktop onto the Terminal crates closes several Qt rows at once: B2, B3, B23, C2 and A11/B14/B22 (core side).
- **Reference version.** The macOS reference is `ext-channel-116`. Our additions there (MIDI, demo, ext channel, pair map, core-voltage status, iOS UI) are not in upstream Weeb Labs v1.1.6.

---

## 5. The macOS feature floor (XC0 audit)

Audited 2026-11-06 by reading code; nothing was built or run. The Mac has every row, so it has no column.
`PLAN-CLIENTS.md` section 3 is a view of this table.

**Sources:**

| Client | Where | Path prefix below |
|---|---|---|
| macOS, iOS | `~/.cache/dspi-build/work/parity-ref` (export of `ext-channel-116`), not the stale `_ref/DSPi-Console`. iOS is this tree minus `project.pbxproj:56-79` | `DSPi Console/` |
| Qt | this tree, `ab95c7f` | `apps/desktop/qt/` |
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
| M1 | Share devices: server, hub name, open or pairing, port | HB1–3, HB5, QD14 | TM11 | no (Play); sideload later | no |
| M2 | Paired clients: roles, revoke, pairing window | HB1, QD14 | TM11 (CLI) | no | no |
| M4 | Web UI served by the hub | HB5 | TM11 | no | no |
| M5 | Bonjour advertisement | HB5 | TM11 | no | no |
| M6 | Menu-bar gateway (`MenuBarMode.swift:104-126`: status, address, pairing code, Open, Share on This Network, Allow a New Client, Quit) | QD12 (tray) | n.a.: no GUI; a service | n.a.: service notification | n.a.: no such surface |
| M7 | Open at login, start minimised | QD13 | n.a.: a service unit | n.a. | n.a. |
| M8 | Keep the machine awake while sharing | QD13 | TM11 | n.a.: no hub | n.a.: no hub |

M3 and M14 (`fw.install`) are deleted: no hub implements them (`PLAN-CLIENTS.md` section 2, row 2).

**Link client**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M9 | Find hubs, add by address | CG2, QD9 | CG2, TM9 | have; AN4 re-resolves | have; kept in SW6 |
| M10 | One merged device list | QD9 | have (R1) | AN7 | have |
| M11 | Non-blocking PIN prompt, clear errors | QD9 | have (R1) | AN5 | have; re-pointed in SW6 |
| M12 | Lost-connection banner (owner's rule) | QD10 | TM8 | AN5 | SW7 |
| M13 | Analyser "following another console", "Take control" (`SpectrumAnalyserView.swift:1046`) | CG5, QD19 | CG5, TM5 | CG5, AN17 | have; re-pointed in SW14 |

**App and firmware**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M15 | App self-update | QD16 | n.a.: package manager | n.a.: Play | n.a.: App Store |
| M16 | Firmware wizard: bundled image, verify, downgrade, enter bootloader without installing, export configuration before flashing (`FirmwareUpdateView.swift:384-385`); hidden on STM32H723 (`DSPi_ConsoleApp.swift:11974`) | QD11 | TM3 (non-graphical) | have most; AN18 | n.a.: no USB bootloader (owner); SW18 hides the dead window |
| M41 | Firmware mismatch banner: any version other than the expected one (`FirmwareUpdateView.swift:481-526`, `DSPViewModel.swift:2099-2106`) | **have**: `qml/main.qml:422-487`, `src/FirmwareUpdater.cpp:128`. Expected version stays beta4 until QD11 | TM4 | **partial**: shown only for an incompatible wire format, never for another version on the same format (`feature-home/HomeModel.kt:89-97`, `feature-home/FirmwareMismatchBanner.kt:35-42`). **AN18**, with the updated images | **partial**: shown, but Update opens the dead firmware window (`ContentView.swift:1075-1077`). **SW18** |

**Onboarding** (Terminal: decided, skips)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M17 | Getting Started wizard (`DSPi_ConsoleApp.swift:12054`) | have | skip | AN19 | have; SW18 drops the firmware step |
| M20 | What's New | have | skip | AN19 | have |
| M18, M19 | Coach-mark tour, hints: **off in the shipped Mac build** | later | skip | later | later |

**Main window and presets**

| # | Feature | Qt | Terminal | Android | iOS | Web |
|---|---|---|---|---|---|---|
| M42 | Unsaved-changes guard on preset switch, Copy to, device switch and quit (`DSPi_ConsoleApp.swift:11604`, `ContentView.swift:241,701`, `DSPViewModel.swift:3175,3231`, `PresetSnapshot.swift:692-735`). In this repository, while the device is on the core (`CoreBackedTransport`: the demo device, `-CoreTransport`, the Mac's hub session under the flag) the flag and the lines are the core's (CG7, `presetChanges`, via `DSPViewModel.corePresetChanges`, SWF6), and each prompt first settles an edit sent in the last 100 ms (`settleUnsavedChanges`, SWF7; at most 100 ms on main, not over the hub session, whose echoes come through main, SWF8); `PresetSnapshot` decides for USB without the flag, MIDI and the Link client | **have**: `qml/main.qml:869,892,901,924,950`, `qml/components/PresetMenu.qml:99`. Diff wording: CG7 | **have**: `live.rs:1555-1566,1763,1869,2151,2263` | **partial**: preset switch and Copy to (`feature-presets/PresetMenuActions.kt:54,85,167`). No user-facing device switch yet, so no guard there (`bridge/DspiBridgeImpl.kt:771,1686,1803,1819` select by themselves). No quit on Android. **AN7** | **have**: `ContentView.swift:250,701-758`, `DSPViewModel.swift:3231-3236`. No quit on iOS. Through the core the lines are CG7's (`presetChanges`, SWF6), settled before each prompt (SWF7); otherwise `PresetSnapshot`'s | **have** (WB6a): preset switch and Copy to (`screens/main/Presets.tsx`); closing a session (Disconnect) asks, as the device switch does; quit is the browser's own leave-page question while something is unsaved (WB6b: one per console, `ui/unsaved.ts`; a host page may take it over, `leavePrompt: false`, `unsaved()`). The lines are the core's (CG7, `presetChanges`). Switching to another open session needs no guard: sessions stay open side by side |
| M43 | Preset slot menu: Save, Rename, Set as Default, Copy to, Clear, Clear All (`ContentView.swift:1546-1566`) | **have**: `qml/components/PresetMenu.qml:33-47` | **have**: `screens/presets.rs:65-110`, `live.rs:1697` | **have**: `feature-home/PresetPickerRow.kt:152-175`, `feature-presets/PresetsScreen.kt:326-364` | **have**: shared view | **have** (WB6a): `screens/main/Presets.tsx`, the core's `preset.*` writes and `presetCopy` |
| M44 | Commit, Revert, Factory Reset (`DSPi_ConsoleApp.swift:11959-11969`) | **have**: `qml/components/AppMenu.qml:60-68` (a Device menu, not Tools), `qml/main.qml:1012-1051` | **have**: palette and Ctrl-S, `live.rs:192,1342-1346,2063-2078`, `actions.rs:1346-1373` | **have**: `feature-presets/ToolsMenuActions.kt:21,49,79`, `feature-presets/PresetsScreen.kt:375-377` | **have**: `iOS/IOSApp.swift:215-217`, `DSPi_ConsoleApp.swift:11300-11373` | **have** (WB6a): in the preset menu (`Presets.tsx`), each confirmed as on the Mac; `presetRevert`, `factoryReset` (`crates/dspi-wasm/src/presets.rs`) |
| M45 | Channel clipboard: context menu and Cmd+C/V (`DSPi_ConsoleApp.swift:11706-11731`, `ContentView.swift:480-483,523-526`) | **have**: `qml/components/ChannelRow.qml:145-148,165-166`, `qml/main.qml:287-296`, `qml/MatrixMixerWindow.qml:196-197` | **have**: `y` and `Y`, `shell/mod.rs:228,763-764`, `live.rs:1826-1831`, `screens/clipboard.rs` | **have**: menu, `feature-home/ContentView.kt`, `feature-matrix/MatrixMixerView.kt:229`; Ctrl/Meta+C and +V on the selected channel, `app/ConsoleKeyMap.kt` (AN35) | **partial**: menu only; the commands are macOS-only (`DSPi_ConsoleApp.swift:11706`). Keys: SW20 | WB6 (channels slice) |
| M46 | Identify output: channel-ID blips, -12 dB, two passes (`ContentView.swift:518`, `Commands.swift:3688-3704`) | **have**: `qml/components/ChannelRow.qml:154,167`, `qml/MatrixMixerWindow.qml:202,221`, `src/SiggenController.cpp:281-289` | **partial**: at -20 dB, one pass, walk flag, no clamp to the valid-channel mask (`screens/mod.rs:416-423`; keys `shell/mod.rs:229,767`, `screens/matrix.rs:1170`). **TM18** | **have**: `feature-home/ContentView.kt:341-343`, `feature-matrix/MatrixMixerView.kt:215-219`, `bridge/ControllersImpl.kt:466-471` | **have**: shared view | WB6 (analyser and generator slice) |
| M47 | Sidebar volume mode, remembered (`Components.swift:579`, `ContentView.swift:879-886`, `DSPi_ConsoleApp.swift:271`) | **have**: `qml/Sidebar.qml:15,329-373,402-405` | **partial**: Enter toggles it, but the choice is never saved; the config value is only read (`shell/mod.rs:856`, `live.rs:653,1734-1740`, `settings/config.rs`). **TM18** | **have**: `feature-home/components/VolumeMode.kt:34-49`, `feature-home/ContentView.kt:421` | **have**: shared view | **have** (WB6a): User or Master volume, kept for this viewer in the page's storage (`screens/main/Volume.tsx`) |
| M48 | L/R link of an input pair, with which side to keep. **The plan's citation, `Components.swift:1520`, is `PreampControlView`, which nothing instantiates.** The live one is `InputChannelHeader`, `Components.swift:1201-1228,1284,1441-1481` | **have**: `qml/components/InputHeaderCard.qml:106-118,201-215` | **have**: `screens/input.rs:35,393-449` | **have**: `feature-home/components/InputChannelHeader.kt:85-111,165-169` | **have**: shared view | WB6 (inputs slice) |
| M49 | Linkwitz Transform panel with implied boost (`Components.swift:2357`) | **have**: `qml/components/FilterRow.qml:157-180`, `qml/components/LinkwitzEditor.qml` | **have**: `screens/linkwitz.rs`, `screens/filters.rs:150,221` | **have**: `feature-home/components/FilterRow.kt`, `feature-home/ChannelDetail.kt:33-34` | **have**: shared view, as a popover | **have** (WB6c): offered in an output's band type menu from wire V22 (not on inputs, as the Console's `includeLinkwitz`); entering it seeds a neutral one (fp = f0, Qp = Q0, the core's `registry::retyped`); the panel (`screens/eq/Linkwitz.tsx`, from the band list) edits f0, Q0, fp and Qp as a draft with Apply and Revert, "Not applied yet" / "Applied", and shows the DC boost (40 log10(f0/fp)) with the Console's warning above +15 dB; writes `eq.freq`, `eq.q`, `eq.fp`, `eq.qp` (`bandWrites`), read back |
| M50 | Bypass Crossovers with warning, Clear All Bands (`Components.swift:1839,1870`) | **have**: `qml/FilterListView.qml:202-207,219,396-417` | **have**: through the `;` bar, no direct keys, same dialogs (`screens/filters.rs:742-775`, `screens/output.rs:428,438`, `screens/input.rs:214,225`) | **have**: `feature-home/components/FilterList.kt:140-184` | **have**: shared view, `PlatformAlert` branch | **have**: Clear All Bands, asked as on the Mac, and Enable All / Bypass All (WB6b, `screens/eq/BandList.tsx`); the XO tab's Bypass All asks "Bypass this output's crossovers?" with the Console's warning, Enable All does not, Clear All asks (WB6c, `screens/outputs/Crossover.tsx`). Global bypass: **have** (WB6a), `Volume.tsx` |
| M51 | Slider-reset and rename gestures (`ContentView.swift:1411,463`) | **have**: right-click reset `qml/Sidebar.qml:459-462`, `qml/components/InputHeaderCard.qml:157`, `qml/ChannelSettingsView.qml:50-59,190-195`, `qml/components/ParamRow.qml:89-94`; Alt+click rename `qml/components/ChannelRow.qml:56` | n.a.: keys, no gestures. Backspace resets and `r` renames (`shell/mod.rs:227,237`) | **have**: **double-tap** resets `ui-common/CustomSlider.kt:166-167`; rename from the long-press menu `feature-home/ContentView.kt:273,301,347` | **have**: long-press resets `Components.swift:91-93`; rename from the context menu `ContentView.swift:478,521`, since option-click is an empty stub `Components.swift:95` | **partial** (WB6a): a double or secondary click puts the volume and the trims back at 0 dB; rename by clicking a channel's name (no Option-click on the web). The other sliders come with their slices |
| M52 | Limiter batch actions (`OutputLimiterView.swift:277-285`) | **have**: `qml/components/LimiterButton.qml:282,300-303` | **have**: `screens/limiter.rs:44-51,212-230,521-545` | **have**: `feature-tools/OutputLimiterView.kt:183-192` | **have**: shared view | **have** (WB6c): "Copy to all outputs" and the "All outputs" menu (Link all stereo pairs, Unlink all outputs, Switch every limiter off) in the limiter's settings (`screens/outputs/Limiter.tsx`), `limit.*` writes to output 255 or per output; the link groups are the core's |
| M57 | Force a reconnect (`Components.swift:538`: right-click the device picker). **New row** | have: right-click `qml/components/CpuSection.qml:127` | have: `live.rs:196,1315` | have: long-press `feature-home/components/ConnectionStatusIndicator.kt:61` | have: long press, shared view | later: a session's reconnect is the core's (Link); no manual one yet |
| M58 | Curve visibility pills (`ContentView.swift:461`, `GraphView.swift:635`). **New row** | have: `qml/components/ChannelRow.qml:24` | **skip**: depends on the graph, which draws one channel and its partner (`live.rs:822-838`). The coordinator is raising it with the owner | have: `feature-home/components/SidebarRows.kt:55` | have: shared view | **have** (WB6b): a pill per channel beside its chooser on the EQ screen, this viewer's (`screens/eq/EqScreen.tsx`) |
| M59 | Dashboard cards per row (`DashboardView.swift:511-538`). **New row** | have: `qml/components/DashboardLayoutPopup.qml` | have: `settings/graphing.rs:284` | have: `feature-home/DashboardView.kt:259-287` | have: shared view | WB6 (analyser slice) |
| M60 | Quick-strip toggles; secondary click opens the tool (`ContentView.swift:575-602`). **New row** | have: `qml/Sidebar.qml:190-211` | have: `shell/mod.rs:823` | have: `feature-home/ContentView.kt:401-436`; a long press opens that tool's own page, and Home's top-left menu button opens the Tools list, the one tap-only way to Tube Modeller, Subharmonic and Upmixer (AN31, `ToolsEntryAn31Test`) | have: shared view, long press | WB6 (later slice) |
| M61 | Help links, Reset Channel Names (`DSPi_ConsoleApp.swift:12080-12086,2391`). **New row** | have: `qml/components/AppMenu.qml:95-101`, `qml/settings/pages/AdvancedPage.qml:23` | have: `settings/about.rs:16`, `settings/advanced.rs:38-42` | have: `feature-settings/GeneralSettingsTab.kt:30`, `feature-settings/AdvancedSettingsTab.kt:22` | have: shared view | WB6 (settings slice) |
| M62 | Show Debug Information (`DSPi_ConsoleApp.swift:2409`). **New row** | **missing**. No WP: on the Mac it only reveals the onboarding developer section (`:2427`), and coach marks are later | have: `settings/advanced.rs` | have: `feature-settings/AdvancedSettingsTab.kt:28` | have: shared view | WB6 (settings slice) |

**Graph and analysis** (Terminal: decided, skips on-graph editing; the `;` bar is the equivalent)

| # | Feature | Qt | Terminal | Android | iOS | Web |
|---|---|---|---|---|---|---|
| M21 | HUD chip, marquee, pull from curve, shape menu | have | skip | have | SW19 | **have** (WB6b): drag (frequency and gain, Q for 2nd-order cuts), Ctrl/Cmd-drag Q, Shift fine, Alt axis lock, Alt-click bypass, marquee, pull from the curve (a shelf near either end), double click adds a bell, Ctrl/Cmd-click shape card, band and graph menus (slope, bypass, invert gain, delete, select all), the chip (shape, slope, bypass, typed values with "2k" and note names; docked under the graph, not floating), the shape card asks a shape's slope ("6 dB" / "12 dB", "180°" / "360°") as the Console's chooser (WB6c), wheel Q and Ctrl-wheel gain; touch: drag, double tap, pinch Q, long press for the menus, Q steppers (`screens/eq/EqGraph.tsx`, `Chip.tsx`); curves the core's (`curves`). Not yet: the hover ghost dot and readouts, the animated band moves |
| M22 | Analyser bar strip | have | TM6 | have: AN17 strip, placed by AN34 on Home and channel pages (gear: RTA Bars) | have | WB6 (analyser slice) |
| M23 | Analyser overlay on the graph | have | TM5 | have | have | WB6 (analyser slice) |
| M24 | About 30 shortcuts, Space for the generator, graph keys | QD17 | have (own mnemonics) | have: `app/ConsoleKeyMap.kt` (AN35; Ctrl or Meta for Command, windows open as routes, Space on the analysis screen only), graph keys `feature-graph/PeqKeyMap.kt` (AN18) | SW20 | **partial** (WB6b): the graph keys (Tab, arrows, Alt-arrows Q, Delete, Escape, Ctrl/Cmd-A, Enter, Shift-F10); the app shortcuts in a later slice |
| M25 | Host audio format (idle channel count, `DSPViewModel.swift:1947-1951`) | QD18 | skip: the device report collapses when idle, and it has no host API | have: `feature-home/HostChannelCount.kt` (AN35) over AN18's `HostAudioFormat.kt`, refreshed on `AudioDeviceCallback` and handed to the bridge (`setHostInputChannels`), whose `liveInputCount` is `effectiveInputChannels` as macOS `numMatrixInputs`: the strips, the pair links and their mirror use that one count | SW20 | later: a page cannot see the host's audio format |
| M26 | C0 and C1 CPU bars | QD5 | have | have | have | WB6 (analyser slice) |
| M27 | Pop-out tool windows | have | have (graph) | n.a.: routes | n.a.: sheets | n.a.: a session's screens are tabs in its view |
| M53 | Stats for Nerds, Reset Watermarks (`StatsView.swift:918-1187`), **including Output Ring Health (`:206-223,478-504,1080-1106`) and fill thresholds taken from it (`:1222`)** | **partial**: every other section, and the reset (`qml/StatsWindow.qml:231-500,331-332`). No ring health. **CG12, QD29** | **partial**: same (`screens/stats.rs:199-510`, reset `:31,358`). **CG12, TM16** | **partial**: same (`feature-analysis/StatsView.kt:274-654`, reset `:431`). **CG12, AN25** | **have**: shared view | WB6 (analyser slice) |
| M54 | Interrupt Monitor (`InterruptMonitor.swift:1001-1010`) | have | have | have | have | WB6 (a later slice) |

Web, the B rows of section 2: B5 (band list) and B6 (response graph) **have** (WB6b, `screens/eq/BandList.tsx`, `draw.ts`); B7 is M21.
WB6c: **B8** crossover and Linkwitz **have**: an output's XO tab beside PEQ (family Off / Linkwitz-Riley / Butterworth / Bessel, Low or High Pass, the family's slopes, frequency, bypass; `screens/outputs/Crossover.tsx`), the type through the binding's `crossoverType` (the core's `set_crossover`: the registry's `eq.type` takes PEQ types only), frequency and bypass by `eq.freq` / `eq.bypass` on bands 20..23; the Transform is M49. **B10** matrix mixer **have**: a Matrix screen (`screens/matrix/MatrixScreen.tsx`): live inputs by every output, a crosspoint per cell (connect, level -60..+12 dB, INV) through the binding's `crosspoint` (the registry's `mix`), per output ENABLE (with the Console's PDM / Core 1 questions and switch-over), GAIN, DELAY, MUTE, Direct 1:1 and Clear with more than two inputs, rename from a column's menu; on a phone the grid scrolls inside its card. Not on the web yet: the column menu's Identify (the generator slice, M46) and Copy / Paste Parameters (the channels slice, M45), the per-input trims of the 8-channel matrix (the inputs slice), the upmixer's C / Ls / Rs row names. **B11** output delay, gain, mute and polarity **have**: an output's settings card atop its page as on the Console (`screens/outputs/OutputCard.tsx`): the first two inputs' routing (connect, level, INV: polarity is per crosspoint, as on the Console), GAIN, DELAY (owner rule SL1, `Level.tsx`: a drag sends whole ms and snaps, typed values and the device's are kept and shown to 0.0001 ms, the wheel and the keys go to the next whole ms, keys and assistive changes commit), mute, and the delay's limit is the device's (`outputLimits`: 21 or 42 ms); output enable is in the matrix, as on the Console. **B12** output limiter **have** (wire V32): a toggle on the card and its settings (`Limiter.tsx`): on/off, threshold -30..0 dBFS, release 10..1000 ms, link group Off / 1-4 with the Console's summary, and M52's batch actions.

**Files**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M28 | Filter import with channel pickers, export | have | have; TM10 adds JSON | have | SW17 |
| M29 | `.dspipreset` with apply options and report | have | TM10 | have | SW17 |
| M30 | Save Master Volume, Save Output Configuration | have | **have**: `live.rs:1349-1352`, `actions.rs:1393,1412`. TM10's check is answered | have | have |
| M31 | AutoEQ favourites | have | have | have | SW18 |
| M32 | AutoEQ update, rebuild from GitHub, import, Reset to Built-in | have; QD27 re-points | CG6, TM12 | CG6, AN18 | CG6, SW18 |

**Device settings**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M33 | Input-source names | QD4 | **partial**: command only, `in.name` (`crates/dspi-proto/src/registry.rs:2274`); no screen uses it. **TM17** | AN6 | have |
| M34 | Device name (`DSPi_ConsoleApp.swift:1740`) | QD4 | **partial**: command only, `dev.name` (`registry.rs:2879`); not on Global Parameters. **TM17** | AN6 | have |
| M35 | Output latency (`DSPi_ConsoleApp.swift:1912`) | QD5 | **partial**: command only, `out.latency` (`registry.rs:2897`); not on Global Parameters. **TM17** | AN6 | have |
| M36 | Core-voltage status | QD3 | TM2 | AN6 | have |
| M37 | Pair-map editor, K340 preset, in files and diff | QD2, CG7 | TM1, TM10 | have; CG7 | have |
| M38 | Tube Basic and Advanced | have | TM7 | have | have |
| M39 | Demo device (dev flag on Mac) | QD7 | have (R1) | have | have |
| M40 | USB MIDI (dev flag on Mac) | CG1, as a dev option | have (R1) | n.a.: vendor USB works | have; SW5 |
| M56 | Core-voltage writes and calibration under Advanced. **Above the floor:** owner's addition; the Mac has status only today | QD28 | have (CLI: set, trial, auto, clear) | AN23 | SW23 (both targets) |
| M55 | Settings: Spectrum, Graphing, Global Parameters (`DSPi_ConsoleApp.swift:1707-2381`) | **partial**: Spectrum and Graphing complete. Global lacks Device Name (QD4) and Output Latency (QD5), so the rest of the row closes with those | **partial**: Spectrum and Graphing complete; Glow has no terminal meaning (`settings/config.rs:21-31`). Global lacks Device Name and Output Latency. **TM17** | **partial**: Spectrum and Graphing complete. Global lacks Device Name and Output Latency (AN6) | **have**: `iOS/IOSApp.swift:306` wraps the same `SettingsView` |

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
| Software Update | QD16 | n.a. | n.a.: Play | n.a.: App Store (`:736`) |
| Advanced | `AdvancedPage.qml`; M62 | `settings/advanced.rs` | `AdvancedSettingsTab.kt` | shared |
| Networking | QD14 | TM11 (CLI) | `NetworkingSettingsTab.kt`, client half | n.a.: excluded from the target; the Devices sheet instead (`iOS/IOSApp.swift:380-414`) |
| Graphing | `GraphingPage.qml` | `settings/graphing.rs` | `GraphingSettingsTab.kt` | shared |
| Spectrum Analyser | `SpectrumPage.qml` | `settings/spectrum.rs` | `SpectrumSettingsTab.kt` | shared |
| Overview | `OverviewPage.qml` | `settings/overview.rs` | `PinOverviewTab.kt` | shared |
| Inputs | `InputsPage.qml` | `settings/inputs.rs` | `HardwareSettingsTab.kt` | shared |
| Outputs | `OutputsPage.qml` | `settings/outputs.rs` | `HardwareSettingsTab.kt` | shared |
| I2S Configuration | `I2SPage.qml` | `settings/i2s.rs` | `HardwareSettingsTab.kt` | shared |
| Global Parameters | `GlobalParametersPage.qml`; M55 | `settings/global.rs`; M55 | `GlobalSettingsTab.kt`; M55 | shared |
| Control Surfaces | `ControlSurfacesPage.qml` | `settings/surfaces.rs` | `ControlSurfacesSettingsTab.kt`, view and run; AN9–AN11 | shared |
| Control Interfaces | `ControlInterfacesPage.qml` | `settings/interfaces.rs` | `ControlInterfacesSettingsTab.kt` | shared |
| Channel Groups, Macros, Auxiliary Outputs | `ChannelGroupsPage.qml`, `MacrosPage.qml`, `AuxOutputsPage.qml` | `settings/groups.rs`, `settings/macros.rs`, `settings/surfaces/aux_outputs.rs` | `ControlSurfacesSettingsTab.kt`, view and run (`SettingsScreen.kt:225-227`); AN12 | shared |

Pages were checked for existence, except Global Parameters, Graphing and Spectrum Analyser (M55), whose rows were compared one by one.
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
| Notes for users: the PipeWire card profile for system volume, KDE effects on Wayland, Qt logging to the journal | `apps/desktop/README.md`, Troubleshooting |
| Channel ids: the Qt bridge's app ids (0-1 inputs 1-2, 2-10 outputs 1-9, 11-16 inputs 3-8) against the core's wire indices | `apps/desktop/qt/src/DSPiBridge.h` (`kAppExtraInputFirst`) and `DSPiBridge::wire` in `DSPiBridge.cpp` |
| Notes on the old core's bulk image (`bulk_raw`, `encode_bulk`, DAC mute and bulk SET) | Dropped: they describe the old core, which the merge replaced (`docs/core-merge/PLAN.md`) |
