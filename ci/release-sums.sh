#!/bin/bash
# A release's SHA256SUMS, signed with the owner's minisign key (WP P0E, PRO1 E5).
# The last step of a release, after ci/release-source.sh, ci/release-terminal.sh
# and each client's packaging have put their files in one folder.
#
#   ci/release-sums.sh --key SECRET_KEY [--comment TEXT] DIR
#       write DIR/SHA256SUMS over every file in DIR (sorted, `sha256sum`'s
#       format, so `sha256sum -c SHA256SUMS` checks them), then sign it:
#       DIR/SHA256SUMS.minisig. Then verify the signature with the public key
#       (docs/release/minisign.pub, or --pub) when one is there.
#   ci/release-sums.sh --verify [--pub PUBLIC_KEY] DIR
#       check DIR/SHA256SUMS.minisig, then every file in SHA256SUMS.
#
# SECRET_KEY may also come from DSPI_MINISIGN_KEY. Without one this refuses: a
# release is never published with sums nobody signed. The key is the owner's,
# made once by hand (docs/RELEASING.md); this script never makes or copies one.
# minisign asks for the key's password on the terminal (ci/minisign.sh runs it
# in a container, from its pinned official build). Runs on the host.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
usage="usage: release-sums.sh --key SECRET_KEY [--comment TEXT] DIR | --verify [--pub PUBLIC_KEY] DIR"
key=${DSPI_MINISIGN_KEY:-}
pub=$here/docs/release/minisign.pub
pub_given=0
verify=0
comment=
dir=
while [ $# -gt 0 ]; do
  case $1 in
    --key) key=${2:?$usage}; shift 2 ;;
    --pub) pub=${2:?$usage}; pub_given=1; shift 2 ;;
    --comment) comment=${2:?$usage}; shift 2 ;;
    --verify) verify=1; shift ;;
    -*) echo "$usage" >&2; exit 2 ;;
    *) [ -z "$dir" ] || { echo "$usage" >&2; exit 2; }; dir=$1; shift ;;
  esac
done
[ -n "$dir" ] || { echo "$usage" >&2; exit 2; }
[ -d "$dir" ] || { echo "release-sums.sh: no folder $dir" >&2; exit 2; }
dir=$(cd "$dir" && pwd)

verify_sig() {
  local pubfile=$1
  "$here/ci/minisign.sh" --mount "$dir:/dist:ro" --mount "$pubfile:/key/minisign.pub:ro" -- \
    -V -p /key/minisign.pub -m /dist/SHA256SUMS -x /dist/SHA256SUMS.minisig
}

if [ "$verify" = 1 ]; then
  [ -f "$pub" ] || { echo "release-sums.sh: no public key at $pub (--pub FILE)" >&2; exit 2; }
  [ -f "$dir/SHA256SUMS" ] && [ -f "$dir/SHA256SUMS.minisig" ] || {
    echo "release-sums.sh: $dir has no SHA256SUMS and SHA256SUMS.minisig" >&2; exit 1; }
  pub=$(cd "$(dirname "$pub")" && pwd)/$(basename "$pub")
  verify_sig "$pub"
  (cd "$dir" && sha256sum --strict -c SHA256SUMS)
  exit 0
fi

if [ -z "$key" ]; then
  cat >&2 <<TXT
release-sums.sh: no signing key: pass --key PATH or set DSPI_MINISIGN_KEY.
Nothing was written. The key is the owner's; docs/RELEASING.md says how it is
made (once, by hand) and where it is kept.
TXT
  exit 2
fi
[ -f "$key" ] || { echo "release-sums.sh: no key file at $key; nothing was written" >&2; exit 2; }
key=$(cd "$(dirname "$key")" && pwd)/$(basename "$key")

# Every regular file at the top of DIR but the sums themselves. Subfolders are
# refused: a release is a flat list of downloads.
if [ -n "$(find "$dir" -mindepth 1 -not -type f -print -quit)" ]; then
  echo "release-sums.sh: $dir holds something other than files; a release is a flat folder" >&2
  exit 2
fi
files=$(cd "$dir" && find . -maxdepth 1 -type f ! -name SHA256SUMS ! -name 'SHA256SUMS.*' -printf '%P\n' | LC_ALL=C sort)
[ -n "$files" ] || { echo "release-sums.sh: $dir has no files to sum" >&2; exit 2; }
rm -f "$dir/SHA256SUMS" "$dir/SHA256SUMS.minisig"
(cd "$dir" && printf '%s\n' "$files" | while IFS= read -r f; do sha256sum -- "$f"; done) > "$dir/SHA256SUMS.part"
mv "$dir/SHA256SUMS.part" "$dir/SHA256SUMS"
[ -n "$comment" ] || comment="uconsole-for-dspi $(basename "$dir") SHA256SUMS"
"$here/ci/minisign.sh" --mount "$dir:/dist" --mount "$key:/key/minisign.key:ro" -- \
  -S -s /key/minisign.key -m /dist/SHA256SUMS -x /dist/SHA256SUMS.minisig -t "$comment"
echo "$dir/SHA256SUMS"
echo "$dir/SHA256SUMS.minisig"
if [ -f "$pub" ]; then
  pub=$(cd "$(dirname "$pub")" && pwd)/$(basename "$pub")
  verify_sig "$pub"
elif [ "$pub_given" = 1 ]; then
  echo "release-sums.sh: no public key at $pub to check the signature with" >&2
  exit 1
else
  echo "release-sums.sh: not checked: no docs/release/minisign.pub yet (--pub FILE)" >&2
fi
