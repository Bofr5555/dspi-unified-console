#!/bin/bash
# One Terminal release archive (WP CLEAN1): the binary under its command name
# (ci/command-name.sh), the README, the licence and notices, the Rust crates'
# licence texts, the udev rule and the hub's service files.
#
#   ci/release-terminal.sh BINARY PLATFORM VERSION [OUTDIR]
#
# BINARY is a release build of `-p dspi-term` (`uconsole` or `uconsole.exe`);
# PLATFORM names the archive, e.g. linux-x86_64, linux-aarch64, macos-universal,
# windows-x86_64 (a .zip for windows-*, a .tar.gz otherwise). The archive's
# NOTICE.md and README name the source archive ci/release-source.sh writes for
# the same VERSION, which the release publishes beside it. Runs on the host.
# -- WP P0E -- Last step of a release: ci/release-sums.sh signs the folder's
# SHA256SUMS (docs/RELEASING.md).
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:?usage: release-terminal.sh BINARY PLATFORM VERSION [OUTDIR]}
platform=${2:?}; version=${3:?}; out=${4:-dist}
cmd=$("$here/ci/command-name.sh")
name=$cmd-$version-$platform
stage=$(mktemp -d "${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/release-terminal.XXXXXX")
trap 'rm -rf "$stage"' EXIT
d=$stage/$name
mkdir -p "$d" "$out"
case $platform in
  windows-*) cp "$bin" "$d/$cmd.exe" ;;
  *) cp "$bin" "$d/$cmd"; chmod 755 "$d/$cmd" ;;
esac
cp "$here/apps/terminal/README.md" "$d/README.md"
cp "$here/LICENSE" "$d/LICENSE"
cp "$here/NOTICE.md" "$d/NOTICE.md"
cp "$here/apps/terminal/notices/rust-crates.txt" "$d/THIRD-PARTY-NOTICES.txt"
cp "$here/xtask/70-dspi.rules" "$d/"
"$here/apps/terminal/packaging/service/render.sh" "$d/service"
cat >"$d/SOURCE.txt" <<TXT
DRAFT notice, pending the owner's approval.

$cmd $version is free software under the GNU General Public License version 3
(LICENSE; NOTICE.md says which parts are under version 3 or later and gives the
additional terms). Its complete source code is published with this release,
beside this archive, as dspi-uconsole-$version-source.tar.gz.
The licence texts of the Rust crates compiled into it are in
THIRD-PARTY-NOTICES.txt (also: $cmd --third-party-licences).
TXT
outabs=$(cd "$out" && pwd)
case $platform in
  windows-*) (cd "$stage" && zip -qr "$outabs/$name.zip" "$name") && echo "$out/$name.zip" ;;
  *) tar -C "$stage" -czf "$outabs/$name.tar.gz" "$name" && echo "$out/$name.tar.gz" ;;
esac
