#!/usr/bin/env python3
"""The hub's REST mirror from Python, with nothing but the standard library
(docs/integrator/hub-api.md, sections 14 and 16).

The hub runs with --rest; an Admin mints an API key on the hub's machine
(`uconsole hub api-key new "My script"`, or `api-key new "My script"` at a
running hub's prompt) and the key is shown once. The hub speaks TLS with its
own self-signed key (docs/integrator/security.md): export its certificate on
the hub's machine once, `uconsole hub tls cert > hub.pem`, and trust only it:

    DSPI_HUB=https://hub.local:11915 DSPI_CA=hub.pem DSPI_KEY=... ./hub_rest.py [SERIAL]

With DSPI_CA the hub's own certificate is the only one trusted, so only the
key it was issued for passes; the name in DSPI_HUB need not be in it (an
address works too). Export it again after `uconsole hub tls rotate`.
A plain http:// URL works only with a hub started with --tls off or
--upstream-compat, and then the key crosses the network in plain text.
(With the requests library: requests.get(url, headers=..., verify="hub.pem"),
which also checks the name, so use the hub's .local name.)
"""

import json
import os
import ssl
import sys
import urllib.error
import urllib.request

HUB = os.environ.get("DSPI_HUB", "https://hub.local:11915").rstrip("/")
KEY = os.environ.get("DSPI_KEY", "")
CA = os.environ.get("DSPI_CA", "")


def tls_context():
    """The hub's own certificate as the only trust anchor, or the system's roots."""
    if not CA:
        return None
    ctx = ssl.create_default_context(cafile=CA)
    # Only the hub's own self-signed certificate is trusted, so the key is what is
    # checked; the hub's certificate names its .local name, not every address.
    ctx.check_hostname = False
    return ctx


def call(method, path, body=None):
    """One request; the answer's JSON, or exit with the hub's code."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        HUB + "/api/v1" + path,
        data=data,
        method=method,
        headers={"Authorization": "Bearer " + KEY, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=10, context=tls_context()) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        try:
            err = json.load(e)
        except ValueError:
            err = {}
        sys.exit(f"{method} {path}: HTTP {e.code} {err.get('code', '')}: {err.get('msg', '')}")
    except urllib.error.URLError as e:
        # A certificate that is not the hub's (DSPI_CA) ends here, before the key is sent.
        sys.exit(f"{method} {path}: {e.reason}")


def main():
    if not KEY:
        sys.exit("set DSPI_KEY to an API key (uconsole hub api-key new <name>)")
    devices = call("GET", "/devices")["devices"]
    for d in devices:
        print(d["serial"], "online" if d["online"] else "offline", d.get("name") or "")
    online = [d["serial"] for d in devices if d["online"]]
    serial = sys.argv[1] if len(sys.argv) > 1 else (online or [None])[0]
    if serial is None:
        sys.exit("no device online")

    volume = call("GET", f"/devices/{serial}/params/vol.master")["value"]
    print("master volume", volume, "dB")
    # A write is judged as any client's: the key's role, the installer's limits, the lock.
    done = call("PUT", f"/devices/{serial}/params/vol.master", {"value": -20.0})
    print("master volume now", done["value"], "dB")

    for alarm in call("GET", "/alarms?state=active")["alarms"]:
        print("alarm", alarm["code"], alarm.get("subject", ""), alarm["message"])
    for h in call("GET", "/health")["devices"]:
        print("health", h.get("serial"), "temperature", h.get("temp_c"), "cpu", h.get("cpu_pct"))


if __name__ == "__main__":
    main()
