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
