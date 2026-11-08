#!/bin/bash
# WP LK3: the rig's scripted scenarios. Each prints `RESULT ...` lines; summary.py
# turns a log of them into tables.
#
#   scenarios.sh smoke            one short run per client at 1 Mbaud, no shaping (CI)
#   scenarios.sh matrix [REPS]    first meter + 30 s of meters: 115200 and 1 Mbaud x
#                                 none, 50 ms, 50 ms with 5/10/20 % loss x our client
#                                 after LK2, before LK2, upstream's TS client
#   scenarios.sh outage           a 20 s outage while the meters run, each client, both rates
#   scenarios.sh pollload         120 s of meters at 10 and 20 % loss, 1 and 2 clients:
#                                 the bridge's 10 s unwritable close
#   scenarios.sh reread           a bulk re-read every 5 s while the meters run
#
# CLIENTS (default "after before ts") picks clients; BAUDS ("115200 1000000") rates;
# MATRIX_SECONDS (30) the meter time in the matrix. Needs `rig.sh build`, `rig.sh
# build-before` (for "before") and `rig.sh provision` for each rate.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
rig=$here/rig.sh
CLIENTS=${CLIENTS:-after before ts}
BAUDS=${BAUDS:-115200 1000000}
BASE="delay 50ms 20ms"

# The bridge's own log lines since `from`: its 10 s unwritable close and its
# "dropped (session queue full)" reports.
bridge_counts() {
  local run=$1 from=$2 log
  log=$("$rig" log "$run" | tail -n +"$((from + 1))")
  printf 'unwritable_closes=%s queue_drop_reports=%s' \
    "$(grep -c 'has sent nothing for' <<< "$log")" "$(grep -c 'session queue full' <<< "$log")"
}

# client RUN NAME SCENARIO-TAGS [VAR=VALUE...]: one client run, its summaries as RESULT lines.
# The demo device has control surfaces, so our client's cs_fetch is real (about
# 180 round trips): no stand-in chain (LK3_CS_CHAIN, CS_CHAIN here) unless asked.
client() {
  local run=$1 who=$2 tags=$3; shift 3
  set -- "LK3_CS_CHAIN=${CS_CHAIN:-0}" "$@"
  local from out
  from=$("$rig" log "$run" | wc -l)
  case "$who" in
    after) out=$("$rig" rust "$run" LK3_MODE=android "$@") ;;
    before) out=$("$rig" rust-before "$run" LK3_MODE=seq "$@") ;;
    ts) out=$("$rig" ts "$run" "$@") ;;
  esac
  echo "$out" | grep -v summary
  local counts; counts=$(bridge_counts "$run" "$from")
  grep 'LK3 summary' <<< "$out" | sed "s/^.*LK3 summary /RESULT $tags who=$who /; s/\$/ $counts/"
  grep -q 'LK3 summary' <<< "$out" || echo "RESULT $tags who=$who failed=no-summary $counts"
  sleep 3
}

group() { # group RUN BAUD SHAPE: a fresh bridge with this shaping
  "$rig" down "$1"
  "$rig" up "$1" "$2" || return 1
  "$rig" shape "$1" $3
}

case "${1:-}" in
  smoke)
    group smoke 1000000 "" || exit 1
    for who in $CLIENTS; do client smoke "$who" "scenario=smoke baud=1000000 shape=none" LK3_SECONDS=10; done
    "$rig" down smoke
    ;;
  matrix)
    reps=${2:-2}
    for baud in $BAUDS; do
      for shape in "" "$BASE" "$BASE loss 5%" "$BASE loss 10%" "$BASE loss 20%"; do
        group mx "$baud" "$shape" || continue
        for rep in $(seq "$reps"); do
          for who in $CLIENTS; do
            client mx "$who" "scenario=matrix baud=$baud shape=${shape:-none} rep=$rep" \
              LK3_SECONDS="${MATRIX_SECONDS:-30}"
          done
        done
        "$rig" down mx
      done
    done
    ;;
  outage)
    for baud in $BAUDS; do
      group og "$baud" "$BASE" || continue
      for who in $CLIENTS; do
        # 15 s after the client starts, which reaches live meters well before.
        (sleep 15; "$rig" outage og 20) &
        client og "$who" "scenario=outage20 baud=$baud shape=base" LK3_SECONDS=60
        wait
      done
      "$rig" down og
    done
    ;;
  pollload)
    for baud in $BAUDS; do
      for loss in 10 20; do
        group pl "$baud" "$BASE loss $loss%" || continue
        for n in 1 2; do
          for who in $CLIENTS; do
            [ "$who" = before ] && continue
            client pl "$who" "scenario=pollload baud=$baud shape=loss$loss clients=$n" \
              LK3_SECONDS=120 LK3_CLIENTS="$n"
          done
        done
        "$rig" down pl
      done
    done
    ;;
  reread)
    for baud in $BAUDS; do
      for shape in "" "$BASE loss 10%"; do
        group rr "$baud" "$shape" || continue
        for who in $CLIENTS; do
          client rr "$who" "scenario=reread baud=$baud shape=${shape:-none}" LK3_SECONDS=40 LK3_REREAD_S=5
        done
        "$rig" down rr
      done
    done
    ;;
  *) sed -n '2,20p' "$0" >&2; exit 2 ;;
esac
