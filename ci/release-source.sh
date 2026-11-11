#!/bin/bash
# The source archive every non-Qt release artifact is published with (WP CLEAN1):
# a `git archive` of the exact commit the release was built from, which the
# notices name (NOTICE.md "Source code", assets/about.json `licence.notices`).
# The Qt desktop app (apps/desktop) is left out while that app is not released;
# a Qt release would get its own archive with it (see NOTICE.md).
#
#   ci/release-source.sh VERSION [OUTDIR] [COMMIT]
#
# Writes OUTDIR/dspi-uconsole-VERSION-source.tar.gz (OUTDIR default: dist) and
# prints its path. COMMIT defaults to HEAD; a dirty work tree is refused when it
# is HEAD, since the archive must be what was built. Runs on the host (git only).
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
version=${1:?usage: release-source.sh VERSION [OUTDIR] [COMMIT]}
out=${2:-dist}
commit=${3:-HEAD}
if [ "$commit" = HEAD ] && [ -n "$(git -C "$here" status --porcelain)" ]; then
  echo "release-source.sh: the work tree has changes; commit them or name a commit" >&2
  exit 1
fi
rev=$(git -C "$here" rev-parse --verify "$commit^{commit}")
name=dspi-uconsole-$version-source
mkdir -p "$out"
git -C "$here" archive --format=tar.gz --prefix="$name/" -o "$(cd "$out" && pwd)/$name.tar.gz" \
  "$rev" -- . ':(exclude)apps/desktop'
echo "$out/$name.tar.gz ($rev)"
