#!/usr/bin/env bash
# Builds every Linux release file into dist/ (or $DSPI_DIST), against Qt 6:
#   DSPi-uConsole-v<version>-x86_64.AppImage       any distribution (glibc 2.39+: Ubuntu 24.04+)
#   DSPi-uConsole-v<version>-linux-x86_64.tar.gz   the AppImage, udev rule and installer
#   dspi-uconsole_<version>_amd64.deb              Debian 13+, Ubuntu 25.04+
#   dspi-uconsole-<version>-1.x86_64.rpm           Fedora 42+
# Each is built in a podman container of the oldest distribution it targets:
# the AppImage on Ubuntu 24.04 with the official Qt 6.8 LTS (checked against
# download.qt.io's SHA-256, see fetch-qt.sh), the packages on the
# distributions' own Qt 6.
# Qt stays dynamic in all of them (LGPL-3): each build fails if the app links
# Qt statically. Nothing is published.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)    # the dspi-unified-console root
version=v$(grep -oP 'setApplicationVersion\("\K[^"]+' "$repo/apps/desktop/qt/src/main.cpp")
dist=${DSPI_DIST:-$repo/dist}
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}

tools="$dist/tools"
mkdir -p "$tools" "$B/cargo-home"
# linuxdeploy's continuous builds and the AppImage runtime, through GitHub's API
# (which also states each file's SHA-256, checked here); a file already in
# dist/tools is reused
fetch_tool() {  # <GitHub owner/repository> <file>
    local json url sum
    json=$(curl -sSf --retry 4 --retry-all-errors "https://api.github.com/repos/$1/releases/tags/continuous")
    read -r url sum < <(python3 -c 'import json, sys
a = [a for a in json.load(sys.stdin)["assets"] if a["name"] == sys.argv[1]][0]
print(a["url"], (a.get("digest") or "none:").split(":", 1)[1] or "none")' "$2" <<<"$json")
    curl -sSfL --retry 4 --retry-all-errors -H 'Accept: application/octet-stream' -o "$tools/$2.part" "$url"
    if [ "$sum" != none ]; then
        echo "$sum  $tools/$2.part" | sha256sum -c --quiet - || { rm -f "$tools/$2.part"; return 1; }
    else
        echo "build-release: GitHub states no SHA-256 for $2; not checked" >&2
    fi
    chmod +x "$tools/$2.part" && mv "$tools/$2.part" "$tools/$2"
}
for t in linuxdeploy-x86_64.AppImage linuxdeploy-plugin-qt-x86_64.AppImage linuxdeploy-plugin-appimage-x86_64.AppImage; do
    [ -x "$tools/$t" ] || fetch_tool "linuxdeploy/${t%-x86_64.AppImage}" "$t"
done
[ -x "$tools/runtime-x86_64" ] || fetch_tool AppImage/type2-runtime runtime-x86_64
rm -f "$dist"/*.AppImage "$dist"/*.tar.gz "$dist"/*.deb "$dist"/*.rpm

# The shared crate cache (as ci/run.sh); cargo's jobs capped as ci/run.sh does
run=(podman run --rm -v "$repo":/src:ro,z -v "$dist":/dist:z -v "$B/cargo-home":/cache/cargo:z
     -e CARGO_HOME=/cache/cargo -e CARGO_BUILD_JOBS="${DSPI_JOBS:-4}" -e JOBS="${DSPI_JOBS:-4}")
podman build -t dspi-console-build-ubuntu -f "$here/Containerfile.ubuntu" "$here"
"${run[@]}" -e VERSION="$version" dspi-console-build-ubuntu /src/apps/desktop/packaging/linux/build-ubuntu.sh
podman build -t dspi-console-build-debian -f "$here/Containerfile.debian" "$here"
"${run[@]}" dspi-console-build-debian /src/apps/desktop/packaging/linux/build-debian.sh
podman build -t dspi-console-build-fedora -f "$here/Containerfile.fedora" "$here"
"${run[@]}" dspi-console-build-fedora /src/apps/desktop/packaging/linux/build-fedora.sh
ls -l "$dist"/*.AppImage "$dist"/*.tar.gz "$dist"/*.deb "$dist"/*.rpm
