# -- WP APPIMG -- Sourced by build-release.sh and linux/test-appimage.sh:
#   image <name> <Containerfile> <context> [build args...]
# builds the image once and sets $tag to <name>:<hash>, the hash of the
# Containerfile, its build args and the files it copies, so a changed
# Containerfile is never served by an image built from an older one.
image() {
    local f sum
    sum=$( { cat "$2"; printf '%s\n' "${@:4}"
             for f in $(sed -n 's/^COPY \(.*\) [^ ]*$/\1/p' "$2"); do cat "$3/$f"; done; } | sha256sum | cut -c1-12)
    tag=$1:$sum
    podman image exists "$tag" || podman build -t "$tag" -f "$2" "${@:4}" "$3"
}
