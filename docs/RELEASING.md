# Releasing

How a release's files are put together and signed. The client-specific steps
(the Qt packages and the Qt `.dmg`, the Android bundle, the web dist) are in each
client's own docs; this page covers what every release shares. How a user installs and checks
the files is [`INSTALL.md`](INSTALL.md).

## The files

1. `ci/release-source.sh VERSION DIR`: the source archive of the exact commit.
2. `ci/release-terminal.sh BINARY PLATFORM VERSION DIR`: one per Terminal build.
3. `ci/release-desktop.sh [--test] VERSION DIR`: the Qt app's files (Windows zip
   and setup, AppImage, `linux-x86_64.tar.gz`, `.deb`, one `.rpm` per Fedora)
   and their source archive, `…-desktop-source.tar.gz`. It runs
   `apps/desktop/packaging/build-release.sh` from a clean commit, unsigned, and
   copies the files in; `--test` first installs the Linux ones in clean
   containers (`linux/test-packages.sh`). The AUR package (`arch/PKGBUILD`)
   builds from the desktop source archive and is published by hand after the
   release, with its `sha256sums` and `.SRCINFO` updated.
4. `apps/desktop/packaging/macos/build-macos.sh`: the Qt app's
   `…-macos.dmg`, built and tested on a Mac from the same clean commit
   (universal, ad hoc signed, not notarised; `apps/desktop/packaging/README.md`,
   "macOS: the .dmg"). A separate step, since it needs the Mac: copy the `.dmg`
   from `~/.cache/dspi-build/work/release-VERSION-macos/` into `DIR`.
5. The other clients' packages, copied into the same `DIR`: the Android APK, the
   web dist. The Swift apps are deferred (owner, 2026-11-19): the Mac's file is
   the Qt `.dmg` of step 4 (owner, 2026-11-21).
6. **Last:** `ci/release-sums.sh --key SECRET_KEY DIR` writes `DIR/SHA256SUMS`
   over every file in `DIR` and signs it as `DIR/SHA256SUMS.minisig`.

The update feed (the Qt app's update check, `DSPI_UPDATE_FEED_URL`) is built
into the project's own Windows files, AppImage, tar.gz and `.dmg` only; the `.deb`, the
`.rpm` and the AUR package are updated by their package managers and never
check (`apps/desktop/packaging/README.md`).

### The file names

Every release file starts with the release's stem, `uConsole-for-DSPi-VERSION-`; distribution packages
use the package name `uconsole-for-dspi`, as their systems expect.

| File | Made by |
|---|---|
| `uConsole-for-DSPi-VERSION-source.tar.gz`, `…-desktop-source.tar.gz` | `ci/release-source.sh [--with-desktop]` |
| `uConsole-for-DSPi-VERSION-terminal-PLATFORM.tar.gz` / `.zip` | `ci/release-terminal.sh` |
| `uConsole-for-DSPi-VERSION-windows-x86_64-setup.exe`, `…-windows-x86_64.zip`, `…-x86_64.AppImage`, `…-linux-x86_64.tar.gz` | `ci/release-desktop.sh` (`apps/desktop/packaging/build-release.sh`) |
| `uconsole-for-dspi_VERSION_amd64.deb`, `uconsole-for-dspi-VERSION-1.fcN.x86_64.rpm` | the same |
| `…-desktop-source.tar.gz` | the same (`ci/release-source.sh --with-desktop`) |
| `uConsole-for-DSPi-VERSION-macos.dmg` | the Qt app: `apps/desktop/packaging/macos/build-macos.sh` (this name was the Swift app's, whose `apps/apple/package-macos.sh` needs another name when it returns) |
| `uConsole-for-DSPi-VERSION.apk` | `apps/android`: `-Pdspi.sideload=true :app:assembleRelease` |
| `uConsole-for-DSPi-VERSION-web.tar.gz` | the web dist (`apps/web`), packed by hand |

The Qt app's update check counts a release only when it carries its own platform's files under this stem
(`apps/desktop/packaging/README.md`), so the names are a contract.

Publish all of `DIR`, `SHA256SUMS` and `SHA256SUMS.minisig` included. The
release notes say how to check a download
([`release/RELEASE-NOTES-TEMPLATE.md`](release/RELEASE-NOTES-TEMPLATE.md)).

## Signing with minisign

`SHA256SUMS` is signed with [minisign](https://jedisct1.github.io/minisign/), so
one public key checks every file of every release. `ci/minisign.sh` runs
minisign 0.12's official Linux build in the build container, without network;
the build is downloaded once and checked against a pinned SHA-256 (the script's
header says how the pin was verified).

`ci/release-sums.sh` refuses to run without the secret key's path (`--key PATH`
or `DSPI_MINISIGN_KEY`). It never makes a key and never copies one. minisign
asks for the key's password on the terminal, so run it from a terminal.

After signing, the script checks the signature against `docs/release/minisign.pub`.
`ci/release-sums.sh --verify DIR` checks a finished folder again: the signature,
then every file.

### The owner's key: made once, by hand

Only the owner makes the key, on their own machine, and only once. The secret
key never goes into the repository, a build cache or a container image.

1. Make a private folder for it, outside every repository:

   ```sh
   mkdir -p ~/.config/dspi-release && chmod 700 ~/.config/dspi-release
   ```

2. Make the key pair with the pinned minisign, from the repository root. It asks
   twice for a password; choose a strong one and keep it in a password manager:

   ```sh
   ci/minisign.sh --mount ~/.config/dspi-release:/k -- -G -p /k/minisign.pub -s /k/minisign.key
   ```

   A minisign installed some other way does the same:
   `minisign -G -p ~/.config/dspi-release/minisign.pub -s ~/.config/dspi-release/minisign.key`.

3. Protect the secret key, and keep a copy offline (an encrypted USB stick, a
   password manager's file storage). Without it no later release can be signed
   with the same key:

   ```sh
   chmod 600 ~/.config/dspi-release/minisign.key
   ```

4. Publish the public key: commit it as `docs/release/minisign.pub`, and copy
   its second line (the key, starting `RW`) into the release notes template's
   placeholder.

   ```sh
   cp ~/.config/dspi-release/minisign.pub docs/release/minisign.pub
   ```

5. Sign each release with:

   ```sh
   ci/release-sums.sh --key ~/.config/dspi-release/minisign.key dist/VERSION
   ```

If the secret key is ever lost or exposed, make a new pair the same way, commit
the new public key, say so in the next release notes, and do not sign with the
old key again.

## Platform signing

Windows Authenticode and Apple Developer ID signing are separate and wait for
the owner's certificates (PRO1 E5). Until then the Qt `.dmg` is signed ad hoc
(so it runs on Apple silicon) and not notarised; `make-dmg.sh` takes `SIGN_ID`
for a Developer ID, and notarisation (`xcrun notarytool`, then `stapler`) is
still to be added then. The Swift app's own updater already requires
Ed25519-signed zips (`apps/apple`).
