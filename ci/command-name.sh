#!/bin/bash
# Prints the Terminal's command name (WP CLEAN1). Its one definition is
# `dspi_proto::command!()` in crates/dspi-proto/src/lib.rs; scripts read it from there.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
name=$(sed -n '/^macro_rules! command {/,/^}/s/^ *"\([a-z0-9-]*\)"$/\1/p' "$here/crates/dspi-proto/src/lib.rs")
[ -n "$name" ] || { echo "command-name.sh: no name in crates/dspi-proto/src/lib.rs" >&2; exit 1; }
echo "$name"
