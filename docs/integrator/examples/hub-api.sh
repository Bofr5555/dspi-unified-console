#!/bin/sh
# The hub API from a shell script, with `uconsole api` (docs/integrator/hub-api.md, section 16).
#
#   HUB="--hub" ./hub-api.sh E46058388B1A2E2C          # the hub found on this network
#   HUB="--link 192.168.1.20" ./hub-api.sh E46058388B1A2E2C
#
# The first run pairs this console with the hub (it asks for the PIN, or give `--pin` in HUB);
# a paired console has the control role. `uconsole api` exits by the hub's answer, so a
# script branches on the status, not on text: 0 ok, 2 not found, 5 denied, 6 offline,
# 7 past the installer's limits, 8 a bad request, 9 busy (try again), 4 no hub.
set -eu

UCONSOLE=${UCONSOLE:-uconsole}
HUB=${HUB:---hub}
SERIAL=${1:?usage: hub-api.sh SERIAL}

# Every device the hub has seen, online or not.
$UCONSOLE $HUB api inventory.list

# Read the master volume: the answer's fields, as JSON.
$UCONSOLE $HUB api param.get "{\"serial\": \"$SERIAL\", \"path\": \"vol.master\"}"

# Turn it to -20 dB, and say why if the hub refused.
rc=0
$UCONSOLE $HUB api param.set "{\"serial\": \"$SERIAL\", \"path\": \"vol.master\", \"value\": -20}" \
    >/dev/null || rc=$?
case $rc in
    0) echo "volume set" ;;
    5) echo "not allowed: this console is a viewer" ;;
    6) echo "$SERIAL is offline: nothing was sent" ;;
    7) echo "past the installer's limit: nothing was sent" ;;
    *) echo "failed ($rc)"; exit "$rc" ;;
esac

# The alarms that are on, as one line of JSON for another program.
$UCONSOLE $HUB api alarm.list '{"state": "active"}' --json
