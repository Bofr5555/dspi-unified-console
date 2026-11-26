#!/usr/bin/env python3
"""gallery.py RUN [dark|light|extras ...] [only=step,step]: screenshots of every Modern screen of an app that
`FUNCTEST_SCREEN=1600x1000 functest.py up --name RUN` left running under Xvfb (WP DESREV). Writes
runs/RUN/shots/<set>/<name>-<window>-N.png and shots/manifest.json. Sets: dark, light (every page, tab, Settings
page, popup, dialog, window and AI state), extras (the Darker tone, 1100x720, Classic for comparison, no device).
`only=` limits a set to named steps (popups, dialogs, windows, ai, darker, narrow, classic, empty, ...).

The app's control socket does the navigation (op "page", "call", "set"); windows are grabbed with op "grab"
(every visible Quick window), whole-screen frames with the host's ffmpeg on the run's Xvfb socket."""
import json
import os
import pathlib
import subprocess
import sys
import threading
import time
import traceback

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from functest import CONTAINER_WORK, Run  # noqa: E402

run = Run(sys.argv[1])
ctl = run.ctl
SHOTS = run.dir / "shots"
SHOTS.mkdir(exist_ok=True)
MANIFEST = SHOTS / "manifest.json"
manifest = json.loads(MANIFEST.read_text()) if MANIFEST.exists() else []
current_set = "dark"


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def xdotool(*args):
    return subprocess.run(["podman", "exec", "-e", "DISPLAY=:99", run.container, "xdotool", *args],
                          capture_output=True, text=True)


def key(k):
    xdotool("search", "--onlyvisible", "--name", "uConsole", "windowfocus")
    xdotool("key", k)
    time.sleep(0.4)


def settle(s=0.8):
    time.sleep(s)


def shot(name, caption, settle_s=0.8):
    """Grab every visible window into the current set."""
    settle(settle_s)
    d = SHOTS / current_set
    d.mkdir(exist_ok=True)
    files = ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{run.name}/shots/{current_set}", prefix=name + "-")
    host = [str(d / pathlib.Path(f).name) for f in files]
    manifest.append({"set": current_set, "name": name, "caption": caption, "files": host})
    MANIFEST.write_text(json.dumps(manifest, indent=1))
    log(current_set, name, [pathlib.Path(f).name for f in host])
    return host


def screen(name, caption):
    """One frame of the whole Xvfb screen (several windows at once)."""
    settle()
    d = SHOTS / current_set
    d.mkdir(exist_ok=True)
    out = d / f"{name}-screen.png"
    subprocess.run(["timeout", "-s", "KILL", "20", "ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error",
                    "-f", "x11grab", "-draw_mouse", "0", "-video_size", "1600x1000", "-i", str(run.dir / "x11/X99"),
                    "-frames:v", "1", "-y", str(out)], capture_output=True)
    if out.exists():
        manifest.append({"set": current_set, "name": name, "caption": caption, "files": [str(out)]})
        MANIFEST.write_text(json.dumps(manifest, indent=1))
        log(current_set, name, out.name)


def page(section, tab=""):
    ok = ctl.send(op="page", section=section, tab=tab)
    if not ok:
        log("!! no page", section, tab)
    settle(0.5)
    return ok


def click(name):
    """Emit a button's clicked() by objectName."""
    return ctl.call(name, "clicked")


def devices():
    return ctl.get("DeviceRegistry", "devices")


def show_device(title):
    for d in devices():
        if d.get("title") == title or d.get("label") == title or d.get("name") == title:
            ctl.call("DeviceRegistry", "activate", d["id"])
            settle(1)
            return d["id"]
    return None


def output_channel(o):
    col = ctl.call("MatrixController", "column", o)
    return col.get("channel", -1) if isinstance(col, dict) else -1


def input_channel(i):
    row = ctl.call("MatrixController", "row", i)
    return row.get("input", -1) if isinstance(row, dict) else -1


