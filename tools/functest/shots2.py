#!/usr/bin/env python3
"""-- WP SHOW2 -- shots2.py RUN [dark|light ...]: the "after" screens SHOWB's "before" set pairs with that gallery.py
does not take, on an app `FUNCTEST_SCREEN=1600x1000 functest.py up --name RUN` left running: Measure refusing at
0 dB volume, a measurement by the user (running, Results), Auto-tune's layout and proposal, History and Events after
it. Writes runs/RUN/shots2/<set>/<name>-<window>-N.png and shots2/manifest.json."""
import json
import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from functest import CONTAINER_WORK, Run  # noqa: E402

run = Run(sys.argv[1])
ctl = run.ctl
SHOTS = run.dir / "shots2"
SHOTS.mkdir(exist_ok=True)
MANIFEST = SHOTS / "manifest.json"
manifest = []


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def shot(theme, name, caption, settle_s=1.0):
    time.sleep(settle_s)
    (SHOTS / theme).mkdir(exist_ok=True)
    files = ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{run.name}/shots2/{theme}", prefix=name + "-")
    manifest.append({"set": theme, "name": name, "caption": caption,
                     "files": [str(SHOTS / theme / pathlib.Path(f).name) for f in files]})
    MANIFEST.write_text(json.dumps(manifest, indent=1))
    log(theme, name)


def page(section, tab=""):
    ctl.send(op="page", section=section, tab=tab)
    time.sleep(0.6)


def wait(what, test, s=120):
    ctl.wait(what, test, s)


for d in ctl.get("DeviceRegistry", "devices"):
    if d.get("title") == "Centre" or d.get("name") == "Centre":
        ctl.call("DeviceRegistry", "activate", d["id"])
time.sleep(1)
themes = sys.argv[2:] or ["dark", "light"]
for i, theme in enumerate(themes):
    ctl.set("LayoutController", "theme", theme)
    time.sleep(1.2)
    # Measure refuses at 0 dB: the preflight's line, and what Measure says when pressed
    ctl.call("SidebarController", "setUserVolume", 0.0)
    page("measure", "measure")
    ctl.set("MeasureController", "placement", "listeningPosition")
    ctl.call("MeasureController", "preflight")
    time.sleep(1.5)
    shot(theme, "measure-volume-refused", "Measure at 0 dB user volume: the check before a run")
    ctl.call("MeasureController", "start")
    shot(theme, "measure-volume-refused-start", "Measure at 0 dB user volume, Measure pressed", 1.5)
    ctl.call("SidebarController", "setUserVolume", -20.0)
    time.sleep(1)
    if i == 0:
        # A measurement by the user on the Centre's drivers (the confirmed layout), then auto-tune
        ctl.call("MeasureController", "preflight")
        time.sleep(1.5)
        ctl.call("MeasureController", "start")
        try:
            wait("measuring", lambda: ctl.get("MeasureController", "measuring"), 30)
            time.sleep(4)
            shot(theme, "measure-running", "Measure while sweeping (the user's own run)")
            wait("result", lambda: ctl.get("MeasureController", "hasResult") and not ctl.get("MeasureController", "measuring"), 300)
        except RuntimeError as e:
            log("!! measure:", e, ctl.get("MeasureController", "issueText"), ctl.get("MeasureController", "playBlocker"))
        page("measure", "results")
        shot(theme, "measure-results", "Measure › Results of that run")
        page("measure", "tune")
        shot(theme, "auto-tune-layout", "Measure › Auto-tune, the layout (names and inputs)")
        ctl.call("TuneController", "propose")
        time.sleep(8)
        shot(theme, "auto-tune-proposal", "Measure › Auto-tune, a proposal")
    else:
        page("measure", "results")
        shot(theme, "measure-results", "Measure › Results of that run")
        page("measure", "tune")
        shot(theme, "auto-tune-proposal", "Measure › Auto-tune, a proposal")
    page("library", "history")
    shot(theme, "history-after-measure", "Library › History after the user's measurement")
    page("system", "events")
    shot(theme, "events", "System › Events")
ctl.set("LayoutController", "theme", "dark")
log("done:", len(manifest))
