#!/usr/bin/env bash
# Installs each Linux release file from build-release.sh in a clean container
# of a distribution it targets and starts the app there on the demo device
# for 15 s. Passes when every package installs with only its required
# dependencies (no recommended or weak ones), the app is still running at the
# end and its log shows no QML module, component or image that failed to load.
# -- WP QD42 -- Each carries its licence texts: the .deb Debian's copyright file,
# the .rpm the GPL and the notice in /usr/share/licenses, the AppImage and the
# archive NOTICE.txt and LICENSES/ (Qt's, ICU's, and every library in usr/lib
# listed in BUNDLED.txt). -- WP LIC1 -- and the additional terms (TERMS.txt, in
# the notice), the Rust crates notice, the AppImage runtime's Alpine parts
# (musl, zlib, zstd, mimalloc), and each BUNDLED.txt row's copyright file.
# -- WP QP20 -- The name people see is "uConsole for DSPi"; every package's
# notice labels it GPL-3.0-only (the .rpm's License field too) and carries
# Lucide's licence; the udev rule is installed (-- WP UDEV1 -- 70-dspi.rules,
# byte for byte xtask/70-dspi.rules, the one rule of every component); the archive's install.sh
# installs and removes a per-user copy; only the AppImage (and so the archive)
# has the update feed, never the .deb or the .rpm.
#   test-packages.sh [dist dir, default dist/ or $DSPI_DIST]
# Needs podman and the network (each container installs its packages).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
dist=$(cd "${1:-${DSPI_DIST:-$repo/dist}}" && pwd)
. "$here/test-lib.sh"

t deb-debian-13 docker.io/library/debian:13 "
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends /dist/uconsole-for-dspi_*_amd64.deb >/dev/null
    doc=/usr/share/doc/uconsole-for-dspi
    grep -q /usr/share/common-licenses/GPL-3 \$doc/copyright && test -s /usr/share/common-licenses/GPL-3 \
        && cmp -s \$doc/copyright \$doc/NOTICE.txt \
        || { echo 'licences: no copyright file pointing at the GPL'; exit 1; }
    notice() { $notice_check
    }; notice \$doc/copyright \$doc
    names() { $names_check
    }; names
    run() { $run_app
    }; QT_QPA_PLATFORM=offscreen run uconsole-for-dspi"
# Each Fedora release its own .rpm (Containerfile.fedora)
for fc in 43 44; do
    t "rpm-fedora-$fc" "registry.fedoraproject.org/fedora:$fc" "
        dnf -y -q install --setopt=install_weak_deps=False /dist/uconsole-for-dspi-*.fc$fc.x86_64.rpm >/dev/null
        lic=/usr/share/licenses/uconsole-for-dspi
        grep -q 'GNU GENERAL PUBLIC LICENSE' \$lic/LICENSES/GPL-3.0.txt && [ \"\$(rpm -q --qf '%{LICENSE}' uconsole-for-dspi)\" = GPL-3.0-only ] \
            || { echo 'licences: no GPL text in \$lic, or the License field is not GPL-3.0-only'; exit 1; }
        notice() { $notice_check
        }; notice \$lic/NOTICE.txt \$lic
        names() { $names_check
        }; names
        run() { $run_app
        }; QT_QPA_PLATFORM=offscreen run uconsole-for-dspi"
done
# -- WP QD42 -- The archive carries the AppImage's licence files beside it;
# -- WP QP20 -- and its install.sh installs a per-user copy and removes it
t archive docker.io/library/debian:13 "
    mkdir /root/x && tar -xzf /dist/uConsole-for-DSPi-*-linux-x86_64.tar.gz -C /root/x && cd /root/x/uConsole-for-DSPi-*-linux-x86_64
    for f in NOTICE.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/Lucide/LICENSE \
             install.sh README.txt 70-dspi.rules uconsole-for-dspi.desktop uconsole-for-dspi.svg; do
        test -s \$f || { echo \"archive: no \$f\"; exit 1; }
    done
    ls uConsole-for-DSPi-*-x86_64.AppImage >/dev/null
    echo '$rule_sum  70-dspi.rules' | sha256sum -c --quiet - || { echo 'archive: 70-dspi.rules is not xtask/70-dspi.rules'; exit 1; }
    export HOME='/root/home dir' PATH=\$PATH
    ./install.sh >/dev/null
    e=\"\$HOME/.local/share/applications/uconsole-for-dspi.desktop\"
    test -x \"\$HOME/.local/bin/uconsole-for-dspi\" && grep -qx 'Name=uConsole for DSPi' \"\$e\" \
        && grep -qx 'Exec=\"/root/home dir/.local/bin/uconsole-for-dspi\"' \"\$e\" \
        && test -s \"\$HOME/.local/share/icons/hicolor/scalable/apps/uconsole-for-dspi.svg\" \
        || { echo 'archive: install.sh did not install the app, its entry (Exec quoted) and its icon'; cat \"\$e\"; exit 1; }
    ./install.sh --uninstall >/dev/null
    ! test -e \"\$HOME/.local/bin/uconsole-for-dspi\" && ! test -e \"\$e\" || { echo 'archive: install.sh --uninstall left files'; exit 1; }
    echo 'archive: NOTICE.txt, LICENSES/, install.sh, the rule; install.sh installs and removes a per-user copy'"
# -- WP APPIMG -- The AppImage: test-appimage.sh (Ubuntu 24.04 and 25.04,
# Debian 13, Fedora 44 with no Qt; X11 and offscreen, its About, ldd)
echo "== appimage (test-appimage.sh)"
if "$here/test-appimage.sh" "$dist"; then echo "== appimage: OK"; else echo "== appimage: FAILED"; fails=$((fails + 1)); fi
[ "$fails" = 0 ] && echo "test-packages: PASS" || echo "test-packages: $fails failed"
[ "$fails" = 0 ]
