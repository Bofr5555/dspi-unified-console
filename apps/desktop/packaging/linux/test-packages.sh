#!/usr/bin/env bash
# Installs each Linux release file from build-release.sh in a clean container
# of a distribution it targets and starts the app there on the demo device
# for 15 s. Passes when every package installs with only its required
# dependencies (no recommended or weak ones), the app is still running at the
# end and its log shows no QML module, component or image that failed to load.
#   test-packages.sh [dist dir, default dist/ or $DSPI_DIST]
# Needs podman and the network (each container installs its packages).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
dist=$(cd "${1:-${DSPI_DIST:-$repo/dist}}" && pwd)
fails=0

# Run in the container: start the app, judge its log
run_app='
set +e
export QT_FORCE_STDERR_LOGGING=1 HOME=/root
timeout 15 "$@" --demo > /root/app.log 2>&1; code=$?
bad=$(grep -E "is not installed|failed to load component|is not a type|Could not load the Qt platform plugin|cannot open shared object|Error decoding|Unsupported image format" /root/app.log)
if [ "$code" = 124 ] && [ -z "$bad" ]; then echo "app ran for 15 s, no load errors"
else echo "app exit $code; the log:"; cat /root/app.log; exit 1; fi'

t() {  # <name> <image> <script>
    echo "== $1"
    if podman run --rm -v "$dist":/dist:ro,z "$2" bash -c "set -e; $3" >"$dist/test-$1.log" 2>&1; then
        tail -n 1 "$dist/test-$1.log"; echo "== $1: OK"
    else
        tail -n 30 "$dist/test-$1.log"; echo "== $1: FAILED (log $dist/test-$1.log)"; fails=$((fails + 1))
    fi
}

t deb-debian-13 docker.io/library/debian:13 "
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends /dist/dspi-console_*_amd64.deb >/dev/null
    run() { $run_app
    }; QT_QPA_PLATFORM=offscreen run dspi-console"
t rpm-fedora-42 registry.fedoraproject.org/fedora:42 "
    dnf -y -q install --setopt=install_weak_deps=False /dist/dspi-console-*.x86_64.rpm >/dev/null
    run() { $run_app
    }; QT_QPA_PLATFORM=offscreen run dspi-console"
# The AppImage on a system with no Qt at all, on X11 (Xvfb): it brings its own.
# Like every AppImage it uses the host's ALSA, GL, fontconfig and a few more
# (linuxdeploy's excludelist), installed here without recommends
# (not ubuntu:22.04: libraries bundled from 24.04 need glibc 2.38, 22.04 has 2.35)
for img in docker.io/library/ubuntu:24.04 docker.io/library/debian:13 registry.fedoraproject.org/fedora:44; do
    name=appimage-${img##*/}; name=${name//:/-}
    t "$name" "$img" "
        if command -v apt-get >/dev/null; then
            apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends xvfb xauth libgl1 libegl1 libopengl0 libfontconfig1 libfreetype6 libharfbuzz0b \
                libx11-6 libx11-xcb1 libxcb1 libsm6 libice6 libwayland-client0 libcom-err2 libgpg-error0 libasound2t64 >/dev/null
        else dnf -y -q install xorg-x11-server-Xvfb mesa-libGL mesa-libEGL fontconfig alsa-lib libglvnd-opengl >/dev/null; fi
        cp /dist/DSPi-Console-*-x86_64.AppImage /root/app.AppImage && chmod +x /root/app.AppImage
        cd /root && ./app.AppImage --appimage-extract >/dev/null
        grep -q apprun-hooks squashfs-root/AppRun || { echo 'AppRun does not source apprun-hooks'; exit 1; }
        Xvfb :5 -nolisten tcp >/dev/null 2>&1 & export DISPLAY=:5; sleep 1
        run() { $run_app
        }; APPIMAGE_EXTRACT_AND_RUN=1 run /root/app.AppImage"
done
[ "$fails" = 0 ] && echo "test-packages: PASS" || echo "test-packages: $fails failed"
[ "$fails" = 0 ]
