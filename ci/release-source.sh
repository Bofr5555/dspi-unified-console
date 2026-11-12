#!/bin/bash
# The source archive every release artifact is published with (WP CLEAN1): a
# `git archive` of the exact commit the release was built from, which the
# notices name (NOTICE.md "Source code", assets/about.json `licence.notices`).
#
#   ci/release-source.sh [--with-desktop] VERSION [OUTDIR] [COMMIT]
#
# By default the Qt desktop app is left out while it is not released: everything
# under apps/desktop except apps/desktop/packaging/licences/, which the Swift
# app's packaging (apps/apple/package-macos.sh runs stage.sh from there) and the
# swift CI step (gen_rust_notices.py --swift) use, so the Swift artifact's
# archive holds the scripts that build its notices. --with-desktop keeps all of
# apps/desktop, for a release that ships the Qt app; it writes
# uConsole-for-DSPi-VERSION-desktop-source.tar.gz, which the Qt packages' notices
# would then name (fix-up cl1, item 6).
#
# Writes OUTDIR/<name>.tar.gz (OUTDIR default: dist) and prints its path. COMMIT
# defaults to HEAD; a dirty work tree is refused when it is HEAD, since the
# archive must be what was built. Runs on the host (git only).
# -- WP P0E -- Last step of a release: ci/release-sums.sh signs the folder's
# SHA256SUMS (docs/RELEASING.md).
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
desktop=0
if [ "${1:-}" = --with-desktop ]; then desktop=1; shift; fi
version=${1:?usage: release-source.sh [--with-desktop] VERSION [OUTDIR] [COMMIT]}
out=${2:-dist}
commit=${3:-HEAD}
if [ "$commit" = HEAD ] && [ -n "$(git -C "$here" status --porcelain)" ]; then
  echo "release-source.sh: the work tree has changes; commit them or name a commit" >&2
  exit 1
fi
rev=$(git -C "$here" rev-parse --verify "$commit^{commit}")
paths=(.)
if [ "$desktop" = 1 ]; then
  name=uConsole-for-DSPi-$version-desktop-source
else
  name=uConsole-for-DSPi-$version-source
  # Every entry of apps/desktop and apps/desktop/packaging but the shared licences
  while IFS= read -r p; do
    case $p in
      apps/desktop/packaging|apps/desktop/packaging/licences) ;;
      *) paths+=(":(exclude)$p") ;;
    esac
  done < <(git -C "$here" ls-tree --name-only "$rev" apps/desktop/ apps/desktop/packaging/)
fi
mkdir -p "$out"
git -C "$here" archive --format=tar.gz --prefix="$name/" -o "$(cd "$out" && pwd)/$name.tar.gz" \
  "$rev" -- "${paths[@]}"
echo "$out/$name.tar.gz ($rev)"
