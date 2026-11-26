#!/bin/bash
# -- WP INTE -- tools/prune-worktree.sh against a scratch repository with two worktrees, a fake
# podman (a file per volume) and a scratch build cache: the dry run removes nothing, --yes only the
# worktree's own caches (not a longer key's), --work its work dirs, --orphans only volumes no
# worktree has, and another worktree's key is refused.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/prune-test.XXXXXX")
trap 'rm -rf "$t"' EXIT
fail() { echo "test-prune-worktree: FAIL: $*" >&2; exit 1; }
have() { [ -e "$1" ] || fail "$1 is gone"; }
gone() { [ ! -e "$1" ] || fail "$1 is still there"; }

# A fake podman: volumes are files in $t/vols
mkdir -p "$t/bin" "$t/vols"
cat > "$t/bin/podman" <<'EOF'
#!/bin/bash
v=$PODMAN_VOLS
case "$1 $2" in
  "volume ls") ls "$v" ;;
  "volume exists") [ -e "$v/$3" ] ;;
  "volume rm") rm "$v/$3" && echo "$3" ;;
  *) echo "fake podman: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$t/bin/podman"
export PODMAN="$t/bin/podman" PODMAN_VOLS="$t/vols" DSPI_BUILD_CACHE="$t/cache"
vol() { printf 'dspi-android-target-%s' "$(printf '%s' "$1/apps/android" | sha256sum | cut -c1-10)"; }

# The repository: main, a worktree on wp/WEB and one on wp/WEB6
git init -q -b main "$t/repo"
mkdir -p "$t/repo/tools"
cp "$here/prune-worktree.sh" "$t/repo/tools/"
git -C "$t/repo" add tools
git -C "$t/repo" -c user.name=t -c user.email=t@t commit -qm init
git -C "$t/repo" worktree add -q -b wp/WEB "$t/WEB"
git -C "$t/repo" worktree add -q -b wp/WEB6 "$t/WEB6"
P=$t/repo/tools/prune-worktree.sh
C=$t/cache
for k in wp-WEB wp-WEB-cross wp-WEB-qt wp-WEB6 wp-WEB6-cross; do mkdir -p "$C/target/$k"; done
for k in wp-WEB wp-WEB-x wp-WEB6 web/wp-WEB; do mkdir -p "$C/work/$k"; done
for w in "$t/repo" "$t/WEB" "$t/WEB6" "$t/old"; do touch "$t/vols/$(vol "$w")"; done
touch "$t/vols/other-volume"

# Dry run: says, removes nothing
out=$("$P" "$t/WEB")
grep -q "would remove $C/target/wp-WEB-cross" <<<"$out" || fail "dry run does not name wp-WEB-cross: $out"
grep -q "would remove podman volume $(vol "$t/WEB")" <<<"$out" || fail "dry run does not name the volume: $out"
have "$C/target/wp-WEB"; have "$t/vols/$(vol "$t/WEB")"

# --yes: the worktree's own targets and volume; not wp-WEB6's, not the work dirs
"$P" --yes "$t/WEB" >/dev/null
gone "$C/target/wp-WEB"; gone "$C/target/wp-WEB-cross"; gone "$C/target/wp-WEB-qt"; gone "$t/vols/$(vol "$t/WEB")"
have "$C/target/wp-WEB6"; have "$C/target/wp-WEB6-cross"; have "$C/work/wp-WEB"; have "$t/vols/$(vol "$t/WEB6")"

# --work, for a worktree already removed: by its key
git -C "$t/repo" worktree remove "$t/WEB"
"$P" --yes --work --key wp-WEB "$t/WEB" >/dev/null
gone "$C/work/wp-WEB"; gone "$C/work/wp-WEB-x"; gone "$C/work/web/wp-WEB"; have "$C/work/wp-WEB6"

# Another worktree's key is refused
if "$P" --yes --key wp-WEB6 "$t/elsewhere" >/dev/null 2>&1; then fail "wp-WEB6's key was taken"; fi
have "$C/target/wp-WEB6"

# --orphans: only the volume of no worktree ($t/old); never another kind of volume
out=$("$P" --orphans)
grep -q "would remove podman volume $(vol "$t/old")" <<<"$out" || fail "orphan not listed: $out"
grep -q "$(vol "$t/WEB6")" <<<"$out" && fail "a live worktree's volume listed as orphan"
"$P" --orphans --yes >/dev/null
gone "$t/vols/$(vol "$t/old")"; have "$t/vols/$(vol "$t/WEB6")"; have "$t/vols/$(vol "$t/repo")"; have "$t/vols/other-volume"
echo "test-prune-worktree: OK"
