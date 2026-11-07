#!/bin/bash
# Build the images in images.conf from their committed Containerfiles.
#
#   ci/container/build-images.sh             build the ones that are missing
#   ci/container/build-images.sh --all       rebuild all (podman's layer cache still applies)
#   ci/container/build-images.sh --list      print each image and whether it exists
#
# DSPI_IMAGES_ONLY="QTPORT COREMERGE" limits a build to the named images.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
# shellcheck source=images.conf
. "$here/images.conf"
android_tag=$(sed -n 's/^IMAGE_TAG="\(.*\)"$/\1/p' "$repo/apps/android/tools/dev.sh")
if [ "localhost/dspi-android-build:$android_tag" != "$ANDROID" ]; then
  echo "build-images: images.conf has $ANDROID, apps/android/tools/dev.sh has :$android_tag" >&2
  exit 1
fi

mode=${1:-missing}
case "$mode" in missing|--all|--list) ;; *) echo "usage: $0 [--all|--list]" >&2; exit 2 ;; esac

build() { # name image containerfile context [build-arg...]
  local name=$1 image=$2 file=$3 context=$4 start
  shift 4
  case " ${DSPI_IMAGES_ONLY:-$name} " in *" $name "*) ;; *) return 0 ;; esac
  if [ "$mode" = --list ]; then
    if podman image exists "$image"; then echo "present  $image"; else echo "missing  $image"; fi
    return 0
  fi
  if [ "$mode" = missing ] && podman image exists "$image"; then
    echo "== $image: present"
    return 0
  fi
  start=$SECONDS
  echo "== $image: building from $file"
  podman build "$@" -t "$image" -f "$file" "$context"
  echo "== $image: built in $((SECONDS - start)) s"
}

build QTPORT "$QTPORT" "$here/Containerfile.qtport" "$here"
build COREMERGE "$COREMERGE" "$here/Containerfile" "$here" --build-arg "BASE_IMAGE=$QTPORT"
build DENY "$DENY" "$here/Containerfile.deny" "$here" --build-arg "BASE_IMAGE=$COREMERGE"
build ANDROID "$ANDROID" "$repo/apps/android/container/Containerfile" "$repo/apps/android/container"
build CROSS "$CROSS" "$here/Containerfile.cross" "$here" --build-arg "ANDROID_IMAGE=$ANDROID"
