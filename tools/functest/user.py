#!/usr/bin/env python3
"""-- WP FUNC2a -- The user, between the agent's prompts (FSTORY P9a, P8).

What a person does in the app, through the app's own controllers on the control
socket (README.md, "Acting as the user"): set a crossover or a route, drag an EQ
band, rename a channel or a preset, take a snapshot with a note, nudge the volume,
confirm a speaker setup, press Undo. A story lists them:

    [[user]]                     before the first prompt (after the speaker setups)
    do = "crossover"
    device = "Fronts"
    ...
    [[step.user]]                before that step's prompt; `during = true` runs it
    do = "drag"                  beside the agent instead, `at_s` after the prompt
    ...
    [[after]]                    after the last step

`seed = "tuned"` (story) runs `seeds/tuned.json`'s actions first. Outputs and inputs
are numbered from 1, as the user sees them.

The app's questions (sheets) are answered by pattern: each `[[answer]]` of the story
is {match = "regex", answer = "allow" | "deny", times = n}; the first rule that
matches a sheet's text and has turns left answers it, else Allow. Every sheet's whole
text, the answer and the rule go to user.jsonl.
"""
import json
import pathlib
import re
import threading
import time

HERE = pathlib.Path(__file__).resolve().parent
# ChannelController's crossover families
FAMILY = {"off": 0, "linkwitz_riley": 1, "butterworth": 2, "bessel": 3}
# FILTER_* of the PEQ types a user picks here
PEQ = {"flat": 0, "peaking": 1, "low_shelf": 2, "high_shelf": 3, "low_pass": 4, "high_pass": 5}


