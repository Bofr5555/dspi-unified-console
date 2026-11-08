# Packaging: licence texts and the release's sources

<!-- WP QD42 -->

## Licence texts in the packages

Every package carries DSPi Console's licence and, where it bundles libraries, theirs:

| Package | Where | What |
|---|---|---|
| AppImage | `usr/share/doc/dspi-console/` | `NOTICE.txt`, `LICENSES/`: GPL-3.0, LGPL-3.0, `Qt/` (Qt 6.8's licence files and `THIRD-PARTY.txt`), `KWindowSystem/`, `ICU/`, `BUNDLED.txt`, `bundled/<Ubuntu package>/copyright` |
| `-linux-x86_64.tar.gz` | top level | the AppImage's `NOTICE.txt` and `LICENSES/` |
| Windows `.zip` and installer | next to `DSPiConsole.exe` | `NOTICE.txt`, `LICENSES/`: GPL-3.0, LGPL-3.0, `Qt/`, `BUNDLED.txt`, `bundled/<Fedora package>/` (MinGW Qt's, GCC's runtime with its exception, the other DLLs') |
| macOS `.dmg` | `DSPi Console.app/Contents/Resources/` | `NOTICE.txt`, `LICENSES/`: GPL-3.0, LGPL-3.0, `Qt/`, `BUNDLED.txt` |
| `.deb` | `/usr/share/doc/dspi-console/copyright` | the notice; the GPL is Debian's `/usr/share/common-licenses/GPL-3` |
| `.rpm`, a plain `cmake --install` | `/usr/share/licenses/dspi-console/` | `GPL-3.0.txt`, `NOTICE.txt` |

`licences/stage.sh` lays these out for the AppImage (`linux/build-ubuntu.sh`), Windows
(`windows/build-windows.sh`) and macOS (`macos/build-macos.sh`). It lists every library in the package
that is not DSPi Console's own in `LICENSES/BUNDLED.txt` (package, version, origin, source, licence, files)
and fails on a file it cannot account for. The `.deb` and `.rpm` use the distribution's Qt, so they carry
only the GPL and the notice (`licences/NOTICE-distro.txt.in`, installed by `qt/CMakeLists.txt`).

The texts in `licences/` are copied verbatim from their official sources by `licences/update-licences.sh`:
the GPL-3.0 and LGPL-3.0 from www.gnu.org, Qt's `LICENSES/` and third-party notices from Qt's source
archives (`licences/qt-third-party.py`), KWindowSystem's `LICENSES/` and ICU's `LICENSE` from theirs. When a
pinned Qt, KWindowSystem or ICU version changes, run `release-sources.sh` and then `update-licences.sh`,
and commit the result; until then the build fails, as it has no texts for the new version.

The notices' wording is pending the owner's approval.

## The release's sources: `release-sources.sh`

The release carries the source of each library the packages bundle as binaries (the LGPL requires it for
Qt and KWindowSystem). Before publishing a release, after `linux/build-release.sh` and
`windows/build-windows.sh`:

```
apps/desktop/packaging/release-sources.sh dist ~/.cache/dspi-build/work/<branch>/dist-windows
```

It downloads into `~/.cache/dspi-build/work/release-sources/` (or `$RELEASE_SOURCES`), checks every file,
and writes `MANIFEST.tsv` (packages, file, URL, SHA-256, how it was checked):

- `qt/`: Qt's source archive of each module the AppImage and the `.dmg` ship (versions and modules read
  from `linux/qt-<version>.sha256` and `macos/fetch-qt.sh` by `licences/qt-modules.sh`), from
  download.qt.io, against the SHA-256 download.qt.io publishes for each (Qt publishes no signatures).
- `kde/`: KWindowSystem (`Containerfile.ubuntu`'s version), from download.kde.org: the pinned SHA-256, the
  one KDE publishes, and the release signature against KDE's release keyring.
- `icu/`: the ICU version in the AppImage, from the ICU project's GitHub release: `SHASUM512.txt` and its
  signature against ICU's `KEYS`.
- `ubuntu/`: the Ubuntu 24.04 source packages of the AppImage's other libraries (`BUNDLED.txt`), by
  `apt-get source` (the archive's signed index; a version the archive has since replaced comes from
  snapshot.ubuntu.com at the time Launchpad states it was published).
- `fedora/`: the Fedora source RPMs of the Windows DLLs (`BUNDLED.txt`: MinGW Qt, GCC's runtime, GLib,
  ...), by `dnf download --srpm`, signatures checked against Fedora's key.

The AppImage's runtime (AppImage's type2-runtime, the first part of the AppImage file) is listed in
`BUNDLED.txt` with the commit it states; it links in libfuse (LGPL-2.1), musl, squashfuse, libzstd and zlib,
as its `--appimage-help` says. `release-sources.sh` does not fetch its source.

Without dist directories it fetches only Qt and KWindowSystem. Fedora's mirrors keep only current package
versions, so run it soon after the Windows build. It is not run in CI, and nothing is published; attach
the files to the release by hand. The archives are never committed.
