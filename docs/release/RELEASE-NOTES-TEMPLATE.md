# uConsole for DSPi VERSION

Release date: YYYY-MM-DD. One version for every client.

<!-- What changed, per client, as in CHANGELOG-0.13.0.md. -->

## Downloads

<!-- As README.md "Download and install"; drop the rows this release does not carry. -->

| System | File |
|---|---|
| Windows 10/11 (x86_64) | `uConsole-for-DSPi-VERSION-windows-x86_64-setup.exe`, or the portable `uConsole-for-DSPi-VERSION-windows-x86_64.zip` |
| macOS 14.6+ | `uConsole-for-DSPi-VERSION-macos.dmg` (the Swift app) |
| Debian 13, Ubuntu 25.04 or newer | `uconsole-for-dspi_VERSION_amd64.deb` |
| Fedora 43, 44 | `uconsole-for-dspi-VERSION-1.fc43.x86_64.rpm`, `uconsole-for-dspi-VERSION-1.fc44.x86_64.rpm` |
| Other Linux (x86_64, glibc 2.39+) | `uConsole-for-DSPi-VERSION-x86_64.AppImage`, or `uConsole-for-DSPi-VERSION-linux-x86_64.tar.gz` with an installer |
| Arch Linux | the AUR package `uconsole-for-dspi` |
| Android 8+ | Google Play, or `uConsole-for-DSPi-VERSION.apk` |
| Terminal (`uconsole`) | `uConsole-for-DSPi-VERSION-terminal-PLATFORM.tar.gz` / `.zip` |
| Web console | `uConsole-for-DSPi-VERSION-web.tar.gz` |

The Windows files, the AppImage and the tar.gz check this project's releases for
updates when that is switched on (Settings > Updates); the `.deb`, the `.rpm`
and the AUR package are updated by their package managers.

Every file of this release is listed in `SHA256SUMS`, which is signed:
`SHA256SUMS.minisig`. The source code of exactly this release is
`uConsole-for-DSPi-VERSION-source.tar.gz`; the Qt packages' (Windows and Linux)
is `uConsole-for-DSPi-VERSION-desktop-source.tar.gz`. Licence: see `NOTICE.md`.

## Check a download

The public key that signs every release:

```
PUBLIC_KEY_LINE
```

It is also in the repository as
[`docs/release/minisign.pub`](https://github.com/Bofr5555/dspi-unified-console/blob/main/docs/release/minisign.pub).
Compare the two before you trust either.

1. Download `SHA256SUMS`, `SHA256SUMS.minisig` and the files you want into one
   folder.
2. Check the signature with [minisign](https://jedisct1.github.io/minisign/)
   (packaged for most systems: `apt install minisign`, `dnf install minisign`,
   `brew install minisign`, `scoop install minisign`):

   ```sh
   minisign -Vm SHA256SUMS -P PUBLIC_KEY_LINE
   ```

   It must say `Signature and comment signature verified`.
3. Check the files against the signed list:

   - Linux: `sha256sum --ignore-missing -c SHA256SUMS`
   - macOS: `shasum -a 256 --ignore-missing -c SHA256SUMS`
   - Windows (PowerShell): `Get-FileHash .\FILE -Algorithm SHA256`, and compare
     the hash with the file's line in `SHA256SUMS`.

   Each file you downloaded must say `OK`.

A file that fails either check is not this release: do not run it.