class User:
    def __init__(self, run):
        self.run = run
        self.ctl = run.ctl
        self.threads = []

    def device_id(self, name):
        for d in self.ctl.get("DeviceRegistry", "devices"):
            if name in (d.get("title"), d.get("label"), d.get("id"), d.get("serial")):
                return d["id"]
        raise RuntimeError(f"no device {name!r} open")

    def show(self, name):
        """Make `name` the shown device, as a click on its tab."""
        i = self.device_id(name)
        if self.ctl.get("DeviceModel", "connected") and self.shown() == i:
            return
        self.ctl.call("DeviceRegistry", "activate", i)
        self.ctl.wait(f"{name} shown", lambda: self.shown() == i and self.ctl.get("DeviceModel", "connected"))
        time.sleep(0.5)

    def shown(self):
        for d in self.ctl.get("DeviceRegistry", "devices"):
            if d.get("active"):
                return d["id"]
        return None

    def do_all(self, actions, label):
        for a in actions:
            if a.get("during"):
                t = threading.Thread(target=self.later, args=(a, label), daemon=True)
                t.start()
                self.threads.append(t)
            else:
                self.do(a, label)

    def later(self, a, label):
        time.sleep(float(a.get("at_s", 0)))
        try:
            self.do(a, label)
        except (OSError, RuntimeError) as e:
            self.run.event("user-failed", {"step": label, "action": a, "error": str(e)})

    def join(self):
        for t in self.threads:
            t.join(timeout=120)
        self.threads = []

    def do(self, a, label=""):
        """One action; logged to user.jsonl before and after."""
        kind = a["do"]
        self.run.event("user", {"step": label, "action": a})
        ctl = self.ctl
        if kind == "wait":
            time.sleep(float(a["s"]))
            return
        if kind == "show":
            self.show(a["device"])
            return
        if kind == "layout":
            self.run.confirm_layout(a["device"])
            return
        if kind in INSTALLER:  # -- WP FUNC2c --
            self.installer(a)
            self.run.event("user-done", {"step": label, "do": kind})
            return
        if kind == "agents":  # -- WP ODEC -- Stop (off) and on again, as in Settings › AI Agents
            ctl.call("AiController", "setEnabled", bool(a["on"]))
            ctl.wait("AI agents " + ("on" if a["on"] else "off"),
                     lambda: bool(ctl.get("AiController", "running")) == bool(a["on"]))
            return
        self.show(a["device"])
        out = a.get("output", 0) - 1
        if kind == "crossover":
            ch = ctl.call("ChannelController", "outputChannel", out)
            low = a["side"] == "lp"
            band = a.get("band", 1 if low else 0)
            t = ctl.call("ChannelController", "crossoverType", FAMILY[a.get("family", "linkwitz_riley")], a.get("order", 4), low)
            ctl.call("ChannelController", "setBandType", ch, band, True, t)
            ctl.call("ChannelController", "setBandValue", ch, band, True, "freq", float(a["hz"]))
        elif kind == "route":
            ctl.call("ChannelController", "setRoute", a["input"] - 1, out, a.get("on", True), float(a.get("gain_db", 0)),
                     a.get("invert", False))
        elif kind == "gain":
            ctl.call("ChannelController", "setOutputGain", out, float(a["db"]))
        elif kind == "peq":
            ch = ctl.call("ChannelController", "outputChannel", out) if "output" in a else a["input"] - 1
            band = a["band"] - 1
            ctl.call("ChannelController", "setBandType", ch, band, False, PEQ[a.get("type", "peaking")])
            for field, key in (("freq", "freq_hz"), ("q", "q"), ("gain", "gain_db")):
                if key in a:
                    ctl.call("ChannelController", "setBandValue", ch, band, False, field, float(a[key]))
        elif kind == "drag":
            # An EQ band's gain held and moved, as a handle on the graph: one undo step
            ch = ctl.call("ChannelController", "outputChannel", out) if "output" in a else a["input"] - 1
            band = a["band"] - 1
            ctl.call("DeviceModel", "beginDrag")
            try:
                for v in a.get("values", []):
                    ctl.call("ChannelController", "setBandValue", ch, band, False, a.get("field", "gain"), float(v))
                    time.sleep(0.15)
                time.sleep(float(a.get("hold_s", 0)))
            finally:
                ctl.call("DeviceModel", "endDrag")
        elif kind == "name":
            ch = ctl.call("ChannelController", "outputChannel", out) if "output" in a else a["input"] - 1
            if not ctl.call("SidebarController", "renameChannel", ch, a["name"]):
                raise RuntimeError(f"rename {a}: refused")
        elif kind == "preset_name":
            ctl.call("SidebarController", "renamePreset", a["slot"] - 1, a["name"])
        elif kind == "snapshot":
            ctl.call("SnapshotsController", "capture", a["name"], a.get("note", ""))
        elif kind == "volume":
            ctl.call("SidebarController", "setUserVolume", float(a["db"]))
        elif kind == "undo":
            ctl.call("HistoryController", "undo")
        # -- WP FUNC2b -- presets as the sidebar's preset menu does them (slots from 1); A/B as its toggle
        elif kind == "preset_save":
            # The active slot: the device's settings saved there
            self.preset(lambda: ctl.call("SidebarController", "savePreset"))
        elif kind == "preset_copy":
            # The settings copied to `slot`, the active one saved again (nothing unsaved first)
            if ctl.get("SidebarController", "unsaved"):
                raise RuntimeError("preset_copy: unsaved changes; save first")
            self.preset(lambda: ctl.call("SidebarController", "requestPresetCopy", a["slot"] - 1))
        elif kind == "startup":
            ctl.call("SidebarController", "setStartupPreset", a["slot"] - 1)
        elif kind == "ab_toggle":
            if not ctl.get("SnapshotsController", "abActive"):
                self.run.event("user-skipped", {"step": label, "do": kind, "why": "no A/B running"})
                return
            for _ in range(a.get("times", 1)):
                ctl.call("SnapshotsController", "abToggle")
                time.sleep(float(a.get("listen_s", 3)))
        elif kind == "ab_keep":
            if not ctl.get("SnapshotsController", "abActive"):
                self.run.event("user-skipped", {"step": label, "do": kind, "why": "no A/B running"})
                return
            ctl.call("SnapshotsController", "abKeep", a["side"])
        # -- WP FUNC2c -- a preset switch (unsaved changes resolved as `unsaved` says), an output's mute
        elif kind == "preset_switch":
            ctl.call("SidebarController", "requestPresetSwitch", a["slot"] - 1)
            self.preset_idle()
            if ctl.get("SidebarController", "unsaved") and a.get("unsaved"):
                ctl.call("SidebarController", "resolveUnsaved", "switch", a["slot"] - 1, a["unsaved"])
                self.preset_idle()
        elif kind == "mute":
            ctl.call("ChannelController", "setOutputMute", out, a.get("muted", True))
        else:
            raise RuntimeError(f"no such user action: {kind}")
        time.sleep(float(a.get("settle_s", 0.3)))
        self.run.event("user-done", {"step": label, "do": kind})


    # -- WP FUNC2b --
    def preset(self, start):
        """A preset operation, and the wait until the device has done it."""
        start()
        self.preset_idle()

    # -- WP FUNC2c -- FSTORY P4: the installer at this console (Settings › Installer, LimitsController)
    def preset_idle(self):
        time.sleep(0.3)
        self.ctl.wait("the preset call", lambda: not self.ctl.get("SidebarController", "presetBusy"), 30)
        time.sleep(0.5)

    def limits_idle(self, what):
        time.sleep(0.3)
        self.ctl.wait(what, lambda: not self.ctl.get("LimitsController", "busy"), 30)
        error = self.ctl.get("LimitsController", "pinError")
        if error:
            raise RuntimeError(f"{what}: {error}")

    def installer(self, a):
        """`limits` (a profile: `outputs` = {output from 1: {field: value}}, `profile` = {field: value},
        `note`, `contact`), `pin` (set it; the recovery code goes to secrets.json, never to the agent),
        `covers` (what the lock covers: on = every page), `lock` (lock now), `unlock` (with the PIN)."""
        kind, ctl = a["do"], self.ctl
        self.show(a["device"])
        if kind == "limits":
            for n, fields in a.get("outputs", {}).items():
                for field, value in fields.items():
                    ctl.call("LimitsController", "setOutputLimit", int(n) - 1, field, float(value))
                    self.limits_idle(f"limit {n} {field}")
            for field, value in a.get("profile", {}).items():
                ctl.call("LimitsController", "setLimit", field, float(value))
                self.limits_idle(f"limit {field}")
            for field in ("note", "contact"):
                if field in a:
                    ctl.call("LimitsController", "setText", field, a[field])
                    self.limits_idle(f"limit {field}")
            ctl.wait("the profile in force", lambda: ctl.get("LimitsController", "active"))
        elif kind == "pin":
            ctl.call("LimitsController", "setPin", a["pin"], a["pin"])
            self.limits_idle("the PIN")
            ctl.wait("the PIN set", lambda: ctl.get("LimitsController", "pinSet"))
            self.run.secret(a["pin"], ctl.get("LimitsController", "recoveryCode"))
            ctl.call("LimitsController", "dismissRecovery")
        elif kind == "covers":
            ctl.call("LimitsController", "setCovers", a.get("on", True))
            self.limits_idle("the lock's cover")
            ctl.wait("the cover set", lambda: ctl.get("LimitsController", "covers") == a.get("on", True))
        elif kind == "lock":
            ctl.call("LimitsController", "lockNow")
            self.limits_idle("the lock")
            ctl.wait("locked", lambda: ctl.get("LimitsController", "locked") == ctl.get("LimitsController", "covers"))
        elif kind == "unlock":
            ctl.call("LimitsController", "unlock", a["pin"])
            self.limits_idle("the unlock")
            ctl.wait("unlocked", lambda: not ctl.get("LimitsController", "locked"))
        self.run.event("installer", {"do": kind, "lock": {k: v for k, v in (ctl.get("LimitsController", "lock") or {}).items()
                                                          if k in ("pinSet", "locked", "covers", "unlocked", "heldBy")}})
        time.sleep(0.3)


INSTALLER = ("limits", "pin", "covers", "lock", "unlock")


def seed_actions(story):
    """The story's actions before the first prompt: its seed file's, then its own."""
    out = []
    if story.get("seed"):
        out += json.loads((HERE / "seeds" / f"{story['seed']}.json").read_text())["actions"]
    return out + story.get("user", [])


class Answers:
    """The user's answers to the app's questions, by pattern."""

    def __init__(self, rules):
        self.rules = [dict(r, left=r.get("times", -1)) for r in rules]
        self.lock = threading.Lock()

    def text(self, q):
        parts = [str(q.get(k, "")) for k in ("title", "device", "client", "text", "summary",
                                             "grantText")]  # -- WP ODEC -- the grant's sentence
        parts += [str(x) for x in q.get("riskLines") or q.get("risks") or []]
        return " | ".join(p for p in parts if p)

    def answer(self, q):
        """(allow, the rule's match or None, the sheet's text)"""
        text = self.text(q)
        with self.lock:
            for r in self.rules:
                if r["left"] != 0 and re.search(r["match"], text, re.I):
                    r["left"] -= 1
                    return r["answer"] == "allow", r["match"], text
        return True, None, text
