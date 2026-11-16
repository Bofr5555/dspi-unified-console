#!/usr/bin/env python3
"""The hub's REST mirror from Python, with nothing but the standard library
(docs/integrator/hub-api.md, sections 14 and 16).

The hub runs with --rest; an Admin mints an API key on the hub's machine
(`uconsole hub api-key new "My script"`, or `api-key new "My script"` at a
running hub's prompt) and the key is shown once. The hub speaks TLS with its
own self-signed key (docs/integrator/security.md): export its certificate on
the hub's machine once, `uconsole hub tls cert > hub.pem`, and trust only it:

    DSPI_HUB=https://hub.local:11915 DSPI_CA=hub.pem DSPI_KEY=... ./hub_rest.py [SERIAL]

It also lists the hub's groups and sets the first group's master volume, one row
per member (hub-api.md sections 19 and 20), and reads the firmware images and
rollouts of a hub started with --rollout (section 21: read only, over TLS only).

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


def call(method, path, body=None, optional=()):
    """One request; the answer's JSON, or exit with the hub's code. A status in
    `optional` answers None instead (a family the hub does not run is 501)."""
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
        if e.code in optional:
            return None
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

    # The fleet (v1): groups, and one volume on every member of the first group.
    # 501: the hub runs without groups (--no-groups).
    groups = (call("GET", "/groups", optional=(501,)) or {}).get("groups", [])
    for g in groups:
        print("group", g["id"], g["name"], "members", ",".join(g.get("members", [])))
    if groups:
        g = groups[0]["id"]
        done = call("PUT", f"/groups/{g}/params/vol.master", {"value": -20.0})
        for row in done["results"]:
            # A refused member is its row's code (offline, limit_exceeded, ...),
            # reported and never retried.
            state = "ok" if row["ok"] else row.get("code", "refused")
            print("group", g, row["serial"], state, row.get("value", ""))

    # Firmware images and rollouts: read only, TLS only (426 over plaintext), and
    # 501 on a hub started without --rollout.
    images = call("GET", "/firmware", optional=(426, 501))
    if images is None:
        print("firmware: not served here (no --rollout, or not over TLS)")
    else:
        for i in images["images"]:
            label = "verified" if i.get("verified") else "NOT VERIFIED"
            print("firmware", i["name"], i.get("platform"), i.get("firmware_version"), label)
        for r in call("GET", "/rollouts")["rollouts"]:
            print("rollout", r["id"], r["target"], r["state"])
        print("firmware images", len(images["images"]))


if __name__ == "__main__":
    main()
