# Installing uConsole for DSPi

Every client of release **0.14.0** shares one version number. The files are on the
[Releases page](https://github.com/Bofr5555/dspi-unified-console/releases) (tag `v<version>`); the
Android app is also on Google Play. The app is called **uConsole for DSPi**: every release file starts with
`uConsole-for-DSPi-<version>`, and packages, commands and installed files use `uconsole-for-dspi` (the
Terminal's command stays `uconsole`).

**No DSPi yet?** Every client has a demo device (`--demo` on the desktop and in the Terminal, Demo mode on
Android and in the web console), so you can try every screen first.

- [Windows](#windows) · [Linux](#linux) (Debian/Ubuntu, Fedora, AppImage and tar.gz, Arch) ·
  [USB access on Linux](#usb-access-on-linux) · [Android](#android) · [Terminal](#terminal-uconsole) ·
  [Web console](#web-console) · [macOS, iPhone and iPad](#macos-iphone-and-ipad) ·
  [Fork firmware](#fork-firmware) · [Check a download](#check-a-download) · [Source code](#source-code)

The desktop installers and the Terminal binaries are **not code-signed** yet, so each system warns the first
time; every file is listed in a signed `SHA256SUMS` instead ([Check a download](#check-a-download)).

## At a glance

| System | Download | How |
|---|---|---|
| **Windows** 10/11 (x86_64) | `uConsole-for-DSPi-<version>-windows-x86_64-setup.exe`, or the portable `…-windows-x86_64.zip` | [Windows](#windows) |
| **Debian 13, Ubuntu 25.04 or newer** | `uconsole-for-dspi_<version>_amd64.deb` | `sudo apt install ./uconsole-for-dspi_<version>_amd64.deb` |
| **Fedora 43 or 44** | `uconsole-for-dspi-<version>-1.fc<N>.x86_64.rpm` (one per Fedora release) | `sudo dnf install ./uconsole-for-dspi-<version>-1.fc<N>.x86_64.rpm` |
| **Other Linux** (x86_64, glibc 2.39+, e.g. Ubuntu 24.04) | `uConsole-for-DSPi-<version>-x86_64.AppImage`, or `…-linux-x86_64.tar.gz` with an installer | [AppImage and tar.gz](#appimage-and-targz) |
| **Arch Linux** | the AUR package `uconsole-for-dspi` | [Arch](#arch-linux) |
| **Android** 8+ | Google Play (closed testing), or `uConsole-for-DSPi-<version>.apk` | [Android](#android) |
| **Terminal** (`uconsole`, every system) | `uConsole-for-DSPi-<version>-terminal-<system>.tar.gz` / `.zip` | [Terminal](#terminal-uconsole) |
| **Web console** | `uConsole-for-DSPi-<version>-web.tar.gz` | [Web console](#web-console) |
| macOS, iOS, iPadOS | not in this release | [macOS, iPhone and iPad](#macos-iphone-and-ipad) |

## Windows

Run `uConsole-for-DSPi-<version>-windows-x86_64-setup.exe`. SmartScreen warns about an unknown publisher:
*More info* → *Run anyway*. No driver step is needed on a clean machine. The zip is the same app without an
installer: unpack it and run `uconsole-for-dspi.exe`.

The Windows files check this project's releases for updates when that is switched on (Settings > Updates).

## Linux

The Qt app needs Qt 6.8 or newer, which the packages bring along or depend on. Every Linux package installs the
USB access rule ([below](#usb-access-on-linux)); the AppImage cannot, so you install it once.

### Debian and Ubuntu

```sh
sudo apt install ./uconsole-for-dspi_<version>_amd64.deb
```

Debian 13 and Ubuntu 25.04 or newer. Updates come through the package, not through the app's update check.

### Fedora

```sh
sudo dnf install ./uconsole-for-dspi-<version>-1.fc44.x86_64.rpm     # or .fc43 on Fedora 43
```

### AppImage and tar.gz

For other distributions with glibc 2.39 or newer (Ubuntu 24.04, for example). The AppImage brings its own Qt:

```sh
chmod +x uConsole-for-DSPi-<version>-x86_64.AppImage
./uConsole-for-DSPi-<version>-x86_64.AppImage
```

`./uConsole-for-DSPi-<version>-x86_64.AppImage --version` (or `--help`) answers without opening a window, so it
also works over SSH.

The tar.gz holds the same app with an installer: unpack it and run `./install.sh`. It installs for you, without
root (the app into `~/.local/bin`, a menu entry and an icon); `./install.sh --udev` also installs the USB rule (it
asks for the password), and `./install.sh --uninstall` removes it again. The AppImage and the tar.gz check this
project's releases for updates when that is switched on (Settings > Updates).

### Arch Linux

The AUR package `uconsole-for-dspi` builds from the release's `…-desktop-source.tar.gz`
([`apps/desktop/packaging/arch/PKGBUILD`](../apps/desktop/packaging/arch/PKGBUILD)): `makepkg -si` in the
package's folder.

### USB access on Linux

Linux needs a udev rule before a program can open a DSPi over USB without root. There is **one rule for every
component** (the Qt app, the Terminal and the packages), which lets any user on the machine open a DSPi
(`MODE="0666"` for the DSPi's USB IDs). Install it once, by whichever way fits:

| You have | How the rule gets installed |
|---|---|
| the `.deb`, the `.rpm` or the AUR package | by the package |
| the tar.gz | `./install.sh --udev` |
| the AppImage | the app offers to install it when it cannot open a DSPi; or take the rule file from the tar.gz and copy it to `/etc/udev/rules.d/` |
| the Terminal | `sudo uconsole --install-udev` (`uconsole doctor` says whether it is needed) |
| the web console in Chrome or Edge | any of the above: WebUSB uses the same rule |

After a manual copy, reload the rules (`sudo udevadm control --reload-rules`) and replug the DSPi. A DSPi in
BOOTSEL mode (for a firmware update) is a USB drive and needs no rule. The packaging details are in
[`apps/desktop/packaging/README.md`](../apps/desktop/packaging/README.md).

## Android

- **Google Play:** the app is in **closed testing** on Google Play (Android 8.0 or newer); once you are a tester,
  install it from its Play listing.
- **Without Play:** `uConsole-for-DSPi-<version>.apk`. It is a separate app, *uConsole for DSPi (APK)*, so it
  installs next to the Play version rather than over it. Allow installs from your browser or file manager when asked.

Connect the DSPi with a USB-C or OTG cable or adapter. The app's own page is
[`apps/android/README.md`](../apps/android/README.md).

## Terminal (`uconsole`)

| System | File |
|---|---|
| Linux x86_64 and aarch64 (glibc 2.39+) | `uConsole-for-DSPi-<version>-terminal-linux-x86_64.tar.gz`, `…-linux-aarch64.tar.gz` (no MIDI on aarch64) |
| Windows x86_64 | `uConsole-for-DSPi-<version>-terminal-windows-x86_64.zip` |
| macOS (universal) | `uConsole-for-DSPi-<version>-terminal-macos-universal.tar.gz` |

Unpack it and run `uconsole` (the interface) or `uconsole --help` (the commands). `uconsole --demo` runs on the
simulated device. On Linux run `sudo uconsole --install-udev` once for USB access. The Terminal also runs the hub
(`uconsole hub`) and serves the web console (`uconsole web`). More: [`apps/terminal/README.md`](../apps/terminal/README.md).

## Web console

The web console runs in the browser and is served by the `uconsole` Terminal program or by a web server. Pick one:

1. **On this computer, with USB and hubs:** unpack the web download into a folder named `web` next to the
   `uconsole` program (or pass `--console <folder>`), run `uconsole web`, and open
   **http://localhost:11916/console/** in Chrome or Edge. USB needs a Chromium-based browser (Firefox and Safari
   have no WebUSB); on Linux the [USB rule](#usb-access-on-linux) is needed.
2. **From a hub on the network:** run `uconsole hub --web --console <folder>` on the machine the DSPi is plugged
   into, then open **https://&lt;that machine&gt;:11915/** from any browser on the network. The hub's certificate
   is its own (self-signed), so the browser warns once: accept it after comparing the fingerprint the page shows
   with the one the hub prints (`uconsole hub tls fingerprint`). The page links to the console (`/console/`) and to
   the hub page (`/hub/`). Pair with the hub's PIN when asked. A PIN gives the Control role; to make a console an
   Admin (for the hub's limits and installer PINs), run `uconsole hub clients` and `uconsole hub role <cid> admin`
   on the hub's machine, or type `clients` and `role <cid> admin` at the running hub's prompt.
3. **On your own web server:** serve the unpacked folder over HTTPS ([`apps/web/DEPLOY.md`](../apps/web/DEPLOY.md)
   has the details, also for embedding it in a fleet product). USB works there in Chromium; hubs can only be
   reached from a page the hub itself serves (option 2), because a hub accepts only its own pages, or through the
   server's own `wss://` proxy to them (DEPLOY.md). Option 1 is the recommended way to hubs: `uconsole web` reaches
   them over TLS pinned to each hub's key, so the browser shows no warning
   ([`integrator/security.md`](integrator/security.md), The browser).

Keep a hub on the local network: it drops peers that are not on a local network. For access from elsewhere use a
VPN ([`integrator/security.md`](integrator/security.md)).

## macOS, iPhone and iPad

The Swift apps for macOS and iOS ([`apps/apple/`](../apps/apple/)) exist and run on the same core, but they are
**not part of release 0.14.0**: the owner has deferred the Apple apps. On a Mac today you can use the Terminal
(`uConsole-for-DSPi-<version>-terminal-macos-universal.tar.gz`) and the web console it serves (`uconsole web`,
in Chrome or Edge for USB). The Qt app has no macOS build.

## Fork firmware

The apps work with stock Weeb Labs firmware. The features marked † in [`FEATURES.md`](FEATURES.md#firmware-stock-vs-fork)
need the DSPi fork firmware, release [**v1.1.6-bofr1**](https://github.com/Bofr5555/DSPi/releases/tag/v1.1.6-bofr1), the
firmware 0.14.0 was tested against. Not an official Weeb Labs release. Pick the image for your board:

| Board | Image |
|---|---|
| RP2040 (Pico) | `DSPi-v1.1.6-bofr1-rp2040.uf2`, or `…-rp2040-midi.uf2` with USB MIDI control |
| RP2350 (Pico 2) | `DSPi-v1.1.6-bofr1-rp2350.uf2`, or `…-rp2350-midi.uf2` with USB MIDI control |
| K340 (RP2350 at 1.20 V) | `DSPi-v1.1.6-bofr1-rp2350-midi-vreg1200.uf2` |
| A Pico 2 as the alink sender | `DSPi-v1.1.6-bofr1-rp2350-alink-sender.uf2` |

Flash it with the app's firmware update (choose the `.uf2` file; the update check reads the board and the image's
stamps first and refuses an image for the other kind of device), or by hand: hold BOOTSEL while you plug the board in
and copy the `.uf2` file to the drive that appears. The release page lists every file's SHA-256 and carries a
`SHA256SUMS` signed with the same key as the apps ([Check a download](#check-a-download)). Back up your presets first.

## Check a download

`SHA256SUMS` lists every file of a release and is signed with the owner's [minisign](https://jedisct1.github.io/minisign/)
key; the public key is [`release/minisign.pub`](release/minisign.pub):

```
RWTiNwy5h2MBuviJDs7LBs3wi2Bh/YdJyh5pXdS3XKo8wRt1igYdrGgY
```

1. Download `SHA256SUMS`, `SHA256SUMS.minisig` and the files you want into one folder.
2. Check the signature (minisign is packaged for most systems: `apt install minisign`, `dnf install minisign`,
   `brew install minisign`, `scoop install minisign`):

   ```sh
   minisign -Vm SHA256SUMS -P RWTiNwy5h2MBuviJDs7LBs3wi2Bh/YdJyh5pXdS3XKo8wRt1igYdrGgY
   ```

   It must say `Signature and comment signature verified`.
3. Check the files against the signed list:
   - Linux: `sha256sum --ignore-missing -c SHA256SUMS`
   - macOS: `shasum -a 256 --ignore-missing -c SHA256SUMS`
   - Windows (PowerShell): `Get-FileHash .\FILE -Algorithm SHA256`, and compare the hash with the file's line in
     `SHA256SUMS`.

A file that fails either check is not this release: do not run it. How releases are made and signed:
[`RELEASING.md`](RELEASING.md).

## Source code

Each release carries the source of the exact commit it was built from, `uConsole-for-DSPi-<version>-source.tar.gz`
(the Qt packages' is `…-desktop-source.tar.gz`). To build from source, every toolchain runs in a container; see
[Building and testing](BUILDING.md).
