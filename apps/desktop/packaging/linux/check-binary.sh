#!/usr/bin/env bash
# Checks a Linux build of the app before it is packaged:
#   check-binary.sh <DSPiConsole> <newest glibc it may need, e.g. 2.39>
# - Qt is linked dynamically (LGPL-3: users can swap the Qt libraries), so
#   libQt6Core.so.6 must be a NEEDED entry (a static Qt leaves none).
# - The glibc symbol versions it needs are no newer than the floor the
#   release notes state.
set -euo pipefail
bin=$1; floor=$2
needed=$(readelf -d "$bin" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
grep -qx 'libQt6Core.so.6' <<<"$needed" || { echo "check-binary: $bin does not link Qt 6 dynamically" >&2; exit 1; }
glibc=$(objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n 1)
if [ "$(printf '%s\n%s\n' "$glibc" "$floor" | sort -V | tail -n 1)" != "$floor" ]; then
    echo "check-binary: $bin needs glibc $glibc, newer than $floor" >&2; exit 1
fi
echo "check-binary: Qt dynamic ($(grep -c '^libQt6' <<<"$needed") Qt libraries), glibc $glibc <= $floor"
