# Credits: the work this is built on

uConsole for DSPi is a console for [DSPi](https://github.com/WeebLabs/DSPi), Weeb Labs' open-source DSP firmware for
the Raspberry Pi Pico and Pico 2. It would not exist without Weeb Labs' firmware, consoles and tools.

## Weeb Labs

| Project | What this repository took from it |
|---|---|
| [DSPi](https://github.com/WeebLabs/DSPi), the firmware | The protocol: its headers and, for firmware 1.1.7, its command registry, vendored in `crates/dspi-proto/firmware/`; the bundled firmware images (release v1.1.6, unmodified) |
| [DSPi Console for macOS](https://github.com/WeebLabs/DSPi-Console) | The reference for behaviour, look and words that every client follows ([`PARITY.md`](PARITY.md)); the Swift app in `apps/apple/` is a fork of it |
| [DSPi Console for Windows](https://github.com/WeebLabs/DSPi-Console-Windows) | The Android app's PEQ graph editor, pin overview and hardware-change order (13 files, listed in [`NOTICE.md`](../NOTICE.md)) |
| DSPi Console for Linux (Qt) | The first Qt app here was a port of it; it was replaced on 2026-11-13 by a rebuild from the macOS console on the core |
| DSPi-Terminal | The crates the core is built on (`dspi-proto`, `dspi-transport`, `dspi-session`, `dspi-cmd`, `dspi-tui`) and the `uconsole` Terminal |
| DSPi-Link | The DSPi Link protocol and its test fixtures; the hub serves DSPi-Link's own web page at `/hub/` |

Weeb Labs: [GitHub](https://github.com/weeblabs) · [YouTube](https://youtube.com/weeblabs) ·
[Discord](https://discord.gg/RCyqxAQ5xS) · [Patreon](https://patreon.com/weeblabs) · [Ko-fi](https://ko-fi.com/weeblabs).

## Bofr5555

The core merge (one Rust core for every client), the unified console, the Android console, the Rust DSPi Link client,
the hub's services, the web console, measurement and auto-tune, and the firmware fork's extensions (the audio link and
its time base, the core-voltage setting, the image stamps) on [Bofr5555/DSPi](https://github.com/Bofr5555/DSPi), released
as [v1.1.6-bofr1](https://github.com/Bofr5555/DSPi/releases/tag/v1.1.6-bofr1).

## Who made what, and where it came from

Everything here builds on Weeb Labs' work: DSPi, the DSPi firmware, DSPi Console
for macOS and for Windows, DSPi-Terminal and DSPi-Link. The core merge, the Android console, the
Rust Link client and the unified console are Bofr5555's. The full list of people and projects, the
same one every client's About page shows, is
[`assets/about.json`](../assets/about.json) (a draft until the owner approves it);
third-party notices are in [`NOTICE.md`](../NOTICE.md).

The imports, with history (each upstream's paths rewritten with
`git filter-repo` and merged with `--allow-unrelated-histories`, so
`git log --follow` works on any file):

| Component | Upstream | Commit imported |
|---|---|---|
| `apps/terminal/`, `crates/dspi-{proto,transport,session,cmd,tui}`, `xtask/`, `tests/fixtures/*.txt` | WeebLabs/DSPi-Terminal `main` | `8389b20` |
| `crates/dspi-link/`, `apps/android/` | Bofr5555's Android console (`dspi-android`) `main` | `63492af` |
| `apps/apple/` | Bofr5555's fork of the macOS DSPi Console, branch `ext-channel-116` | `48cf7c5` |
| `tests/fixtures/link/` (data) | WeebLabs/DSPi-Link | `86f10e3` |

## Libraries and data

- **Rust crates** (`Cargo.lock`): among them `nusb` (USB), `rustls` (TLS), `tungstenite` (WebSocket),
  `mdns-sd`, `ratatui` (the Terminal's interface) and UniFFI and cbindgen for the bindings. Each package carries their
  licence texts.
- **Qt 6** (LGPL-3) for the desktop app; **Jetpack Compose** for the Android app; **Preact** (MIT) for the web
  console.
- **AutoEq** headphone data by Jaakko Pasanen (MIT), from measurements AutoEq credits to crinacle, innerfidelity,
  oratory1990, rtings and headphone.com.
- **In the firmware:** Raspberry Pi's pico-sdk and pico-extras, TinyUSB, LUFA, Elehobica's S/PDIF receiver and Alex
  Stanoev's original code; and code by Jonathan Rosser, Tim Prangenberg and Jeremy Blencowe.
- **Noto Sans Mono** glyphs in the app icon (SIL Open Font License 1.1).

The licences that apply are summarised in [`LICENCES.md`](LICENCES.md).
