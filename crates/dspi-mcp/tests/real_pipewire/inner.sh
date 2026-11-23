#!/bin/bash
# WP FEAT3, inside the PipeWire container (run.sh starts it): a headless
# PipeWire + WirePlumber, two null sinks named like a DSPi and a laptop's
# speakers, the laptop the default, the DSPi at 50 %, an app ("Game")
# playing to the DSPi. Then pw-dump to /out and the ignored test against the
# live server. No sound device: null sinks only, nothing reaches a speaker.
set -euo pipefail
export XDG_RUNTIME_DIR=/out/run
rm -rf "$XDG_RUNTIME_DIR"; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap cleanup EXIT
dbus-daemon --session --address="unix:path=$XDG_RUNTIME_DIR/bus" --nofork --nopidfile & pids+=($!)
export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
sleep 0.5
pipewire > /out/pipewire.log 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/pipewire-0" ] && break; sleep 0.1; done
wireplumber > /out/wireplumber.log 2>&1 & pids+=($!)
sleep 2
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=laptop_speakers
  node.description="Laptop speakers" media.class=Audio/Sink object.linger=true
  audio.channels=2 audio.position=[ FL FR ] }' > /dev/null
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=dspi_desk
  node.description="DSPi Desk" media.class=Audio/Sink object.linger=true
  audio.channels=8 audio.rate=48000 audio.position=[ FL FR FC LFE RL RR SL SR ]
  device.serial="WeebLabs_DSPi_E66118604B3B5F2A" }' > /dev/null
sleep 1
id_of() { pw-dump | python3 -I -c 'import json,sys
n=sys.argv[1]
for o in json.load(sys.stdin):
    p=(o.get("info") or {}).get("props") or {}
    if p.get("node.name")==n: print(o["id"]); break' "$1"; }
desk=$(id_of dspi_desk); laptop=$(id_of laptop_speakers)
wpctl set-default "$laptop"
wpctl set-volume "$desk" 0.5
# 60 s of quiet noise as raw s16 stereo, played to the DSPi sink by "Game"
python3 -I -c 'import os,sys; sys.stdout.buffer.write(os.urandom(48000*4*60))' 2>/dev/null \
  | pw-cat --playback --raw --format s16 --rate 48000 --channels 2 --target dspi_desk \
      -P '{ application.name="Game" media.name="Game audio" }' - > /out/pw-cat.log 2>&1 &
pids+=($!)
sleep 2
pw-dump > /out/pw-dump-real.json
wpctl status > /out/wpctl-status.txt || true
/t/test-binary --ignored --exact --nocapture \
  host_audio::pipewire::tests::wp_feat3_real_pipewire_reads_sinks_default_volume_and_a_stream
