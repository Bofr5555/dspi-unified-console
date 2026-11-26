#!/usr/bin/env bash
# -- FIX016 -- (REVG R1) tools/seed-gradle-cache.sh: give this checkout's Gradle home the
# Robolectric runtime before Gradle runs. tools/ci.sh calls it (on the host, before it goes
# into the container); run it by hand before `tools/dev.sh ./gradlew ...` if you like.
#
# Why: the Robolectric modules resolve org.robolectric:android-all-instrumented (about 150 MB,
# the `robolectricRuntime` configuration, checksum-pinned in gradle/verification-metadata.xml)
# through Gradle, and Maven Central answers that one download with 429 in the CI VM. A fresh
# checkout's Gradle home (dspi-android-target-<hash>, see dev.sh) never gets it, so every
# Robolectric test step fails. Copying the jar's files into a Gradle home that already has
# a cache is not enough: Gradle keeps its own index (modules-2/metadata-*), finds no entry for
# the jar, asks Central again (HEAD, 429) and `--offline` says "No cached version". So this
# REPLACES the checkout's modules-2 with the whole modules-2 of a Gradle home that has the jar,
# index included (the way Gradle documents seeding a cache); anything else the checkout needs
# Gradle then downloads into it as usual.
#
#   - A no-op when the checkout's Gradle home has the jar already (the second run, or when
#     Gradle could download it).
#   - The seeds: DSPI_ANDROID_GRADLE_SEED, podman volumes of other checkouts' Gradle homes,
#     space-separated, tried in order (default dspi-android-target-ebd91d1d11, the VM's
#     seeded one). Each is mounted READ-ONLY; one that does not exist is skipped, never
#     created. DSPI_ANDROID_GRADLE_SEED= (empty) turns seeding off.
#   - The seed's jar must match the sha256 pinned in verification-metadata.xml.
#   - No seed has it: says so and leaves the home as it is (Gradle tries Central). Never fails
#     the caller; never downloads anything itself.
#   - Touches only this checkout's own volume. No network, no devices (as dev.sh).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$here/.." && pwd)"
say() { echo "seed-gradle-cache: $*" >&2; }

# Inside the container there is no podman; dev.sh's volumes are what they are.
if [[ "${DSPI_IN_CONTAINER:-}" == 1 ]] || ! command -v podman >/dev/null 2>&1; then exit 0; fi

seeds="${DSPI_ANDROID_GRADLE_SEED-dspi-android-target-ebd91d1d11}"
[[ -n "$seeds" ]] || exit 0

ver="$(sed -n 's/^robolectric-android-all *= *{.*version *= *"\([^"]*\)".*/\1/p' "$ROOT/gradle/libs.versions.toml" || true)"
jar="android-all-instrumented-$ver.jar"
sha="$(grep -A1 "<artifact name=\"$jar\">" "$ROOT/gradle/verification-metadata.xml" | sed -n 's/.*<sha256 value="\([0-9a-f]*\)".*/\1/p' || true)"
if [[ -z "$ver" || -z "$sha" ]]; then
  say "no android-all version or checksum in the catalog; nothing to do"
  exit 0
fi

# This checkout's volume, as dev.sh names it.
wt_hash="$(printf '%s' "$ROOT" | sha256sum | cut -c1-10)"
target="dspi-android-target-${wt_hash}"

mounts=(-v "$target:/cache/target")
names=()
for s in $seeds; do
  [[ "$s" != "$target" ]] || continue
  if podman volume exists "$s"; then
    mounts+=(-v "$s:/seed/${#names[@]}:ro")
    names+=("$s")
  fi
done

image="$("$here/dev.sh" --print-image)"
podman image exists "$image" || { say "image $image missing (tools/dev.sh --build-image); nothing to do"; exit 0; }

# The same user mapping as dev.sh, so the copy belongs to the build's user.
podman run --rm -i \
  --userns=keep-id:uid=1000,gid=1000 \
  --user 1000:1000 \
  --security-opt no-new-privileges \
  --cap-drop=all \
  --network none \
  "${mounts[@]}" \
  -e VER="$ver" -e JAR="$jar" -e SHA="$sha" -e TARGET="$target" -e NAMES="${names[*]}" \
  "$image" bash -s <<'SH' || say "seeding failed; Gradle tries Maven Central"
set -euo pipefail
say() { echo "seed-gradle-cache: $*" >&2; }
caches=/cache/target/gradle-home/caches
rel=gradle-home/caches/modules-2
jar_in() { compgen -G "$1/$rel/files-2.1/org.robolectric/android-all-instrumented/$VER/*/$JAR" | head -n1 || true; }

if [[ -n "$(jar_in /cache/target)" ]]; then
  say "$TARGET has the Robolectric runtime ($VER); nothing to do"
  exit 0
fi
read -r -a names <<<"$NAMES"
for i in "${!names[@]}"; do
  seed=/seed/$i
  j="$(jar_in "$seed")"
  if [[ -z "$j" ]]; then say "${names[$i]} has no $JAR; skipped"; continue; fi
  if ! echo "$SHA  $j" | sha256sum -c --status -; then say "${names[$i]}: $JAR does not match the pinned sha256; skipped"; continue; fi
  mkdir -p "$caches"
  tmp="$caches/.modules-2.seed" old="$caches/.modules-2.old"
  rm -rf "$tmp" "$old"
  cp -a "$seed/$rel" "$tmp"
  # Another home's lock files mean nothing here (Gradle's advice for a copied cache)
  find "$tmp" -name '*.lock' -type f -delete
  if [[ -e "$caches/modules-2" ]]; then mv "$caches/modules-2" "$old"; fi
  mv "$tmp" "$caches/modules-2"
  rm -rf "$old"
  say "$TARGET: Gradle's modules-2 seeded from ${names[$i]} (the Robolectric runtime $VER, index included)"
  exit 0
done
say "no seed volume has $JAR (${NAMES:-none found}); Gradle downloads it from Maven Central"
SH
