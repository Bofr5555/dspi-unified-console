#!/bin/bash
# -- WP QD42 -- The source of every library the release packages bundle as
# binaries, for publishing with the release (the LGPL requires Qt's; the rest
# are there so nothing is missing). Downloads only
# from each project's or distribution's own servers, checks every file, and
# writes MANIFEST.tsv (packages, file, URL, SHA-256, how it was checked).
# Not run in CI. Nothing is published.
#
#   apps/desktop/packaging/release-sources.sh [dist dir ...]
#
# Always, from the versions the build scripts pin (licences/qt-modules.sh):
#   qt/        Qt's source archives of the modules the AppImage and the Windows
#              build ship, from download.qt.io, against the SHA-256 it publishes
#              next to each (read from download.qt.io itself, never a mirror).
#              -- WP QP20 -- The Windows build's own source is Fedora's (below);
#              its archives here are what update-licences.sh reads Qt's
#              licence files from. KWindowSystem and the Qt .dmg are gone.
# With dist dirs (build-release.sh's dist/ and build-windows.sh's output), from
# each package's LICENSES/BUNDLED.txt:
#   icu/       ICU (in the AppImage, from Qt's ICU build), from the ICU
#              project's GitHub release: SHASUM512.txt and its signature,
#              against ICU's KEYS (also $ICU_VERSIONS, e.g. "73.2")
#   runtime/   the AppImage runtime (type2-runtime, the commit it states): its
#              tree by git at that commit, and the libfuse and squashfuse
#              archives its build script pins by SHA-256 (libfuse's signature
#              checked against its maintainer's key) (also $RUNTIME_COMMITS);
#              (WP LIC1) the Alpine packages it links statically
#              (licences/appimage-runtime-<commit>/ALPINE.txt): their aports
#              directories by git at the pinned commit, and the upstream archives
#              the APKBUILDs name, against the APKBUILDs' SHA-512 (musl's release
#              signature against musl's key)
#   ubuntu/    the Ubuntu 24.04 source packages of the AppImage's other
#              libraries, by apt-get source (the archive's signed index; a
#              version since replaced from snapshot.ubuntu.com, at the time
#              Launchpad states it was published)
#   fedora/    the Fedora source RPMs of the Windows build's DLLs (MinGW Qt,
#              GCC's runtime, ...), by dnf, signatures checked by rpmkeys
#              against Fedora's keys
# Output: $RELEASE_SOURCES, default ~/.cache/dspi-build/work/release-sources.
# A file already there that still matches is not fetched again. The containers
# (ubuntu:24.04, fedora:44, as the builds use) need podman and the network.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=${RELEASE_SOURCES:-$HOME/.cache/dspi-build/work/release-sources}
mkdir -p "$out"
manifest=$out/MANIFEST.tsv
rows=$out/.rows
: >"$rows"
fail() { echo "release-sources: $*" >&2; exit 1; }
row() {  # <packages> <file under $out> <url> <how checked>
    printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$(sha256sum "$out/$2" | cut -d' ' -f1)" "$4" >>"$rows"
}
# A checksum or signature: only from the named host, no redirect elsewhere
get_from() {  # <host> <url>
    local body eff
    body=$(curl -sSfL --retry 3 --proto-redir =https -w '\n%{url_effective}' "$2") || return 1
    eff=$(tail -n 1 <<<"$body")
    case $eff in "https://$1/"*) ;; *) echo "release-sources: $2 came from $eff, not $1" >&2; return 1 ;; esac
    sed '$d' <<<"$body"
}
fetch() {  # <url> <file under $out> <sha256>: (mirrors allowed; the hash decides)
    local f=$out/$2
    mkdir -p "$(dirname "$f")"
    if [ -f "$f" ] && echo "$3  $f" | sha256sum -c --status - 2>/dev/null; then return 0; fi
    curl -sSfL --retry 4 --retry-all-errors --proto-redir =https -o "$f.part" "$1"
    echo "$3  $f.part" | sha256sum -c --status - || { rm -f "$f.part"; fail "$2 does not match SHA-256 $3"; }
    mv "$f.part" "$f"
}