ONLY = [a.split("only=")[1].split(",") for a in sys.argv if a.startswith("only=")]
ONLY = ONLY[0] if ONLY else None


def step(name, fn):
    if ONLY and name not in ONLY and not name.startswith("x-"):
        return
    try:
        fn()
    except Exception as e:  # one failed shot must not end the gallery
        log("!!", name, "failed:", e)
        traceback.print_exc()
    # anything left open goes
    for _ in range(2):
        key("Escape")


# ---------------------------------------------------------------- the MCP probe (a question sheet, a working agent)
class Probe:
    def __init__(self, client="Claude Code"):
        self.p = subprocess.Popen(
            ["podman", "exec", "-i", run.container, f"{CONTAINER_WORK}/build-functest/uconsole-for-dspi", "--mcp"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.n = 0
        self.lock = threading.Lock()
        self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": client, "version": "0"}})
        self.p.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        self.p.stdin.flush()

    def rpc(self, method, params):
        with self.lock:
            self.n += 1
            self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
            self.p.stdin.flush()
            while True:
                line = self.p.stdout.readline()
                if not line:
                    return {}
                m = json.loads(line)
                if m.get("id") == self.n:
                    return m

    def call(self, tool, **args):
        r = self.rpc("tools/call", {"name": tool, "arguments": args})
        return r

    def call_async(self, tool, **args):
        t = threading.Thread(target=self.call, args=(tool,), kwargs=args, daemon=True)
        t.start()
        return t

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(5)
        except Exception:
            self.p.kill()


def wait_for(what, test, seconds=20):
    ctl.wait(what, test, seconds)


# ---------------------------------------------------------------- the screens
def pages_set():
    dev = devices()
    centre = show_device("Centre") or dev[0]["id"]
    out1 = output_channel(0)
    in1 = input_channel(0)

    # Overview
    step("overview", lambda: (page("overview"), shot("overview", "Overview: three devices open, the Centre shown")))

    # Equaliser
    def eq_peq():
        page("equaliser", "peq")
        ctl.call("SidebarController", "select", out1)
        settle(0.5)
        # a few real bands on output 1, as a tuned device has
        for band, t, f, g, q in ((0, 1, 63.0, -4.0, 3.0), (1, 1, 180.0, 2.5, 1.4), (2, 3, 2500.0, -1.5, 0.7)):
            ctl.call("ChannelController", "setBandType", out1, band, False, t)
            for field, v in (("freq", f), ("gain", g), ("q", q)):
                ctl.call("ChannelController", "setBandValue", out1, band, False, field, v)
        settle(0.8)
        shot("eq-peq-output", "Equaliser › PEQ, output 1 (Centre woofer) selected, inspector as it comes up")
        ctl.call("PeqEditorController", "rowClicked", 1, 0)
        settle(0.3)
        shot("eq-peq-band", "Equaliser › PEQ with band 2 selected (band inspector)")
        click("EqualiserTools.inspector")
        settle(0.3)
        shot("eq-peq-noinspector", "Equaliser › PEQ with the inspector collapsed")
        click("EqualiserTools.inspector")
        settle(0.3)
        ctl.call("SidebarController", "select", in1)
        settle(0.5)
        shot("eq-peq-input", "Equaliser › PEQ, input 1 selected (preamp, pair link)")
        ctl.call("SidebarController", "select", out1)
    step("eq-peq", eq_peq)
    step("eq-xo", lambda: (page("equaliser", "crossover"), shot("eq-crossover", "Equaliser › Crossover")))
    step("eq-delay", lambda: (page("equaliser", "delay"), shot("eq-delay", "Equaliser › Delay & Gain")))

    # Routing (matrix)
    def routing():
        page("routing", "matrix")
        shot("routing-matrix", "Routing › Matrix mixer (the page)")
        ctl.call("SidebarController", "select", out1)
        settle(0.4)
        try:
            click("RoutingTools.inspector")
        except Exception:
            pass
        shot("routing-matrix-inspector", "Routing with the output inspector open")
    step("routing", routing)

    # Effects
    for tab, cap in [("crossfeed", "Crossfeed"), ("loudness", "Loudness compensation"), ("leveller", "Volume leveller"),
                     ("psybass", "Psychoacoustic bass"), ("subharmonic", "Subharmonic synthesizer"), ("tube", "Tube modeller"),
                     ("upmixer", "Stereo upmixer")]:
        step("fx-" + tab, lambda tab=tab, cap=cap: (page("effects", tab), shot("effects-" + tab, "Effects › " + cap)))

    # Measure
    for tab, cap in [("measure", "Measure (setup)"), ("results", "Results"), ("live", "Live"), ("tune", "Auto-tune"),
                     ("documents", "Documents"), ("units", "Several units")]:
        step("measure-" + tab, lambda tab=tab, cap=cap: (page("measure", tab), shot("measure-" + tab, "Measure › " + cap)))

    # Analyse
    for tab, cap in [("spectrum", "Spectrum analyser"), ("signalGenerator", "Signal generator"), ("stats", "Stats for nerds"),
                     ("interruptMonitor", "Interrupt monitor")]:
        step("analyse-" + tab, lambda tab=tab, cap=cap: (page("analyse", tab), shot("analyse-" + tab, "Analyse › " + cap)))

    # Library
    for tab, cap in [("presets", "Presets"), ("snapshots", "Snapshots"), ("history", "History"), ("files", "Files"),
                     ("autoeq", "AutoEQ (headphone profiles)")]:
        step("library-" + tab, lambda tab=tab, cap=cap: (page("library", tab), shot("library-" + tab, "Library › " + cap)))

    # System
    for tab, cap in [("status", "Units"), ("clock", "Clock and links"), ("installation", "Installation (setup)"), ("events", "Events")]:
        step("system-" + tab, lambda tab=tab, cap=cap: (page("system", tab), shot("system-" + tab, "System › " + cap)))

    # Devices
    for tab, cap in [("devices", "This computer"), ("hubs", "Hubs"), ("firmware", "Firmware update")]:
        step("devices-" + tab, lambda tab=tab, cap=cap: (page("devices", tab), shot("devices-" + tab, "Devices › " + cap)))

    # Settings: the four groups, then every page
    for tab, cap in [("application", "Application"), ("display", "Display"), ("device", "Device"), ("control", "Control")]:
        step("settings-" + tab, lambda tab=tab, cap=cap: (page("settings", tab), shot("settings-" + tab, "Settings › " + cap)))

    def settings_pages():
        rows = ctl.call("SettingsController", "sidebarRows", "")
        ids = [r.get("id") for r in rows if isinstance(r, dict) and r.get("id")]
        log("settings pages:", ids)
        for pid in ids:
            page("settings", "")
            ctl.call("SettingsController", "open", pid)
            shot("settings-page-" + pid, "Settings page: " + pid, 0.6)
    step("settings-pages", settings_pages)
    step("settings-ai", lambda: (page("settings", "aiAgents"), shot("settings-aiAgents", "Settings › AI agents")))
    step("settings-whatsnew", lambda: (page("settings", "whatsNew"), shot("settings-whatsNew", "Settings › What's new")))

    # Top bar and its popups
    page("overview")

    def popup(name, caption, opener, closer=None):
        try:
            opener()
            shot(name, caption)
        except Exception as e:
            log("!! popup", name, e)
        key("Escape")
        if closer:
            try:
                closer()
            except Exception:
                pass

    def popups():
        popup("top-appmenu", "Top bar: the app menu (uConsole ▾)", lambda: click("AppMenuButton"))
        # -- INTC (DL5 fold) -- the preset control is one popover (PresetPopover): the list, then a slot's menu
        # from the active slot's row
        def preset_slot_menu():
            ctl.set("TopBar.preset", "popup.visible", True)
            settle(0.6)
            click(f"PresetPopover.slot.{ctl.get('DeviceModel', 'presets.activeSlot')}.menu")
        popup("top-preset-list", "Top bar: the preset popover", lambda: ctl.set("TopBar.preset", "popup.visible", True),
              lambda: ctl.set("TopBar.preset", "popup.visible", False))
        popup("top-preset-menu", "Top bar: a slot's menu, from its row in the preset popover", preset_slot_menu)
        popup("dock-source-list", "Dock: the source list (combo box)", lambda: ctl.set("Dock.source", "popup.visible", True),
              lambda: ctl.set("Dock.source", "popup.visible", False))
        popup("top-save-menu", "Top bar: Save to device split button menu", lambda: click("TopBar.save.more"))
        popup("top-theme-menu", "Top bar: theme menu", lambda: click("TopBar.theme"))
        popup("top-undo-menu", "Top bar: undo history menu", lambda: click("TopBar.undo.more"))
        popup("top-notifications", "Notification drawer", lambda: click("TopBar.bell"), lambda: click("NotificationDrawer.close"))
        popup("top-ai-popover", "AI popover (agents on, none connected)", lambda: click("TopBar.ai"))
        popup("top-device-switcher", "Device switcher (device picker)", lambda: ctl.call("DevicesController", "openList"),
              lambda: ctl.call("DevicesController", "closeList"))
        popup("palette", "Command palette (Ctrl K)", lambda: ctl.call("PaletteController", "open"))
        popup("palette-search", "Command palette with a search", lambda: (ctl.call("PaletteController", "open"), settle(0.5), xdotool("type", "cross")))
    step("popups", popups)

    # Dialogs
    def dialogs():
        for name, caption, action in (("dialog-factory-reset", "Factory reset confirmation", "device.factoryReset"),
                                      ("dialog-import-filters", "Import filters", "file.importFilters"),
                                      ("dialog-import-config", "Import device configuration", "file.importConfiguration"),
                                      ("dialog-export-config", "Export device configuration", "file.exportConfiguration"),
                                      ("dialog-save-template", "Save as template", "file.saveAsTemplate"),
                                      ("dialog-new-template", "New from template", "file.newFromTemplate"),
                                      ("dialog-diagnostics", "Save diagnostics", "help.saveDiagnostics"),
                                      ("dialog-about", "About", "help.about"),
                                      ("dialog-getting-started", "Getting started", "help.gettingStarted"),
                                      ("dialog-close-device", "Close device", "device.close")):
            try:
                ctl.call("Shell", "trigger", action)
                settle(1.2)
                shot(name, caption)
                screen(name, caption + " (whole screen)")
            except Exception as e:
                log("!! dialog", name, e)
            key("Escape")
            key("Escape")
            if action == "help.gettingStarted":
                # the wizard replaces the console: leave it
                try:
                    ctl.call("OnboardingController", "finishSetup")
                except Exception:
                    pass
                settle(1)
    step("dialogs", dialogs)

    # Tool windows: a page opened in its own window
    def windows():
        page("effects", "crossfeed")
        ctl.call("Shell", "openWindow", "tools.crossfeed")
        settle(1.5)
        shot("window-crossfeed", "Effects › Crossfeed opened in its own window (and the page saying so)")
        screen("window-crossfeed", "Both windows on the screen")
        ctl.call("Shell", "closeWindow", "tools.crossfeed")
        page("routing", "matrix")
        ctl.call("Shell", "openWindow", "tools.matrix")
        settle(1.5)
        shot("window-matrix", "Routing › Matrix opened in its own window")
        ctl.call("Shell", "closeWindow", "tools.matrix")
        page("equaliser", "peq")
        ctl.call("Shell", "trigger", "window.popOutGraph")
        settle(1.5)
        shot("window-graph", "The graph popped out")
        screen("window-graph", "Graph window and main window")
        ctl.call("Shell", "closeWindow", "window.popOutGraph")
        key("Escape")
    step("windows", windows)

    # The AI: pairing sheet, question sheet, working strip
    def ai():
        page("equaliser", "peq")
        ctl.set("AiController", "confirmMode", "ask_everything")
        def pair():
            try:
                wait_for("pairing", lambda: bool(ctl.get("AiController", "pairing")), 20)
                shot("ai-pairing", "An agent asks to connect (pairing sheet)")
            except RuntimeError:
                log("no pairing sheet")
            ctl.call("AiController", "answerPairing", True)
        for c in ctl.get("AiController", "clients") or []:
            try:
                ctl.call("AiController", "revoke", c["id"])
            except Exception as e:
                log("revoke", e)
        settle(0.5)
        t = threading.Thread(target=pair, daemon=True)
        t.start()
        probe = Probe()
        t.join(30)
        settle(1)
        probe.call_async("dspi_set_gain", device=centre, output=1, gain_db=-2.0)  # -- WP INTE -- Out 1: agents count from 1 (SHOWFIX 9)
        wait_for("question", lambda: bool(ctl.get("AiController", "confirmations")), 20)
        shot("ai-question", "An agent's question sheet (a write under ask everything)")
        for q in ctl.get("AiController", "confirmations"):
            ctl.call("AiController", "answer", q["id"], False)
        settle(1)
        probe.call_async("dspi_measure_start", device=centre, outputs=[1])  # -- WP INTE -- Out 1 (SHOWFIX 9)
        try:
            wait_for("question", lambda: bool(ctl.get("AiController", "confirmations")), 20)
            shot("ai-question-signal", "Question sheet for a test signal (with the 10-minute grant)")
            for q in ctl.get("AiController", "confirmations"):
                ctl.call("AiController", "answer", q["id"], True)
        except RuntimeError:
            log("no measure question")
        try:
            wait_for("working", lambda: ctl.get("AiController", "working"), 20)
            settle(1.5)
            shot("ai-working", "The AI working strip while an agent measures")
            click("TopBar.ai")
            shot("ai-popover-connected", "AI popover with an agent connected and its activity")
            key("Escape")
        except RuntimeError:
            log("agent never 'working'")
        probe.call("dspi_measure_stop", device=centre)
        settle(2)
        page("library", "history")
        shot("library-history-ai", "History with AI entries")
        probe.close()
        settle(2)
    step("ai", ai)

    page("overview")


def extras_set():
    global current_set
    for g in ctl.get("AiController", "grants") or []:
        try:
            ctl.call("AiController", "revokeGrant", g["id"])
        except Exception as e:
            log("revoke grant", e)
    dev = devices()
    centre = (show_device("Centre") or dev[0]["id"]) if dev else None
    out1 = output_channel(0) if dev else -1

    # Darker grey tone
    def darker():
        global current_set
        ctl.set("LayoutController", "arrangement.tone", "darker")
        settle(1)
        for theme in ("dark", "light"):
            ctl.set("LayoutController", "theme", theme)
            settle(1)
            current_set = "darker-" + theme
            page("overview")
            shot("overview", "Darker grey tone: Overview")
            page("equaliser", "peq")
            ctl.call("SidebarController", "select", out1)
            shot("eq-peq", "Darker grey tone: Equaliser")
            page("routing", "matrix")
            shot("routing", "Darker grey tone: Routing")
        ctl.set("LayoutController", "arrangement.tone", "grey")
        ctl.set("LayoutController", "theme", "dark")
        settle(1)
    step("darker", darker)

    # Narrow window
    def narrow():
        global current_set
        xdotool("search", "--onlyvisible", "--name", ".", "windowsize", "1100", "720", "windowmove", "0", "0")
        settle(1.5)
        for theme in ("dark", "light"):
            ctl.set("LayoutController", "theme", theme)
            settle(1)
            current_set = "narrow-" + theme
            page("overview")
            shot("overview", "1100×720: Overview")
            page("equaliser", "peq")
            ctl.call("SidebarController", "select", out1)
            shot("eq-peq", "1100×720: Equaliser › PEQ")
            page("routing", "matrix")
            shot("routing", "1100×720: Routing")
            page("effects", "crossfeed")
            shot("effects-crossfeed", "1100×720: Effects › Crossfeed")
            page("measure", "measure")
            shot("measure", "1100×720: Measure")
            page("settings", "application")
            shot("settings", "1100×720: Settings")
        xdotool("search", "--onlyvisible", "--name", ".", "windowsize", "1600", "1000", "windowmove", "0", "0")
        ctl.set("LayoutController", "theme", "dark")
        settle(1.5)
    step("narrow", narrow)

    # Classic, for comparison
    def classic():
        global current_set
        ctl.set("LayoutController", "layout", "classic")
        wait_for("classic", lambda: ctl.get("LayoutController", "layout") == "classic")
        settle(2)
        for theme in ("dark", "light"):
            ctl.set("LayoutController", "theme", theme)
            settle(1)
            current_set = "classic-" + theme
            ctl.call("SidebarController", "select", out1)
            settle(0.5)
            shot("classic-output", "Classic: output 1 selected")
            ctl.call("Shell", "trigger", "tools.matrix")
            settle(1.5)
            shot("classic-matrix", "Classic: Matrix Mixer window")
            screen("classic-matrix", "Classic with the matrix window over it")
            ctl.call("Shell", "closeWindow", "tools.matrix")
            ctl.call("Shell", "trigger", "tools.crossfeed")
            settle(1.5)
            shot("classic-crossfeed", "Classic: Crossfeed window")
            ctl.call("Shell", "closeWindow", "tools.crossfeed")
            ctl.call("Shell", "trigger", "edit.settings")
            settle(1.5)
            shot("classic-settings", "Classic: Settings window")
            ctl.call("Shell", "closeWindow", "edit.settings")
            ctl.call("Shell", "trigger", "tools.measure")
            settle(1.5)
            shot("classic-measure", "Classic: Measure window")
            ctl.call("Shell", "closeWindow", "tools.measure")
            ctl.call("SidebarController", "select", -1)
            settle(0.5)
            shot("classic-dashboard", "Classic: dashboard (no channel selected)")
        ctl.set("LayoutController", "theme", "dark")
        ctl.set("LayoutController", "layout", "modern")
        wait_for("modern", lambda: ctl.get("LayoutController", "layout") == "modern")
        settle(2)
    step("classic", classic)

    # Empty state: no device (last: the devices are closed)
    def empty():
        global current_set
        for d in devices():
            ctl.call("DeviceRegistry", "activate", d["id"])
            settle(1)
            ctl.call("Shell", "trigger", "device.commit")
            settle(1.5)
        for d in devices():
            ctl.call("DevicesController", "closeOpen", d["id"])
            settle(1.5)
            key("Return")
            settle(0.5)
        settle(1.5)
        for theme in ("dark", "light"):
            ctl.set("LayoutController", "theme", theme)
            settle(1)
            current_set = "empty-" + theme
            page("overview")
            shot("overview", "No device: Overview")
            page("equaliser", "peq")
            shot("eq-peq", "No device: Equaliser")
            page("routing", "matrix")
            shot("routing", "No device: Routing")
            page("measure", "measure")
            shot("measure", "No device: Measure")
            page("devices", "devices")
            shot("devices", "No device: Devices › This computer")
        ctl.set("LayoutController", "theme", "dark")
    step("empty", empty)


what = sys.argv[2:] or ["dark", "light", "extras"]
for w in what:
    if w in ("dark", "light"):
        current_set = w
        ctl.set("LayoutController", "theme", w)
        settle(1.5)
        pages_set()
    elif w == "extras":
        extras_set()
log("done:", len(manifest), "entries")
