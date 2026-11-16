#!/bin/sh
# The hub's REST mirror from curl, over TLS pinned to the hub's key
# (docs/integrator/hub-api.md, sections 2 and 14; docs/integrator/security.md).
#
# On the hub's machine, once (and again after `uconsole hub tls rotate`):
#   uconsole hub tls fingerprint      # prints the --pinnedpubkey value: sha256//...
#   uconsole hub api-key new "My script" viewer   # the key, shown once
#
#   DSPI_HUB=https://192.168.1.20:11915 DSPI_PIN='sha256//...' DSPI_KEY=... \
#       ./hub_curl.sh E46058388B1A2E2C [g1]
#
# With a group id (hub-api.md section 19) it also sets the group's master volume,
# one row per member (section 20.2). The firmware images and rollouts are read
# only on a hub started with --rollout (section 21; there is no way to start a
# rollout or upload an image over REST).
#
# --pinnedpubkey makes curl refuse any other key before the request (and the
# key in it) is sent. -k only skips the name and date checks of the hub's
# self-signed certificate, which the pin replaces. Without a pin, use the
# exported certificate instead: --cacert hub.pem (it then checks the name too:
# use the hub's .local name or localhost).
set -eu

HUB=${DSPI_HUB:?set DSPI_HUB, e.g. https://hub.local:11915}
PIN=${DSPI_PIN:?set DSPI_PIN to sha256//... from uconsole hub tls fingerprint}
KEY=${DSPI_KEY:?set DSPI_KEY to an API key}
SERIAL=${1:?usage: hub_curl.sh SERIAL [GROUP]}
GROUP=${2:-}

api() {
    curl -sS --fail-with-body -k --pinnedpubkey "$PIN" \
        -H "Authorization: Bearer $KEY" "$@"
}

# The hub's health, and one parameter.
api "$HUB/api/v1/health"
echo
api "$HUB/api/v1/devices/$SERIAL/params/vol.master"
echo
# A write is judged as any client's: the key's role, the installer's limits, the lock.
api -X PUT -H 'Content-Type: application/json' -d '{"value": -20.0}' \
    "$HUB/api/v1/devices/$SERIAL/params/vol.master"
echo

# The fleet (hub-api.md sections 19 to 21, v1): the groups, and with a group id a
# volume on every member of it, 2 dB lower on the first one. Each member is judged
# on its own; the answer has one row per member, and a refused member is its row's
# code, never retried.
api "$HUB/api/v1/groups"
echo
if [ -n "$GROUP" ]; then
    api -X PUT -H 'Content-Type: application/json' \
        -d "{\"value\": -20.0, \"offsets\": {\"$SERIAL\": -2.0}}" \
        "$HUB/api/v1/groups/$GROUP/params/vol.master"
    echo
fi
# Firmware images and rollouts: read only, over TLS only. 501 when the hub was not
# started with --rollout.
api "$HUB/api/v1/firmware" || echo "(no firmware rollout on this hub)"
echo
api "$HUB/api/v1/rollouts" || echo "(no firmware rollout on this hub)"
echo