# ── Qt, per package ──
qt_for() {  # <package kind> <label>
    local line version m base url sum
    line=$("$here/licences/qt-modules.sh" "$1")
    version=${line%% *}
    for m in ${line#* }; do
        url=; sum=
        for base in official_releases archive; do
            u=https://download.qt.io/$base/qt/${version%.*}/$version/submodules/$m-everywhere-src-$version.tar.xz
            if s=$(curl -sSf --max-redirs 0 "$u.sha256" 2>/dev/null); then url=$u; sum=$(cut -c1-64 <<<"$s"); break; fi
        done
        [ -n "$url" ] || fail "download.qt.io has no $m-everywhere-src-$version.tar.xz"
        [[ $sum =~ ^[0-9a-f]{64}$ ]] || fail "no SHA-256 from download.qt.io for $m $version"
        fetch "$url" "qt/$m-everywhere-src-$version.tar.xz" "$sum"
        echo "$2 qt/$m-everywhere-src-$version.tar.xz $url sha256-published-by-download.qt.io"
    done
}
{ qt_for appimage AppImage; qt_for windows Windows; } | while read -r for file url how; do
    row "$for" "$file" "$url" "$how"
done

mkdir -p "$out/keys"
gpg_verify() {  # <key file or dir under $out> <signature under $out> <file under $out>: prints the signer
    podman run --rm --network=none -v "$out":/o:ro,z registry.fedoraproject.org/fedora:44 bash -c '
        set -e; export GNUPGHOME=$(mktemp -d)
        if [ -d "/o/$1" ]; then cat "/o/$1"/*.asc; else cat "/o/$1"; fi | gpg -q --import 2>/dev/null || true
        gpg --status-fd 1 --verify "/o/$2" "/o/$3" 2>/dev/null | sed -n "s/^\[GNUPG:\] VALIDSIG \([0-9A-F]*\) .*/\1/p" | grep .' _ "$1" "$2" "$3"
}

# ── What the packages in the dist dirs list as bundled ──
bundled=$out/.bundled
: >"$bundled"
for d in "$@"; do
    d=$(cd "$d" && pwd)
    for t in "$d"/DSPi-uConsole-*-linux-x86_64.tar.gz; do
        [ -f "$t" ] || continue
        tar -xzOf "$t" --wildcards '*/LICENSES/BUNDLED.txt' | sed 's/^/AppImage\t/' >>"$bundled" \
            || fail "$t has no LICENSES/BUNDLED.txt"
    done
    for b in "$d"/DSPi-uConsole-*-windows-x86_64/LICENSES/BUNDLED.txt; do
        [ -f "$b" ] && sed 's/^/Windows\t/' "$b" >>"$bundled"
    done
done
# BUNDLED.txt: package, version, origin, source, licence, files (tab-separated; # comments)
grep -v -P '^\w+\t#' "$bundled" >"$bundled.rows" || true

# ICU (Qt's build of it in the AppImage)
# (ICU_VERSIONS="73.2 ..." adds versions, for licences/update-licences.sh before a first build)
for v in $({ awk -F'\t' '$4 == "unicode-org/icu" {print $3}' "$bundled.rows"; tr ' ' '\n' <<<"${ICU_VERSIONS:-}"; } | grep . | sort -u); do
    tag=release-${v//./-}; base=icu4c-${v//./_}-src.tgz
    rel=https://github.com/unicode-org/icu/releases/download/$tag
    curl -sSfL --retry 3 -o "$out/icu/SHASUM512.txt" --create-dirs "$rel/SHASUM512.txt"
    curl -sSfL --retry 3 -o "$out/icu/SHASUM512.txt.asc" "$rel/SHASUM512.txt.asc"
    curl -sSfL --retry 3 -o "$out/keys/icu-KEYS" "https://raw.githubusercontent.com/unicode-org/icu/main/KEYS"
    isig=$(gpg_verify keys/icu-KEYS icu/SHASUM512.txt.asc icu/SHASUM512.txt) || fail "ICU $v: SHASUM512.txt's signature does not verify against ICU's KEYS"
    sum=$(awk -v f="$base" '$2 == f || $2 == "*" f {print $1}' "$out/icu/SHASUM512.txt")
    [ -n "$sum" ] || fail "ICU $v: $base is not in SHASUM512.txt"
    f=$out/icu/$base
    if ! { [ -f "$f" ] && echo "$sum  $f" | sha512sum -c --status - 2>/dev/null; }; then
        curl -sSfL --retry 4 -o "$f.part" "$rel/$base"
        echo "$sum  $f.part" | sha512sum -c --status - || { rm -f "$f.part"; fail "$base does not match ICU's SHA-512"; }
        mv "$f.part" "$f"
    fi
    row AppImage "icu/$base" "$rel/$base" "sha512-in-SHASUM512.txt;signature-by-$isig"
done

# The AppImage runtime (AppImage's type2-runtime, at the commit it states): its
# tree at that commit, by git (the commit's hash checks every file), and the
# archives its build script downloads and pins by SHA-256
# (scripts/common/install-dependencies.sh: libfuse, LGPL-2.1, linked in
# statically, with the patch in the tree's patches/libfuse/; squashfuse).
# libfuse's release signature is checked against its maintainer's key.
# (RUNTIME_COMMITS="fedfad5 ..." adds commits, as ICU_VERSIONS does)
libfuse_key=ED31791B2C5C1613AF388B8AD113FCAC3C4E599F   # Nikolaus Rath, libfuse's maintainer, who signs its releases
musl_key=836489290BB6B70F99FFDA0556BCDB593020450F     # -- WP LIC1 -- musl's release key (Rich Felker), https://musl.libc.org/musl.pub
for c in $({ awk -F'\t' '$4 == "github.com/AppImage/type2-runtime" {print $3}' "$bundled.rows"; tr ' ' '\n' <<<"${RUNTIME_COMMITS:-}"; } | grep . | sort -u); do
    full=$(get_from api.github.com "https://api.github.com/repos/AppImage/type2-runtime/commits/$c" \
           | python3 -c 'import json, sys; print(json.load(sys.stdin)["sha"])') || fail "GitHub knows no type2-runtime commit $c"
    case $full in "$c"*) ;; *) fail "type2-runtime commit $c resolves to $full" ;; esac
    git=$out/.git-runtime
    mkdir -p "$out/runtime"
    [ -d "$git" ] || git init -q --bare "$git"
    git -C "$git" cat-file -e "$full^{commit}" 2>/dev/null \
        || git -C "$git" fetch -q --depth 1 https://github.com/AppImage/type2-runtime.git "$full"
    [ "$(git -C "$git" rev-parse "$full^{commit}")" = "$full" ] || fail "git fetched no commit $full"
    tarball=runtime/type2-runtime-$full.tar.gz
    git -C "$git" archive --format=tar --prefix="type2-runtime-$full/" "$full" | gzip -n >"$out/$tarball"
    row AppImage "$tarball" "https://github.com/AppImage/type2-runtime/tree/$full" "git-commit-$full"
    deps=$(git -C "$git" show "$full:scripts/common/install-dependencies.sh")
    for pin in $(sed -n 's/^echo "\([0-9a-f]\{64\}\)  \([^"]*\)" | sha256sum -c -$/\1:\2/p' <<<"$deps"); do
        sum=${pin%%:*}; file=${pin#*:}
        url=$(sed -n 's/^wget "\{0,1\}\([^" ]*\)"\{0,1\}$/\1/p' <<<"$deps" | awk -v f="$file" '{n = split($0, a, "/"); if (a[n] == f) print}' | head -n 1)
        [ -n "$url" ] || fail "type2-runtime $c: no download for $file in its install-dependencies.sh"
        local_name=$file
        case $url in https://github.com/vasi/squashfuse/*) local_name=squashfuse-$file ;; esac
        fetch "$url" "runtime/$local_name" "$sum"
        how="sha256-pinned-in-type2-runtime-$c"
        case $url in https://github.com/libfuse/libfuse/releases/download/*)
            curl -sSfL --retry 3 -o "$out/runtime/$local_name.asc" "$url.asc" || fail "no signature for $url"
            mkdir -p "$out/keys"
            get_from keyserver.ubuntu.com "https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x$libfuse_key" >"$out/keys/libfuse.asc" \
                || fail "cannot read libfuse's signing key"
            lsig=$(gpg_verify keys/libfuse.asc "runtime/$local_name.asc" "runtime/$local_name") || fail "$file: the signature does not verify"
            [ "$lsig" = "$libfuse_key" ] || fail "$file is signed by $lsig, not libfuse's maintainer"
            how="$how;signature-by-$lsig"
            row AppImage "runtime/$local_name.asc" "$url.asc" "signature" ;;
        esac
        row AppImage "runtime/$local_name" "$url" "$how"
    done
    # -- WP LIC1 -- The Alpine packages the runtime links statically (musl, zlib,
    # zstd, mimalloc), as licences/appimage-runtime-<commit>/ALPINE.txt pins them:
    # each package's aports directory at the pinned commit, by git (the commit's
    # hash checks every file), and the upstream archive its APKBUILD names,
    # against the APKBUILD's SHA-512 (from upstream, else Alpine's distfiles
    # mirror). The APKBUILD is a shell script: it is read only in a container
    # with no network. musl's release signature is checked against musl's key.
    alpine=$here/licences/appimage-runtime-${full:0:7}/ALPINE.txt
    [ -f "$alpine" ] || continue
    ag=$out/.git-aports
    [ -d "$ag" ] || git init -q --bare "$ag"
    while IFS=$'\t' read -r apkg aver apath acommit aarchive _; do
        case $apkg in '#'*|'') continue ;; esac
        git -C "$ag" cat-file -e "$acommit^{commit}" 2>/dev/null \
            || git -C "$ag" fetch -q --depth 1 --filter=blob:none https://gitlab.alpinelinux.org/alpine/aports.git "$acommit"
        [ "$(git -C "$ag" rev-parse "$acommit^{commit}")" = "$acommit" ] || fail "git fetched no aports commit $acommit"
        ad=$out/.alpine/$apkg
        rm -rf "$ad"; mkdir -p "$ad"
        git -C "$ag" show "$acommit:$apath/APKBUILD" >"$ad/APKBUILD" || fail "aports $acommit has no $apath/APKBUILD"
        apk_info=$(podman run --rm --network=none -v "$ad":/a:ro,z docker.io/library/alpine:3.21 sh -c '
            set -e; cd /a; CARCH=x86_64; CBUILD=x86_64-alpine-linux-musl; CHOST=$CBUILD; CTARGET=$CBUILD
            srcdir=/nonexistent/src; pkgdir=/nonexistent/pkg; startdir=/a
            . ./APKBUILD
            echo "V $pkgver-r$pkgrel"
            for s in $source; do echo "S $s"; done
            echo "$sha512sums" | sed -n "s/^\([0-9a-f]\{128\}\)  *\(.*\)\$/H \1 \2/p"') \
            || fail "cannot read $apath/APKBUILD at $acommit"
        [ "$(sed -n 's/^V //p' <<<"$apk_info")" = "$aver" ] || fail "$apath/APKBUILD at $acommit is not $apkg $aver"
        tarball=runtime/alpine-$apkg-$aver-aports.tar.gz
        git -C "$ag" archive --format=tar --prefix="aports-$acommit/" "$acommit" "$apath" | gzip -n >"$out/$tarball"
        row AppImage "$tarball" "https://gitlab.alpinelinux.org/alpine/aports/-/tree/$acommit/$apath" "git-commit-$acommit"
        found=
        while read -r _ src; do
            case $src in *://*) ;; *) continue ;; esac
            name=${src%%::*}; aurl=${src#*::}
            [ "$name" != "$src" ] || name=${src##*/}
            sum=$(awk -v f="$name" '$1 == "H" && $3 == f {print $2}' <<<"$apk_info")
            [ -n "$sum" ] || fail "$apath/APKBUILD gives no SHA-512 for $name"
            f=$out/runtime/$name
            if ! { [ -f "$f" ] && echo "$sum  $f" | sha512sum -c --status - 2>/dev/null; }; then
                got=
                for u in "$aurl" "https://distfiles.alpinelinux.org/distfiles/v3.21/$name"; do
                    curl -sSfL --retry 4 --retry-all-errors --proto-redir =https -o "$f.part" "$u" 2>/dev/null \
                        && echo "$sum  $f.part" | sha512sum -c --status - && { got=$u; break; }
                    rm -f "$f.part"
                done
                [ -n "$got" ] || fail "$name: neither $aurl nor Alpine's distfiles match the APKBUILD's SHA-512"
                mv "$f.part" "$f"
            fi
            how="sha512-in-aports-$acommit-$apath/APKBUILD"
            case $aurl in https://musl.libc.org/releases/*)
                get_from musl.libc.org "$aurl.asc" >"$f.asc" || fail "no signature for $aurl"
                mkdir -p "$out/keys"
                get_from musl.libc.org https://musl.libc.org/musl.pub >"$out/keys/musl.pub" || fail "cannot read musl's signing key"
                msig=$(gpg_verify keys/musl.pub "runtime/$name.asc" "runtime/$name") || fail "$name: the signature does not verify"
                [ "$msig" = "$musl_key" ] || fail "$name is signed by $msig, not musl's release key"
                how="$how;signature-by-$msig"
                row AppImage "runtime/$name.asc" "$aurl.asc" "signature" ;;
            esac
            row AppImage "runtime/$name" "$aurl" "$how"
            [ "$name" = "$aarchive" ] && found=1
        done < <(grep '^S ' <<<"$apk_info")
        [ -n "$found" ] || fail "$apath/APKBUILD at $acommit does not name $aarchive"
    done <"$alpine"
    rm -rf "$out/.alpine"
done

# Ubuntu's source packages (AppImage). A version the archive has since replaced
# comes from snapshot.ubuntu.com (the archive as it was, its index signed as
# then), at a time Launchpad states it was published and not yet superseded
ub=$(awk -F'\t' '$4 == "ubuntu-24.04" {print $5}' "$bundled.rows" | sort -u)
if [ -n "$ub" ]; then
    mkdir -p "$out/ubuntu"
    # src=version@snapshot time
    items=$(python3 -I -c '
import datetime, json, sys, urllib.parse, urllib.request
for sv in sys.argv[1:]:
    src, ver = sv.split("=", 1)
    q = urllib.parse.urlencode({"ws.op": "getPublishedSources", "source_name": src, "version": ver, "exact_match": "true",
                                "distro_series": "https://api.launchpad.net/1.0/ubuntu/noble"})
    with urllib.request.urlopen("https://api.launchpad.net/1.0/ubuntu/+archive/primary?" + q, timeout=60) as r:
        pubs = [e for e in json.load(r)["entries"] if e["pocket"] in ("Release", "Updates", "Security") and e["date_published"]]
    if not pubs:
        sys.exit(f"release-sources: Launchpad has no publication of {src} {ver} in noble")
    t = lambda s: datetime.datetime.fromisoformat(s)
    p = min(pubs, key=lambda e: t(e["date_published"]))
    ts = t(p["date_published"]) + datetime.timedelta(days=1)
    sup = [t(e["date_superseded"]) for e in pubs if e["date_superseded"]]
    if sup and len(sup) == len(pubs):
        ts = min(ts, max(sup) - datetime.timedelta(hours=1))
    print(f"{sv}@{ts.astimezone(datetime.timezone.utc):%Y%m%dT%H%M%SZ}")
' $ub) || fail "cannot read the Ubuntu packages' publication times from Launchpad"
    podman run --rm -v "$out/ubuntu":/o:z docker.io/library/ubuntu:24.04 bash -c '
        set -e
        sed -i "s/^Types: deb$/Types: deb deb-src/" /etc/apt/sources.list.d/ubuntu.sources
        apt-get update -qq && apt-get install -y -qq ca-certificates >/dev/null
        # An empty folder: apt-get leaves out the URIs of files already there
        rm -rf /o/.get && mkdir /o/.get && cd /o/.get && : >/o/.uris
        get() { apt-get source -qq --download-only --print-uris "$@" | sed -n "s/^.\(http[^ ]*\). \([^ ]*\) .*/\1 \2/p" >>/o/.uris
                apt-get source -qq --download-only "$@" >/dev/null; }
        now=; old=
        for i in "$@"; do
            sv=${i%@*}
            if apt-cache showsrc "${sv%%=*}" | grep -qx "Version: ${sv#*=}"; then now="$now $sv"; else old="$old $i"; fi
        done
        [ -z "$now" ] || get $now
        for ts in $(tr " " "\n" <<<"$old" | sed -n "s/.*@//p" | sort -u); do
            printf "Types: deb-src\nURIs: https://snapshot.ubuntu.com/ubuntu/%s\nSuites: noble noble-updates noble-security\nComponents: main restricted universe multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n" "$ts" \
                >/etc/apt/sources.list.d/snapshot.sources
            apt-get update -qq
            get $(tr " " "\n" <<<"$old" | sed -n "s/@$ts\$//p")
        done
        mv -f /o/.get/* /o/ && rmdir /o/.get' _ $items \
        || fail "apt-get source failed for an Ubuntu package"
    sort -u "$out/ubuntu/.uris" | while read -r url file; do
        row AppImage "ubuntu/$file" "$url" "apt-get-source:ubuntu-archive-signed-index"
    done
    rm -f "$out/ubuntu/.uris"
fi

# Fedora's source RPMs (Windows)
fd=$(awk -F'\t' '$4 == "fedora-44" {print $2 "-" $3}' "$bundled.rows" | sort -u)
if [ -n "$fd" ]; then
    mkdir -p "$out/fedora"
    podman run --rm -v "$out/fedora":/o:z registry.fedoraproject.org/fedora:44 bash -c '
        set -e
        dnf -q download --srpm --url "$@" 2>/dev/null | grep "\.src\.rpm$" | sort -u >/o/.urls
        cd /o && dnf -q download --srpm "$@" >/dev/null
        rpmkeys --import /etc/pki/rpm-gpg/RPM-GPG-KEY-fedora-44-primary
        for f in $(sed "s|.*/||" /o/.urls); do
            rpmkeys --checksig "/o/$f" | grep -q "digests signatures OK" || { echo "$f: signature not OK" >&2; exit 1; }
        done' _ $fd || fail "dnf download --srpm or its signature check failed"
    while read -r url; do
        row Windows "fedora/${url##*/}" "$url" "rpm-signature:fedora-44-key"
    done <"$out/fedora/.urls"
    rm -f "$out/fedora/.urls"
fi

{
    printf '# Sources for DSPi uConsole release packages; made by apps/desktop/packaging/release-sources.sh on %s\n' "$(date -u +%Y-%m-%d)"
    printf '# packages\tfile\turl\tsha256\tchecked by\n'
    sort -k2,2 "$rows"
} >"$manifest"
rm -f "$rows" "$bundled" "$bundled.rows"
echo "release-sources: $(grep -vc '^#' "$manifest") files in $out, listed in $manifest"
