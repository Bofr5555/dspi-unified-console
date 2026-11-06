# DSPi console apps: feature parity

Written 2026-11-06. This is a code-reading survey: nothing was built or run for it.

The reference is the **macOS Swift DSPi Console**: upstream Weeb Labs v1.1.6 (`51fd1bb`) plus our stack, branch `ext-channel-116` (`48cf7c5`). Our stack adds FakeTransport, USB MIDI, the iOS target, the iOS Link client UI and the extension channel. iOS builds from the same sources.

Each other app is compared against that reference:

| App | Source | Version surveyed |
|---|---|---|
| **iOS** | the iOS target of the same Swift project (`ext-channel-116`) | same branch as the reference |
| **Qt desktop** | `dspi-unified-console/apps/desktop` on `crates/dspi-core-linux` | — |
| **Terminal** | `apps/terminal` + `crates/dspi-{proto,transport,session,cmd,tui}` | — |
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
6. **No accessibility** (no `Accessible.*` anywhere), **no localisation** (no `qsTr`), and no tray or menu-bar mode.
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
| B2 | Input source and source names | ✓ | ✓ (long-press) | ◐ no source names | ✓ | ✓ | ◐ no names |
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
| B23 | Output latency target (0x03) | ✓ | ✓ | ✗ | ✓ `out.latency` | ? not checked | ✗ |

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
| G6 | Accessibility | ◐ SwiftUI defaults + `.help` | ◐ Dynamic Type capped at xxxLarge, 44 pt targets | ✗ | ◐ `--lite`, ASCII fallback, no screen reader | ◐ thin TalkBack | ✗ |
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
| Accessibility (`Accessible.name/role` on custom controls) | M | UI |
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
  - Android: B22, and B23 (not checked).
- **Qt core vs Terminal crates.** The Qt desktop's `dspi-core-linux` has constants hand-copied at 1.1.6 beta4. The Terminal crates are generated from `bofr-main` `af001ec` (1.1.6 final + ext channel). Moving the desktop onto the Terminal crates closes several Qt rows at once: B2, B3, B23, C2 and A11/B14/B22 (core side).
- **Reference version.** The macOS reference is `ext-channel-116`. Our additions there (MIDI, demo, ext channel, pair map, core-voltage status, iOS UI) are not in upstream Weeb Labs v1.1.6.
