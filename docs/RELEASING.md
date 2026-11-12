# Releasing

How a release's files are put together and signed. The client-specific steps
(the Qt packages, the Swift `.dmg`, the Android bundle, the web dist) are in each
client's own docs; this page covers what every release shares.

## The files

1. `ci/release-source.sh VERSION DIR`: the source archive of the exact commit.
2. `ci/release-terminal.sh BINARY PLATFORM VERSION DIR`: one per Terminal build.
3. Each client's packages, copied into the same `DIR`.
4. **Last:** `ci/release-sums.sh --key SECRET_KEY DIR` writes `DIR/SHA256SUMS`
   over every file in `DIR` and signs it as `DIR/SHA256SUMS.minisig`.

### The file names (WP RN1)

Every release file starts with the release's stem, `uConsole-for-DSPi-VERSION-`; distribution packages
use the package name `uconsole-for-dspi`, as their systems expect.

| File | Made by |
|---|---|
| `uConsole-for-DSPi-VERSION-source.tar.gz`, `…-desktop-source.tar.gz` | `ci/release-source.sh [--with-desktop]` |
| `uConsole-for-DSPi-VERSION-terminal-PLATFORM.tar.gz` / `.zip` | `ci/release-terminal.sh` |
| `uConsole-for-DSPi-VERSION-windows-x86_64-setup.exe`, `…-windows-x86_64.zip`, `…-x86_64.AppImage`, `…-linux-x86_64.tar.gz` | `apps/desktop/packaging/build-release.sh` |
| `uconsole-for-dspi_VERSION_amd64.deb`, `uconsole-for-dspi-VERSION-1.fcN.x86_64.rpm` | the same |
| `uConsole-for-DSPi-VERSION-macos.dmg` | `apps/apple/package-macos.sh` |
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
the owner's certificates (PRO1 E5). The Swift app's own updater already requires
Ed25519-signed zips (`apps/apple`).
