#!/bin/sh
# The hub's REST mirror from curl, over TLS pinned to the hub's key
# (docs/integrator/hub-api.md, sections 2 and 14; docs/integrator/security.md).
#
# On the hub's machine, once (and again after `uconsole hub tls rotate`):
#   uconsole hub tls fingerprint      # prints the --pinnedpubkey value: sha256//...
#   uconsole hub api-key new "My script" viewer   # the key, shown once
#
#   DSPI_HUB=https://192.168.1.20:11915 DSPI_PIN='sha256//...' DSPI_KEY=... \
#       ./hub_curl.sh E46058388B1A2E2C
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
SERIAL=${1:?usage: hub_curl.sh SERIAL}

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
