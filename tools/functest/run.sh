#!/bin/bash
# The function test (README.md). An agent run costs the agent's account tokens, so it runs only
# when a person asks for it with --agent; nothing in CI does.
#
#   run.sh --agent [--times 3] [--story F] [--confirm ask_risky] [--steps 1,2]   a real agent
#   run.sh --replay [FILE] [--story F]                                           no model
#
# Each time is a run of its own, <name>-1, <name>-2, ...; the pass rate is at the end.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
mode=; times=1; name=ft-$(date +%m%d-%H%M%S); args=()
story=$here/stories/home-theatre.toml; replay=
while [ $# -gt 0 ]; do
    case "$1" in
        --agent) mode=agent ;;
        --replay)
            mode=replay; replay=$here/replay/smoke.json; story=$here/stories/replay-smoke.toml
            if [ -n "${2:-}" ] && [ "${2#--}" = "$2" ]; then replay=$2; shift; fi ;;
        --story) story=$2; shift ;;
        --times) times=$2; shift ;;
        --name) name=$2; shift ;;
        *) args+=("$1") ;;
    esac
    shift
done
[ -n "$mode" ] || { sed -n '2,10p' "$0" >&2; exit 2; }
[ "$mode" = replay ] && args+=(--replay "$replay")
ok=0
for i in $(seq "$times"); do
    n=$name; [ "$times" = 1 ] || n=$name-$i
    python3 "$here/functest.py" run --name "$n" --story "$story" "${args[@]}" && ok=$((ok + 1))
done
echo "passed $ok of $times"
[ "$ok" = "$times" ]
