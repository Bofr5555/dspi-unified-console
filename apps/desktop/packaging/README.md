# Packaging the Qt desktop app: uConsole for DSPi

<!-- WP QP20 --> The Qt app for Linux and Windows, as people install it. The name they see is
"uConsole for DSPi" (`qt/src/AppIdentity.h`); files, packages and commands are `uconsole-for-dspi`
(`/usr/bin/uconsole-for-dspi`, the desktop entry and AppStream id `uconsole-for-dspi`) and the release files are
`uConsole-for-DSPi-<version>-…`, below (WP RN1). The Mac release is the Swift app (`apps/apple`); there is no Qt `.dmg`.

## Building a release

From a clean commit, on the host (podman, the network for cargo's crates; nothing on tmpfs):

```
apps/desktop/packaging/build-release.sh [--key MINISIGN_SECRET_KEY] [output dir]
```

It builds each file in its own image from the Containerfiles here and gathers them, under the names below,
in one flat folder (default `~/.cache/dspi-build/work/release-<version>`):

| File | Built by, in | Installs |
|---|---|---|
| `uConsole-for-DSPi-<v>-windows-x86_64.zip` | `windows/build-windows.sh`, `windows/Containerfile` (Fedora 44, MinGW Qt 6) | a folder that runs as is |
| `uConsole-for-DSPi-<v>-windows-x86_64-setup.exe` | the same (NSIS, `windows/uconsole-for-dspi.nsi`) | per user, `%LOCALAPPDATA%\Programs\uConsole for DSPi`, no administrator |
| `uConsole-for-DSPi-<v>-x86_64.AppImage` | `linux/build-ubuntu.sh` + `linux/make-appimage.sh`, `linux/Containerfile.ubuntu` (Ubuntu 24.04, Qt 6.8.3 from download.qt.io) | runs as is; glibc 2.39 or newer |
| `uConsole-for-DSPi-<v>-linux-x86_64.tar.gz` | the same | the AppImage with `install.sh` (per user, menu entry; `--udev` adds the rule), `README.txt`, the rule, the notices |
| `uconsole-for-dspi_<v>_amd64.deb` | `linux/build-debian.sh`, `linux/Containerfile.debian` (Debian 13's Qt) | Debian 13 |
| `uconsole-for-dspi-<v>-1.fc<N>.x86_64.rpm` | `linux/build-fedora.sh`, `linux/Containerfile.fedora` (`FEDORA=<N>`; 43 and 44) | Fedora N only: the QML is compiled against that release's Qt |
| `uConsole-for-DSPi-<v>-desktop-source.tar.gz` | `ci/release-source.sh --with-desktop` | the source the notices name |

The AUR package (`arch/PKGBUILD`, `.SRCINFO`, `uconsole-for-dspi.install`) builds from that source archive on
Arch; it is published to the AUR by hand.

**These names are a contract.** The app's update checker (`qt/src/platform/AppUpdater.cpp`) counts a GitHub
release as one for this app only when it carries `uConsole-for-DSPi-<v>-windows-…` (Windows) or
`uConsole-for-DSPi-<v>-linux-…` / `uConsole-for-DSPi-<v>-….AppImage` (Linux). Renaming them silently stops update
notices for every installed copy.

**The update feed** (`DSPI_UPDATE_FEED_URL`, QP17) is built into the project's own downloads only: the
Windows zip and setup, the AppImage and the archive (`build-windows.sh`, `build-ubuntu.sh` pass the
project's releases feed; set the variable empty to leave it out, as `ci/check.sh --step qt-windows`
does). The `.deb`, the `.rpm` and the AUR package never carry it: their package manager updates them.
`linux/test-packages.sh` checks both ways.

**Signing.** With a key (`--key` or `DSPI_MINISIGN_KEY`), `build-release.sh` ends with
`ci/release-sums.sh`, which writes `SHA256SUMS` over the folder and signs it (`SHA256SUMS.minisig`; the
public key is `docs/release/minisign.pub`, `docs/RELEASING.md`). Without a key it signs nothing and writes
no `SHA256SUMS`; the folder is complete otherwise, and `ci/release-sums.sh --key <key> <folder>` signs it
later. The release also carries the library sources (below).

**Tests.** `linux/test-packages.sh <folder>` installs each Linux file in a clean container of its
distributions (Debian 13; Fedora 43, 44; the AppImage on Ubuntu 24.04, Debian 13, Fedora 44) and runs the
app on the demo device; it checks names, the udev rule, the notices, the feed rule and the archive's
`install.sh`. `windows/test-installer.sh <setup.exe>` runs the installer under Wine:
`ci/check.sh --step qt-windows` builds the zip and the setup and runs it (also in `ci/matrix.sh`, row
`windows-qt`).

## Windows: the names the setup and the app write

Every entry carries the name "uConsole for DSPi": the uninstall key (`HKCU\…\Uninstall\uConsole for DSPi`),
the key that records the folder (`HKCU\Software\uConsole for DSPi`, `InstallDir`, where an update installs
again), the folder (`%LOCALAPPDATA%\Programs\uConsole for DSPi`) and the Start menu shortcut
("uConsole for DSPi.lnk"). The exe is `uconsole-for-dspi.exe`. The app's own entries are the app's: open at
login is the Run value "uConsole for DSPi" (QP14), which the uninstaller removes; settings are under
`%APPDATA%\uconsole-for-dspi` (QSettings `uconsole-for-dspi/uconsole-for-dspi`), and stay after an uninstall.
WP RN1 renamed all of them; no release had used the earlier names, so nothing is carried over. The setup refuses to replace or remove the files while the app runs (also from the
tray), with Retry; a silent run exits 2.

## Linux: the udev rule

`linux/70-uconsole-for-dspi.rules` gives the person at the seat access to a DSPi (USB vendor `2e8b`, and
`2e8a:feaa` for older firmware) through logind's `uaccess` tag. The `.deb`, the `.rpm` and the AUR package
install it in `/usr/lib/udev/rules.d/` and reload udev (`linux/postinst`, `postrm`,
`arch/uconsole-for-dspi.install`); the archive's `install.sh --udev` installs it in `/etc/udev/rules.d/`. The
app itself offers its own rule (`70-dspi.rules`) when a DSPi cannot be opened.

## Licence texts in the packages

The desktop package's licence label is **GPL-3.0-only**: the app as a whole is distributed under the GNU
GPL version 3, while Bofr5555's own contributions are GPL-3.0-or-later with the additional terms of
`licences/TERMS.txt`. The label is in every notice ("Licence: GPL-3.0-only"), the `.rpm`'s License field,
the PKGBUILD, the AppStream `project_license`, and About (beside the GPL's text, from the core's About
table). The rest of the project keeps its own labels. The notices are drafts until the owner approves them.

Every package carries the app's licence and, where it bundles libraries, theirs:

| Package | Where | What |
|---|---|---|
| AppImage | `usr/share/doc/uconsole-for-dspi/` | `NOTICE.txt`, `LICENSES/`: GPL-3.0, LGPL-3.0, `TERMS.txt`, `rust-crates.txt`, `Lucide/`, the firmware's notices, AutoEq's MIT, `Qt/` (Qt 6.8's licence files and `THIRD-PARTY.txt`), `ICU/`, `AppImage-runtime/` (the runtime's, libfuse's LGPL-2.1, squashfuse's, and Alpine's musl, zlib, zstd and mimalloc it links), `BUNDLED.txt`, `bundled/<Ubuntu package>/copyright` |
| `-linux-x86_64.tar.gz` | top level | the AppImage's `NOTICE.txt` and `LICENSES/` |
| Windows `.zip` and setup | next to `uconsole-for-dspi.exe` | `NOTICE.txt`, `LICENSES/`: as the AppImage's, with `bundled/<Fedora package>/` (MinGW Qt's, GCC's runtime with its exception, the other DLLs') instead of ICU and the runtime |
| `.deb` | `/usr/share/doc/uconsole-for-dspi/` | `copyright` and `NOTICE.txt` (the notice with the additional terms; the GPL is Debian's `/usr/share/common-licenses/GPL-3`), `LICENSES/`: `TERMS.txt`, `rust-crates.txt`, `Lucide/`, the firmware's notices, AutoEq's MIT |
| `.rpm`, AUR, a plain `cmake --install` | `/usr/share/licenses/uconsole-for-dspi/` | `NOTICE.txt`, `LICENSES/`: as the `.deb`'s, with `GPL-3.0.txt` |

The app's About lists the files of whichever folder it finds (`AboutController`). The layout and its
notice (`licences/NOTICE-distro.txt.in`) for the `.deb` and `.rpm` are `qt/cmake/wp/qp20.cmake`'s
(`DSPI_NOTICES=deb|rpm|none`).

<!-- WP LIC1 --> Every notice states the additional terms under GPL-3.0 section 7 (the project's `NOTICE.md`),
verbatim from `licences/TERMS.txt`, which a test in `crates/dspi-session` (about.rs) holds equal to the words every
client's About shows (`assets/about.json`). `licences/rust-crates.txt` lists the Rust crates in the core
(`dspi_capi`) with their licence texts, for Linux, Windows and macOS together; `licences/gen_rust_notices.py`
writes it and `ci/check.sh --step qt` fails when it is stale.

`licences/stage.sh` lays these out for the AppImage (`linux/make-appimage.sh`), Windows
(`windows/build-windows.sh`) and the Swift macOS app (`apps/apple/package-macos.sh`, which keeps its own
name). It lists every library in the package that is not the app's own in `LICENSES/BUNDLED.txt`
(package, version, origin, source, licence, files) and fails on a file it cannot account for. The `.deb`
and `.rpm` use the distribution's Qt, so they carry no Qt texts.

The texts in `licences/` are copied verbatim from their official sources by `licences/update-licences.sh`:
the GPL-3.0 and LGPL-3.0 from www.gnu.org, Qt's `LICENSES/` and third-party notices from Qt's source
archives (`licences/qt-third-party.py`), ICU's `LICENSE`, the AppImage runtime's, libfuse's and
squashfuse's from theirs, and (`licences/fedora/`) the licence file of each Fedora MinGW package that ships
none (bzip2, OpenSSL, zlib, win-iconv) from the upstream archive in its source RPM. Lucide's comes with
the icons (`qt/icons/fetch-lucide.sh`). When a pinned Qt or ICU version, the AppImage runtime
(`linux/appimage-runtime.sha256`) or such a Fedora package changes, run `release-sources.sh` (with the
dist directories) and then `update-licences.sh`, and commit the result; until then the build fails, as it
has no texts for the new version.

## The release's sources: `release-sources.sh`

The release carries the source of each library the packages bundle as binaries (the LGPL requires it for
Qt). Before publishing a release, after `build-release.sh`, with its two dist folders (the AppImage's
archive and the unpacked Windows bundle):

```
apps/desktop/packaging/release-sources.sh ~/.cache/dspi-build/work/release-build/dist \
    ~/.cache/dspi-build/work/release-build/windows/dist
```

It downloads into `~/.cache/dspi-build/work/release-sources/` (or `$RELEASE_SOURCES`), checks every file,
and writes `MANIFEST.tsv` (packages, file, URL, SHA-256, how it was checked):

- `qt/`: Qt's source archive of each module the AppImage and the Windows bundle ship (versions and
  modules read from `linux/qt-<version>.sha256` and `windows/Containerfile`'s `MINGW_QT_VERSION` by
  `licences/qt-modules.sh`), from download.qt.io, against the SHA-256 download.qt.io publishes for each
  (Qt publishes no signatures).
- `icu/`: the ICU version in the AppImage, from the ICU project's GitHub release: `SHASUM512.txt` and its
  signature against ICU's `KEYS`.
- `ubuntu/`: the Ubuntu 24.04 source packages of the AppImage's other libraries (`BUNDLED.txt`), by
  `apt-get source` (the archive's signed index; a version the archive has since replaced comes from
  snapshot.ubuntu.com at the time Launchpad states it was published).
- `runtime/`: the AppImage runtime (AppImage's type2-runtime, at the commit it states): its tree at that commit
  by git (`git archive`; the commit hash checks every file), and the archives its build script
  (`scripts/common/install-dependencies.sh`) downloads and pins by SHA-256: libfuse 3.15.0 (LGPL-2.1, linked
  in statically, with the patch in the tree's `patches/libfuse/`; its release signature checked against its
  maintainer's key) and squashfuse 0.5.2.
  <!-- WP LIC1 --> And the Alpine 3.21 packages the runtime links statically, as
  `licences/appimage-runtime-<commit>/ALPINE.txt` pins them (musl 1.2.5-r11, zlib 1.3.2-r1, zstd 1.5.6-r2,
  mimalloc2 2.1.7-r0 for fedfad5, read from the log of the GitHub Actions job that built the released
  runtime): each package's aports directory (APKBUILD and patches) by git at the pinned commit, and the
  upstream archive its APKBUILD names, against the APKBUILD's SHA-512 (Alpine's distfiles mirror if upstream
  is gone; musl's release signature against musl's key). The APKBUILD is read only in a container with no
  network.
- `fedora/`: the Fedora source RPMs of the Windows DLLs (`BUNDLED.txt`: MinGW Qt, GCC's runtime, GLib,
  ...), by `dnf download --srpm`, signatures checked against Fedora's key.

Without dist directories it fetches only Qt. Fedora's mirrors keep only current package
versions, so run it soon after the Windows build. It is not run in CI, and nothing is published. The
archives are never committed.

**Publishing a release:** every notice says the bundled libraries' source "is included with this release", so
attach the whole output of `release-sources.sh` (every file `MANIFEST.tsv` lists, and `MANIFEST.tsv` itself)
to the GitHub release, next to the binaries, built from the same tree and dist directories. A release
without them does not match its notices.
