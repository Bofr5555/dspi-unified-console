#!/bin/bash
# The function test (README.md). An agent run costs the agent's account tokens, so it runs only
# when a person asks for it with --agent; nothing in CI does.
#
#   run.sh --agent [--times 3] [--story F] [--confirm ask_risky] [--steps 1,2]   a real agent
#   run.sh --replay [FILE | all] [--story F]                                     no model
#
# Each time is a run of its own, <name>-1, <name>-2, ...; the pass rate is at the end.
# A replay names its story (its "story"); `all` plays every replay in replay/.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
mode=; times=1; name=ft-$(date +%m%d-%H%M%S); args=()
story=; replays=()
while [ $# -gt 0 ]; do
    case "$1" in
        --agent) mode=agent ;;
        --replay)
            mode=replay; replays=("$here/replay/smoke.json")
            if [ -n "${2:-}" ] && [ "${2#--}" = "$2" ]; then
                # -- WP FUNC2a -- every replay, each with its own story
                if [ "$2" = all ]; then replays=("$here"/replay/*.json); else replays=("$2"); fi
                shift
            fi ;;
        --story) story=$2; shift ;;
        --times) times=$2; shift ;;
        --name) name=$2; shift ;;
        *) args+=("$1") ;;
    esac
    shift
done
[ -n "$mode" ] || { sed -n '2,10p' "$0" >&2; exit 2; }
ok=0; all=0
one() {  # name, story, more args
    local n=$1 s=$2; shift 2
    all=$((all + 1))
    python3 "$here/functest.py" run --name "$n" --story "$s" "$@" && ok=$((ok + 1))
}
for i in $(seq "$times"); do
    n=$name; [ "$times" = 1 ] || n=$name-$i
    if [ "$mode" = agent ]; then
        one "$n" "${story:-$here/stories/home-theatre.toml}" "${args[@]}"
        continue
    fi
    for r in "${replays[@]}"; do
        s=$story
        [ -n "$s" ] || s=$here/stories/$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("story", "replay-smoke.toml"))' "$r")
        m=$n; [ "${#replays[@]}" = 1 ] || m=$n-$(basename "$r" .json)
        one "$m" "$s" --replay "$r" "${args[@]}"
    done
done
echo "passed $ok of $all"
[ "$ok" = "$all" ]
