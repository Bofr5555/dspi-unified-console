#!/bin/bash
# Writes the hub's service files with the Terminal's command name (WP CLEAN1):
# @CMD@ in the templates becomes `ci/command-name.sh`'s name, and each file takes
# its installed name. The release archives carry the result (ci/release-terminal.sh).
#
#   apps/terminal/packaging/service/render.sh OUTDIR
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=${1:?usage: render.sh OUTDIR}
cmd=$("$here/../../../../ci/command-name.sh")
mkdir -p "$out"
render() { sed "s/@CMD@/$cmd/g" "$here/$1" >"$out/$2"; }
render hub.service.in "$cmd-hub.service"
render hub-task.xml.in "$cmd-hub-task.xml"
render hub.plist.in "se.bofr.$cmd.hub.plist"
sed "s/@CMD@/$cmd/g" "$here/README.md" >"$out/README-hub-service.md"
