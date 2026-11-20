# Features, and where each one works

Release **0.14.0**. Every feature lives once in the Rust core (`crates/`); each client shows it. This page is the
short view; the full, cell-by-cell comparison with the macOS reference console, with every caveat, is
[`PARITY.md`](PARITY.md) (sections 2, 5 and H to K), and the user's pages are under [`manual/`](manual/).

✓ works · ◐ partly (the note says what) · – not offered · *later* planned for a later batch.
**Apple** is the Swift app for macOS and iOS in [`apps/apple/`](../apps/apple/): it exists and runs on the core,
but it is **not part of release 0.14.0** (the owner has deferred it), so its column says what the code on `main` has.

**Firmware** says what the DSPi itself must run: **stock** is Weeb Labs' firmware, 1.1.6 or upstream's paged 1.1.7;
**fork †** needs the DSPi fork firmware ([below](#firmware-stock-vs-fork)).

## Firmware: stock vs fork

† **Needs the DSPi fork firmware** on the device: [Bofr5555/DSPi](https://github.com/Bofr5555/DSPi), branch
`bofr-main`. The consoles themselves are the same for both; they ask each DSPi once per connection which firmware
generation it runs and which of the fork's extensions it has (the extension channel: request `0x00` on 1.1.6, the
private page `0xE0` on paged firmware), and simply do not offer what a device lacks.

**On stock firmware (1.1.6, or the paged 1.1.7) everything else works:** every DSP control, presets, files and
AutoEQ, the analyser and generator, undo, snapshots, A/B and safe apply, limits profiles and the installer lock, the
modes, the hub with its roles, TLS, history, alarms, schedules, MQTT, REST, groups and rollout, the device's health
(load, temperature, the clip latch), firmware updates with their checks, and **measurement and auto-tune of one unit**
(the generator of firmware 1.1.5 and later is the stimulus).

What needs the fork firmware, checked against the code and the firmware trees (upstream's v1.1.6 and
`release/v1.1.7` have none of it):

| Feature | Why | The fork's extension |
|---|---|---|
| The audio link (alink): the System view's live state, alignment, scenes recalled at a frame, alink alarms | the alink is the fork's | `EXT_ALINK_*` (`0x8000`-`0x800A`) |
| The alink sender itself | its firmware is the fork's (`firmware/DSPi-Sender`) | – |
| USB-direct units: phase lock, delay, lock errors | the fork's (`0x8010`-`0x8012`) | `EXT_ALINK_USB*` |
| S/PDIF units on a DSPi time base, the time base itself | the fork's | `EXT_ALINK_SPDIF*`, `EXT_TIME_BASE` (`0x8020`-`0x8022`, `0x8030`) |
| Measuring several units together | each unit's generator starts on a sample of the shared time base | `EXT_SIGGEN_TIMELINE` (`0x8031`) |
| Core voltage: reading the live and stored setting, setting it, calibration, the K340's 1.20 V | upstream runs a fixed 1.15 V | `EXT_CORE_VREG` |
| Identify (the device blinks its LED) | the hub answers `unsupported` without it | `EXT_IDENTIFY` |
| Health: uptime, restarts with their reason, hang count | read from the reset record | `EXT_RESET_INFO` |
| Health: clip events per minute | stock has only the sticky clip latch | `EXT_NOTIFY_FEATURES` |
| The crossfeed pair map (Default, K340, Custom) | the fork's | `EXT_CROSSFEED_PAIR_MAP` |
| USB MIDI control (SysEx), used by the Apple apps on iPhone and iPad | upstream builds without USB MIDI | – |
| Image stamps (device kind, core voltage) | only the fork's images carry them; the update check reads them, and treats an unstamped image as possibly upstream's | – |

The fork's firmware is of the 1.1.6 generation today; its port onto 1.1.7 waits for upstream's 1.1.7 tag, and its
alink sender already speaks both ([`STATUS.md`](STATUS.md)). On our hub the fork's extensions are for Admin sessions
([`manual/notices.md`](manual/notices.md), section 3).

## The console: one DSPi

| Feature | Firmware | Qt desktop | Android | Web | Terminal | Apple (not in 0.14.0) |
|---|---|---|---|---|---|---|
| USB | stock | ✓ | ✓ (USB-C/OTG) | ✓ WebUSB, Chrome and Edge | ✓ | ✓ macOS |
| USB MIDI (SysEx) | fork † | – | – | – (*later*: Web MIDI) | – | ✓ iPhone, iPad, Mac |
| Demo device, no hardware | – | ✓ `--demo` | ✓ Demo mode | ✓ | ✓ `--demo` | ✓ |
| EQ with the response graph, crossovers, delays, gains, routing, matrix mixer | stock | ✓ | ✓ | ✓ | ✓ the interface (TUI) and commands | ✓ |
| Effects: loudness, crossfeed, leveller, bass, subharmonic, tube, upmixer, limiters | stock | ✓ | ✓ | ✓ | ✓ | ✓ |
| Crossfeed pair map | fork † | ✓ | ✓ | – | ◐ `uconsole ext xfeed-map` | ✓ |
| Presets, filter files, AutoEQ with a preview | stock | ✓ | ✓ | ✓ | ✓ `autoeq preview`, `autoeq apply` | ✓ |
| Signal generator, analyser, statistics, interrupt monitor | stock | ✓ | ✓ | ✓ | ✓ | ✓ |
| Undo and redo | stock | ✓ Edit menu, History window | ✓ | ✓ | ✓ in the interface | ✓ |
| Firmware update, with the update check (see below) | stock | ✓ | ✓ | ◐ checks an image; writes only through a hub's rollout | ✓ `uconsole flash` | ◐ writes; no update check yet |
| Core voltage | fork † | ✓ with calibration | ✓ | ✓ Installer mode | ✓ | ◐ |
| Device info: board, firmware generation, protocol | stock | ✓ Settings > About | ✓ Settings > About | ✓ Device screen | ✓ `uconsole info` | – |
| Both firmware generations, 1.1.6 and upstream's paged 1.1.7 | stock | ✓ | ✓ | ✓ | ✓ | ◐ on the core paths |
| Read-only for firmware it cannot write, with a notice | stock | ✓ | ✓ | ✓ | ✓ | ✓ |
| Diagnostics bundle | stock | ✓ Help > Save Diagnostics | – | ✓ Events screen (Advanced and up) | ✓ `uconsole diag` | – |

## Modes, installers and safety (PRO phase 1)

All of these are the core's and work on stock firmware.

| Feature | Firmware | Qt desktop | Android | Web | Terminal | Apple (not in 0.14.0) |
|---|---|---|---|---|---|---|
| Simple, Advanced and Installer modes ([`UI-MODES.md`](UI-MODES.md)) | stock | ◐ Settings > Installer; no mode switch shown yet | ✓ | ✓ | – (commands) | ✓ |
| Installer lock and PIN, with a recovery code | stock | ✓ | ✓ | ◐ unlocks a hub's lock; no PIN of its own | ✓ `lock` | ✓ |
| Limits profiles (output gain, high-pass, delay, limiter ceiling, PEQ boost, volume) | stock | ✓ | ✓ | ✓ | ✓ `limits` | ✓ |
| Snapshots, A/B compare | stock | ✓ | ✓ | ✓ | ✓ `snapshot`, `ab` | ✓ |
| Safe apply: Keep, or it goes back by itself | stock | ✓ | ✓ | ✓ | ✓ `--guard` (on by default over DSPi Link) | ✓ |
| A revert that did not fully succeed says so, and to check the settings or restore a snapshot | stock | ✓ | ✓ | ✓ | ✓ (and a non-zero exit) | – |
| Offline documents and templates | – | ✓ | ✓ | ✓ | ◐ `open`; no templates | ✓ |
| Document diff and partial import | stock | ✓ | ✓ | ✓ | ✓ `diff`, `import --only` | ✓ |
| Confirmation tokens on every dangerous call | stock | ✓ | ✓ | ✓ | ✓ `--yes` | ✓ |

## The hub and DSPi Link (PRO phases 2 and 3)

The hub is `uconsole hub` (the Terminal), or the Qt app's sharing (Settings > Networking). It speaks DSPi Link over TLS
1.3, with the hub's key pinned at pairing, and roles (Viewer, Control, Admin). It serves stock and fork devices alike.
Its API: [`integrator/hub-api.md`](integrator/hub-api.md); security: [`integrator/security.md`](integrator/security.md).

| Feature | Firmware | Qt desktop | Android | Web | Terminal | Apple (not in 0.14.0) |
|---|---|---|---|---|---|---|
| Open a DSPi through a hub (DSPi Link client, PIN pairing bound to the hub's key) | stock | ✓ | ✓ | ✓ | ✓ `--link` | ✓ |
| Run a hub | stock | ✓ Settings > Networking | – | – (served by one) | ✓ `uconsole hub` | ✓ the Mac's hub |
| Shared snapshots, the hub's history, the alarm banner | stock | ✓ | ✓ | ✓ | ◐ the hub runs them; read through `uconsole --hub api` | ✓ |
| Identify: the device blinks its LED | fork † | ✓ | ✓ | ✓ | ◐ through `uconsole --hub api` | ✓ |
| Inventory, alarms, schedules, notifications, MQTT and Home Assistant, REST | stock | – | – | ✓ Hub and Hub Settings screens | ✓ `uconsole hub` services | – |
| Health: load, temperature, the clip latch | stock | – | – | ✓ | ✓ the hub; `uconsole --hub api` | – |
| Health: uptime, restarts and their reason, clip events per minute | fork † | – | – | ✓ | ✓ the hub; `uconsole --hub api` | – |
| Fleet: groups, group writes and bulk apply, the health board, the Overview | stock | – | – | ✓ | – | – |
| Firmware rollout through the hub, one device at a time | stock | – | – | ✓ (Admin; TLS only) | ✓ `uconsole hub --rollout` | – |

## The audio link (alink): several units as one system

Every row here needs the fork firmware †: the alink, its sender and its time base are the fork's.

| Feature | Firmware | Qt desktop | Android | Web | Terminal | Apple (not in 0.14.0) |
|---|---|---|---|---|---|---|
| The System view: every unit, its role and lock state, audio link delay | fork † | ◐ Window > System…: a units table | ◐ the System screen: unit rows | ✓ rooms, zones, topology, slots | – | – |
| Clock and audio links: system delay, lip sync, alignment | fork † | ◐ no slot matrix, link stats or trim scan | ◐ as Qt | ✓ Installer mode, with the slot matrix and link stats | – | – |
| Scenes at a frame, refusals in words | fork † | ✓ | ✓ | ✓ | – | – |
| Compact status ("Audio link: 4 of 4 locked") | fork † | ◐ a status line | ◐ Home's chip | ✓ the status rail | – | – |
| USB-direct units (state, delay, lock errors) | fork † | ✓ | ✓ | ✓ | ✓ `uconsole ext usb-direct` | – |
| S/PDIF units on a DSPi time base | fork † | ✓ | ◐ view only | ✓ | – | – |
| The alink sender: its own page, the DSP controls greyed | fork † | ✓ one banner, DSP menus greyed | ✓ its own page | ✓ its own page | ◐ `uconsole info` names it | – |

## Measurement and auto-tune (PRO phase 5)

The user's page is [`manual/measurement.md`](manual/measurement.md). The DSPi plays the test signal from its own
generator; the console only listens through the microphone. Every client's demo device has a simulated microphone
in a simulated room.

| Feature | Firmware | Qt desktop | Android | Web | Terminal | Apple (not in 0.14.0) |
|---|---|---|---|---|---|---|
| Measure with a calibrated microphone (UMIK-1/2, REW, CSV, Dayton Audio files) | stock (1.1.5 and later) | ✓ Tools > Measure… | *later* | ✓ Measure screen | ◐ imports and documents; no microphone | *later* |
| Live response, microphone RTA, "cut this peak" / "fit this range" | stock | ✓ | *later* | ✓ | – | *later* |
| Auto-tune: delay and polarity, crossover corner and levels, target EQ; summed check, guarded apply, verify, Keep or Revert | stock | ✓ | *later* | ✓ | ◐ `measure tune` from a saved measurement | *later* |
| Passive RTA for a Viewer or a read-only connection | stock | ✓ | *later* | ✓ | – | *later* |
| Measurement, `mic` and `target` documents, local and on the hub; REW export | – | ✓ | *later* | ✓ | ✓ `measure docs`, `measure export` | *later* |
| Several units with one microphone, through the hub (overlapped starts where safe) | fork † | ✓ "Several units" tab | *later* | ✓ | – | *later* |

## Firmware updates that check first

Before a board is sent into its bootloader, the core checks the image against the board it read: a board without
BOOTSEL (the DSPi Zero) is offered nothing; leaving this fork's firmware asks you to type a phrase; a K340 at 1.20 V
asks for its voltage; an image built for the other kind of device (a DSP image on the alink sender, or the sender's
image on a DSP) is refused, by the **kind stamp** † every fork image carries; and the core's own write gate checks
again at the moment of writing. The user's page is [`manual/notices.md`](manual/notices.md), the details
[`integrator/protocol-generations.md`](integrator/protocol-generations.md).

| | Firmware | Qt desktop | Android | Web | Terminal | Apple (not in 0.14.0) |
|---|---|---|---|---|---|---|
| The checks before the bootloader entry | stock (the stamps: fork †) | ✓ | ✓ | ✓ Check an Image…, the rollout dialog | ✓ | – |

## Not on real hardware yet

Some of the above has only run against the simulated devices, fakes and a fake hub in CI. What is still owed on
hardware (several real units on one time base, a full measurement session with a real microphone, a real 1.1.7
board, the dual-stack alink sender on its board) is listed in [`STATUS.md`](STATUS.md).
