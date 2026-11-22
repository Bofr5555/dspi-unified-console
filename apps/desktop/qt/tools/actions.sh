#!/bin/bash
# -- WP LAY3b -- Write assets/actions.json, the actions as the web and the
# Terminal read them, from qml/main/MenuTable.qml, with the smoke build of the
# app (which evaluates the table). The file is never edited by hand; the smoke
# section lay3b-actions (run.sh) fails when it is not current.
#
#   ci/run.sh 'apps/desktop/qt/tools/actions.sh [<uconsole-for-dspi-smoke>]'
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
app=${1:-/work/build-desktop-qt-smoke/uconsole-for-dspi-smoke}
out=/work/actions-out
# run.sh's comparison fails while the file is old: its exit code says nothing here
DSPI_SMOKE_SECTION=lay3b-actions "$here/../smoke/run.sh" "$app" "$out" > "$out.log" 2>&1 || true
grep -q '^smoke: [0-9]* checks, 0 failures' "$out/log.txt" || { cat "$out.log" >&2; exit 1; }
cp "$out/actions.json" "$here/../../../../assets/actions.json"
echo "assets/actions.json written"
