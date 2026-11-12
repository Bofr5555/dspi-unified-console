# DSPi uConsole VERSION

Release date: YYYY-MM-DD. One version for every client.

<!-- What changed, per client, as in CHANGELOG-0.13.0.md. -->

## Downloads

<!-- The table of files per system, as in README.md "Download and install". -->

Every file of this release is listed in `SHA256SUMS`, which is signed:
`SHA256SUMS.minisig`. The source code of exactly this release is
`dspi-uconsole-VERSION-source.tar.gz`. Licence: see `NOTICE.md`.

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
