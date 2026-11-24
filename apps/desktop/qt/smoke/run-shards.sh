#!/bin/bash
# -- WP CIPAR -- A whole smoke run (run.sh) split across app instances that run
# side by side: the main run's sections dealt into N shards, each its own
# launch of the app on the demo device, and the other launches (demo entry,
# tray, pseudo locale, hardware dry run) as one more. Called by ci/check.sh
# --step qt-offscreen when DSPI_SMOKE_SHARDS is above 1 (the default, 4).
#
#   run-shards.sh <shards> <uconsole-for-dspi-smoke> <output dir under /work>
#
# Runs on the host; each part runs run.sh in a container of its own
# (ci/run.sh), so loopback ports and the fake hub's never meet. Paths are the
# container's. DSPI_SMOKE_FAKEHUB and DSPI_SMOKE_UCONSOLE pass through.
#
# A shard is a stretch of the usual run order, so each section but a shard's
# first follows the section it follows in a single run; the stretches are
# balanced by the last run's section times (<work>/smoke-section-times.txt,
# rewritten by each run; a section not in it counts as 10 s). The verdict is the whole
# run's: every part passes (each with run.sh's own checks, the repeated-warning
# one among them) and every listed section ran in some shard. The warning check
# stays per app instance, as in a single run: dspi-capi's limiter is per process
# (each instance logs a failure's first line once), so summed over the shards
# it would count each such line once per shard. <out> then
# has what a single run leaves (each part's files merged in, index.html for all)
# and log.txt: the combined summary first, then each shard's log. Each part's
# own log and console stay in <out>/shards/.
set -uo pipefail
n=$1; app=$2; out=$3
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
RUN=$root/ci/run.sh
case "$out" in /work/*) ;; *) echo "run-shards.sh: the output dir must be under /work" >&2; exit 2 ;; esac
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
W=$B/work/${DSPI_WORK_KEY:-${branch//\//-}}   # /work, as ci/run.sh mounts it
hout=$W/${out#/work/}
times=$W/smoke-section-times.txt
pass="DSPI_SMOKE_FAKEHUB=${DSPI_SMOKE_FAKEHUB:-} DSPI_SMOKE_UCONSOLE=${DSPI_SMOKE_UCONSOLE:-}"

# The main run's sections, in run order
mapfile -t sections < <("$RUN" "set -e; d=$out-list; rm -rf \$d; mkdir -p \$d/home/.config/uconsole-for-dspi
  printf '[onboarding]\nsetupDone=true\n' > \$d/home/.config/uconsole-for-dspi/uconsole-for-dspi.conf
  HOME=\$d/home XDG_CONFIG_HOME=\$d/home/.config DSPI_SMOKE_OUT=\$d QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
    DSPI_SMOKE_LIST=1 timeout 300 $app --demo 2>/dev/null | sed -n 's/^smoke: section //p'; rm -rf \$d")
[ ${#sections[@]} -gt 0 ] || { echo "run-shards.sh: the app listed no sections" >&2; exit 1; }

declare -A secs
[ -f "$times" ] && while read -r s t; do secs[$s]=$t; done < "$times"
# Cut the run order into n stretches, as even in time as stretches can be (the
# smallest largest stretch: a binary search over it, each try filled greedily).
# Every section but a stretch's first keeps the predecessor it has in a single run.
mapfile -t plan < <(for i in "${!sections[@]}"; do echo "$i ${sections[$i]} ${secs[${sections[$i]}]:-10}"; done |
  awk -v n="$n" '
    { name[NR] = $2; w[NR] = $3; total += $3; if ($3 > lo) lo = $3 }
    function parts(m,   k, sum, i) { k = 1; sum = 0
      for (i = 1; i <= NR; i++) { if (sum + w[i] > m) { k++; sum = 0 } sum += w[i] }
      return k }
    END { hi = total
      while (hi - lo > 0.5) { mid = (lo + hi) / 2; if (parts(mid) <= n) hi = mid; else lo = mid }
      k = 0; sum = 0
      for (i = 1; i <= NR; i++) { if (sum + w[i] > hi) { printf "load %d %.0f\n", k, sum > "/dev/stderr"; k++; sum = 0 }
        sum += w[i]; print k, i, name[i] }
      printf "load %d %.0f\n", k, sum > "/dev/stderr" }' 2>"$W/smoke-shard-plan.txt")
lists=()
for line in "${plan[@]}"; do
  read -r k _ s <<<"$line"
  lists[$k]+="${lists[$k]:+,}$s"
done

rm -rf "$hout"; mkdir -p "$hout/shards"
printf '%s\n' "${sections[@]}" > "$hout/shards/sections.txt"
echo "smoke: ${#sections[@]} sections in $n shards, the other launches beside them; planned loads (s): $(awk '{printf "%s ", $3}' "$W/smoke-shard-plan.txt")"
t0=$SECONDS
pids=(); names=()
for k in $(seq 0 $((n - 1))); do
  [ -n "${lists[$k]:-}" ] || continue
  "$RUN" "$pass DSPI_SMOKE_SECTION=${lists[$k]} DSPI_SMOKE_LAUNCHES=0 apps/desktop/qt/smoke/run.sh $app $out/part-$k" \
    > "$hout/shards/part-$k.console" 2>&1 &
  pids+=($!); names+=("part-$k")
done
"$RUN" "$pass DSPI_SMOKE_MAIN=0 DSPI_SMOKE_LAUNCHES=1 apps/desktop/qt/smoke/run.sh $app $out/launches" \
  > "$hout/shards/launches.console" 2>&1 &
pids+=($!); names+=(launches)
fail=0
for i in "${!pids[@]}"; do
  if wait "${pids[$i]}"; then rc=0; else rc=$?; fi
  echo "== ${names[$i]}: exit $rc$( [ "${names[$i]}" = launches ] || echo ", sections ${lists[${names[$i]#part-}]}")"
  cat "$hout/shards/${names[$i]}.console"
  [ "$rc" = 0 ] || { echo "FAIL smoke part ${names[$i]} exited with $rc" >&2; fail=1; }
done
echo "smoke: all parts done in $((SECONDS - t0)) s"

# Each part's files into <out>, as one run lays them out; its log and home kept apart
for d in "${names[@]}"; do
  p=$hout/$d
  [ -d "$p" ] || continue
  [ ! -f "$p/log.txt" ] || mv "$p/log.txt" "$hout/shards/$d.log"
  rm -rf "$p/home" "$p/index.html"
  cp -a --reflink=auto "$p"/. "$hout"/ && rm -rf "$p"
done

# The combined log and summary, and the checks over the whole run
logs=()
for k in $(seq 0 $((n - 1))); do [ -f "$hout/shards/part-$k.log" ] && logs+=("$hout/shards/part-$k.log"); done
cat "${logs[@]}" > "$hout/shards/main.log"
summary=$(awk '/^smoke: [0-9]+ checks, [0-9]+ failures/ { c += $2; f += $4; q += substr($6, 2); t += $9 }
  END { printf "smoke: %d checks, %d failures (%d QML errors, %d thread warnings), %d shards", c, f, q, t, n }' n="${#logs[@]}" "${logs[@]}")
for s in "${sections[@]}"; do
  grep -qxF "smoke: == $s" "$hout/shards/main.log" || { echo "FAIL section $s ran in no shard" >&2; fail=1; }
done
{
  echo "$summary"
  for f in "${logs[@]}"; do echo "==== $(basename "$f" .log)"; cat "$f"; done
  echo "$summary"
} > "$hout/log.txt"
# The section times, for the next run's plan
grep -hE '^smoke: -- [^ ]+: [0-9.]+ s$' "${logs[@]}" | sed -E 's/^smoke: -- ([^ ]+): ([0-9.]+) s$/\1 \2/' > "$times.new" && mv "$times.new" "$times"
"$here/run.sh" --gallery "$hout" | sed "s|$hout|$out|"
echo "$summary"
[ "$fail" = 0 ] || { echo "smoke: the sharded run FAILED" >&2; exit 1; }
