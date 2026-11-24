#!/usr/bin/env python3
"""The function test: a real AI agent, or a replay of one, tunes a simulated home
theatre through the desktop app's MCP server (README.md). Host side: standard
library only; the app runs in a container with no USB, no sound device and no
network. run.sh is the way in.

    functest.py run  --name r1 [--story F] [--confirm never_ask|ask_risky] [--steps 1,2,3] [--replay F] [--keep]
                     (--confirm: the replay's `confirm`, else the story's, else never_ask)
    functest.py up   --name r1 ...         the app, set up, left running
    functest.py ask  --name r1 --steps 1   the story's prompt, or: ask --name r1 --label l "the words"
    functest.py save --name r1 label       the state now
    functest.py down --name r1             History, screenshots, stop, the verdict

    --record (or FUNCTEST_SCREEN=1600x1000): the app on a screen of its own (Xvfb in its container, the
    xcb platform) in place of offscreen; --record also films it (screen.mp4, host ffmpeg) and stamps the
    agent's tool calls and the app's questions in record.jsonl, for captions (WP SHOW)
"""
import argparse
import json
import os
import pathlib
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import tomllib
import uuid

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent.parent
CACHE = pathlib.Path(os.environ.get("DSPI_BUILD_CACHE", pathlib.Path.home() / ".cache/dspi-build"))
BRANCH = subprocess.run(["git", "-C", str(REPO), "rev-parse", "--abbrev-ref", "HEAD"],
                        capture_output=True, text=True).stdout.strip().replace("/", "-") or "detached"
# -- WP SHOWFIX -- a detached worktree is named by its short commit (as ci/run.sh does), not "HEAD"
if BRANCH == "HEAD":
    BRANCH = subprocess.run(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"],
                            capture_output=True, text=True).stdout.strip() or "detached"
WORK = CACHE / "work" / BRANCH
AGENT = os.environ.get("FUNCTEST_AGENT", "claude2")
MODEL = os.environ.get("FUNCTEST_MODEL", "opus")
CONTAINER_WORK = "/work"
SCREEN = os.environ.get("FUNCTEST_SCREEN", "")  # -- WP SHOW -- "1600x1000": Xvfb, not offscreen


def image():
    conf = subprocess.run(["bash", "-c", f'. "{REPO}/ci/container/images.conf"; echo "$COREMERGE"'],
                          capture_output=True, text=True, check=True).stdout.strip()
    return os.environ.get("DSPI_IMAGE", conf)


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


class Ctl:
    """The app's control socket: what the user does in the app."""

    def __init__(self, path):
        self.path = str(path)
        self.lock = threading.Lock()

    def send(self, **cmd):
        with self.lock, socket.socket(socket.AF_UNIX) as s:
            s.settimeout(60)
            s.connect(self.path)
            s.sendall(json.dumps(cmd).encode() + b"\n")
            buf = b""
            while not buf.endswith(b"\n"):
                part = s.recv(1 << 20)
                if not part:
                    break
                buf += part
        r = json.loads(buf or b"{}")
        if not r.get("ok"):
            raise RuntimeError(f"{cmd}: {r.get('error', 'no answer')}")
        return r.get("value")

    def get(self, on, prop):
        return self.send(op="get", on=on, prop=prop)

    def set(self, on, prop, value):
        return self.send(op="set", on=on, prop=prop, value=value)

    def call(self, on, method, *args):
        return self.send(op="call", on=on, method=method, args=list(args))

    def wait(self, what, test, seconds=20):
        end = time.time() + seconds
        while time.time() < end:
            try:
                if test():
                    return
            except (OSError, RuntimeError):
                pass
            time.sleep(0.2)
        raise RuntimeError(f"waited {seconds} s for: {what}")


class Run:
    def __init__(self, name):
        self.name = name
        self.dir = WORK / "runs" / name
        self.container = f"functest-{name}"
        self.ctl = Ctl(self.dir / "ctl.sock")
        self.meta = self.dir / "run.json"
        self.stop = threading.Event()
        self.watcher = None

    def story(self):
        return tomllib.loads((self.dir / "story.toml").read_text())

    # ------------------------------------------------------------- the app

    def up(self, confirm, story):
        if self.dir.exists():
            raise SystemExit(f"{self.dir} exists: choose another --name")
        for d in ("home/.config/uconsole-for-dspi", "sim", "agent"):
            (self.dir / d).mkdir(parents=True)
        (self.dir / "home/.config/uconsole-for-dspi/uconsole-for-dspi.conf").write_text(
            "[onboarding]\nsetupDone=true\n"
            # -- WP INTG -- (review RE1) the simulated microphone reads no dB SPL:
            # the agents' SPL cap off (on, every agent measurement is refused)
            "[mcp]\nagentSplCapOn=false\n")
        shutil.copy(story, self.dir / "story.toml")
        # -- WP FUNC2c -- a story may name a variant of the scenario (scenario.py)
        variant = self.story().get("scenario")
        shutil.copy(HERE / (f"scenario-{variant}.json" if variant else "scenario.json"), self.dir / "scenario.json")
        unconfirmed = self.story().get("unconfirmed", [])
        (CACHE / "work/empty").mkdir(parents=True, exist_ok=True)
        c = f"{CONTAINER_WORK}/runs/{self.name}"
        env = {"HOME": f"{c}/home", "XDG_CONFIG_HOME": f"{c}/home/.config", "XDG_DATA_HOME": f"{c}/home/.local/share",
               "QT_QPA_PLATFORM": "offscreen", "QT_QUICK_BACKEND": "software", "QT_FORCE_STDERR_LOGGING": "1",
               "RUST_LOG": "warn", "TMPDIR": f"{CONTAINER_WORK}/tmp",
               "DSPI_SIM_SCENARIO": f"{c}/scenario.json", "DSPI_SIM_OUT": f"{c}/sim",
               "DSPI_FUNCTEST_SOCKET": f"{c}/ctl.sock"}
        # More for the app's environment, as NAME=value words
        env.update(x.split("=", 1) for x in os.environ.get("FUNCTEST_ENV", "").split())
        cmd = ["podman", "run", "-d", "--rm", "--name", self.container, "--network", "none", "--userns=keep-id",
               "--user", f"{os.getuid()}:{os.getgid()}", "-v", f"{WORK}:{CONTAINER_WORK}:z",
               "-v", f"{CACHE}/work/empty:/sys/bus/usb/devices:ro,z", "-w", c]
        start = ""
        # -- WP SHOW -- a screen: Xvfb's socket in the run's folder (the host's ffmpeg films it from
        # there; MIT-SHM needs the host's IPC namespace, else x11grab fails or hangs)
        if SCREEN:
            (self.dir / "x11").mkdir()
            (self.dir / "x11").chmod(0o1777)
            env.update(QT_QPA_PLATFORM="xcb", DISPLAY=":99")
            cmd += ["--ipc=host", "-v", f"{self.dir}/x11:/tmp/.X11-unix:z"]
            start = (f"Xvfb :99 -screen 0 {SCREEN}x24 -nolisten tcp > {c}/xvfb.log 2>&1 & "
                     "while [ ! -S /tmp/.X11-unix/X99 ]; do sleep 0.1; done; ")
        for k, v in env.items():
            cmd += ["-e", f"{k}={v}"]
        cmd += [image(), "bash", "-c",
                f"{start}exec {CONTAINER_WORK}/build-functest/uconsole-for-dspi > {c}/app.log 2>&1"]
        subprocess.run(cmd, check=True, capture_output=True)
        log("the app starts in", self.container, "(simulated: DSPI_SIM_SCENARIO)")  # -- WP ODEC -- testers see it
        ctl = self.ctl
        ctl.wait("the app's socket", lambda: ctl.get("AiController", "port"), 60)
        if SCREEN:  # -- WP SHOW -- the window fills the screen (no window manager)
            time.sleep(1)
            w, h = SCREEN.split("x")
            subprocess.run(["podman", "exec", "-e", "DISPLAY=:99", self.container, "xdotool", "search", "--onlyvisible",
                            "--name", ".", "windowsize", w, h, "windowmove", "0", "0", "mousemove", str(int(w) - 1),
                            str(int(h) - 1)], capture_output=True)  # the pointer out of the way (no hover)
            if os.environ.get("FUNCTEST_RECORD"):
                self.record()
        # -- WP FUNC2b -- Modern unless the story or --layout says Classic (FSTORY S15)
        # (a string; a `[layout.<device>]` table is FUNC2c's seeded setup, not the app's layout: INTI)
        story_layout = self.story().get("layout")
        layout = os.environ.get("FUNCTEST_LAYOUT") or (story_layout if isinstance(story_layout, str) else None) or "modern"
        ctl.set("LayoutController", "layout", layout)
        ctl.wait(f"the {layout} layout", lambda: ctl.get("LayoutController", "layout") == layout)
        scenario = json.loads((self.dir / "scenario.json").read_text())
        for i, dev in enumerate(scenario["devices"]):
            ctl.call("DeviceRegistry", "openDemo")
            ctl.wait(f"device {i + 1}", lambda: [d.get("serial") for d in ctl.get("DeviceRegistry", "devices")]
                     == [d["serial"] for d in scenario["devices"][:i + 1]] and ctl.get("DeviceModel", "connected"))
            time.sleep(1)
        devices = ctl.get("DeviceRegistry", "devices")
        log("open:", [(d.get("id"), d.get("title") or d.get("label"), d.get("serial")) for d in devices])
        if [d.get("serial") for d in devices] != [d["serial"] for d in scenario["devices"]]:
            raise SystemExit("the scenario was not loaded: see app.log")
        # -- WP FUNC2c -- `[layout.<device>]` seeds another setup than the scenario's (S6: the old wiring)
        stale = self.story().get("layout", {})
        stale = stale if isinstance(stale, dict) else {}  # `layout = "classic"` is the app's layout (FUNC2b; INTI)
        # -- WP ADETS -- `speakers = "none"`: nobody set up the speakers, not even an unconfirmed
        # setup (AUTODES S19: the agent detects them)
        for dev, shown in zip(scenario["devices"], devices) if self.story().get("speakers") != "none" else []:
            seeded = dict(dev, outputs=stale[dev["name"]]["outputs"]) if dev["name"] in stale else dev
            self.layout(shown["id"], seeded, dev["name"] not in unconfirmed)
        # -- WP FUNC2a -- P1: each demo is a DSPi of its own (its serial; its layout kept under it)
        serials = [d.get("serial") for d in devices]
        if len(set(serials)) != len(serials):
            raise SystemExit(f"the demos share a serial: {serials}")
        # -- WP FUNC2a -- P8, P9a: what the user had done before (user.py)
        self.user = user.User(self)
        self.user.do_all(user.seed_actions(self.story()), "seed")
        self.user.show(self.story().get("show", scenario["devices"][0]["name"]))
        # The user turns agents on and chooses the questions
        ctl.set("AiController", "confirmMode", confirm)
        ctl.set("AiController", "showReads", True)
        # -- WP ADETS -- Settings › AI Agents › Agents may detect speakers (what never-ask needs)
        if self.story().get("agent_detect"):
            ctl.set("AiController", "agentDetect", True)
            ctl.wait("agents may detect speakers", lambda: ctl.get("AiController", "agentDetect"))
        ctl.call("AiController", "setEnabled", True)
        ctl.wait("AI agents on", lambda: ctl.get("AiController", "running"))
        log("AI agents on:", ctl.get("AiController", "addresses"), "questions:", confirm)
        self.meta.write_text(json.dumps({"name": self.name, "confirm": confirm, "unconfirmed": unconfirmed,
                                         "layout": layout,  # -- WP FUNC2b --
                                         "devices": devices, "session": None, "steps": []}, indent=1))
        (self.dir / "agent/mcp.json").write_text(json.dumps({"mcpServers": {"dspi": {
            "command": "podman", "args": ["exec", "-i", self.container,
                                          f"{CONTAINER_WORK}/build-functest/uconsole-for-dspi", "--mcp"]}}}, indent=1))
        # -- WP FBUS -- the story's host audio bus, if it has one (P11, bus.py)
        import bus
        bus.setup(self.dir, self.story().get("bus"))
        self.save("0-start")

    def layout(self, device_id, dev, confirmed):
        """The user's speaker setup of one device, on its Speakers page."""
        ctl = self.ctl
        ctl.call("DeviceRegistry", "activate", device_id)
        time.sleep(0.5)
        ctl.call("SpeakersController", "reload")
        ctl.wait("the speaker setup", lambda: ctl.get("SpeakersController", "loaded")
                 and ctl.get("SpeakersController", "rows"))
        time.sleep(0.5)
        wanted = {o["output"]: o for o in dev["outputs"]}
        for row in ctl.get("SpeakersController", "rows"):
            n = row["output"]
            ctl.call("SpeakersController", "setIncluded", n, n in wanted)
        for n, o in wanted.items():
            ctl.call("SpeakersController", "setRole", n, o["role"])
            ctl.call("SpeakersController", "setSpeaker", n, o["speaker"])
            ctl.call("SpeakersController", "setGroup", n, o["group"])
        if confirmed:
            ctl.call("SpeakersController", "confirm")
            ctl.wait("the layout confirmed", lambda: ctl.get("SpeakersController", "status") == "confirmed")
        else:
            # -- WP ROLES -- only Confirm keeps the page's draft: an unconfirmed setup is kept
            # row by row as the Measure tables' write-back keeps it (before, none was kept at all)
            for n, o in wanted.items():
                ctl.call("SpeakersController", "writeBack", n, o["role"], o["speaker"], o["group"])
                ctl.wait(f"{dev['name']} Out {n + 1} kept", lambda: ctl.call("SpeakersController", "roleOf", n) == o["role"])
        log(f"{dev['name']}: speaker setup {ctl.get('SpeakersController', 'status')}")

    # -- WP ADETS -- the user corrects rows of a (detected) setup on the Speakers page, as
    # Settings › Speakers does: `roles` = {output from 1: role}; it stays unconfirmed
    def correct_layout(self, name, roles):
        ctl = self.ctl
        ctl.call("DeviceRegistry", "activate", user.User(self).device_id(name))
        time.sleep(0.5)
        ctl.call("SpeakersController", "reload")
        ctl.wait("the speaker setup", lambda: ctl.get("SpeakersController", "loaded"))
        for n, role in roles.items():
            ctl.call("SpeakersController", "writeBack", int(n) - 1, role)
            ctl.wait(f"{name} Out {n} {role}", lambda: ctl.call("SpeakersController", "roleOf", int(n) - 1) == role)
        log(f"{name}: the user set {roles}; speaker setup {ctl.get('SpeakersController', 'status')}")

    # -- WP FUNC2a -- the user confirms a setup later (P9a)
    def confirm_layout(self, name):
        scenario = json.loads((self.dir / "scenario.json").read_text())
        dev = next(d for d in scenario["devices"] if d["name"] == name)
        self.layout(user.User(self).device_id(name), dev, True)
        # -- WP FUNC2c -- confirmed from now on: test signals before this moment break the
        # invariant (counted here), later ones are the user's leave (S6)
        if self.meta.exists():
            meta = json.loads(self.meta.read_text())
            if name in meta.get("unconfirmed", []):
                log_ = self.dir / "sim/oracle.jsonl"
                lines = [json.loads(x) for x in log_.read_text().splitlines()] if log_.exists() else []
                meta["unconfirmed"].remove(name)
                meta.setdefault("confirmed_later", {})[name] = {
                    "after_step": getattr(self, "step_label", "setup"),
                    # -- WP ADETS -- a signal the oracle judged by the core's knowledge (the
                    # unknown output's envelope, AUTODES 7.2) is not one the user had to allow first
                    "signals_before": sum(1 for x in lines if x["kind"] == "signal" and x["device"] == name
                                          and "knowledge" not in x)}
                self.meta.write_text(json.dumps(meta, indent=1))

    # ------------------------------------------------------- the user's answers

    def watch(self, answer=True):
        """Answer the app's questions as the user: pairing, and each change (by the story's
        `[[answer]]` patterns, else allowed; user.py)."""
        seen = set()
        answers = user.Answers(self.story().get("answer", []))  # -- WP FUNC2a --
        screen = {"last": None, "shots": {}}

        def loop():
            while not self.stop.is_set():
                try:
                    p = self.ctl.get("AiController", "pairing")
                    if p:
                        self.event("pairing", p)
                        self.ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{self.name}", prefix="pairing-")
                        self.ctl.call("AiController", "answerPairing", True)
                        time.sleep(1)
                    for q in self.ctl.get("AiController", "confirmations") or []:
                        if q["id"] in seen:
                            continue
                        seen.add(q["id"])
                        if len(seen) <= 3:
                            self.ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{self.name}",
                                          prefix=f"question-{len(seen)}-")
                        allow, rule, text = answers.answer(q)
                        self.event("question", dict(q, text=text, answer="allow" if allow else "deny", rule=rule))
                        self.stamp("question", text=text, answer="allow" if allow else "deny")  # -- WP SHOW --
                        if answer:
                            time.sleep(float(os.environ.get("FUNCTEST_READ_S", "1.5")))  # a person reads it
                            self.ctl.call("AiController", "answer", q["id"], allow)
                    self.screen(screen)  # -- WP FUNC2a --
                except (OSError, RuntimeError) as e:
                    if not self.stop.is_set():
                        self.event("watch", str(e))
                        time.sleep(2)
                self.stop.wait(0.4)

        self.watcher = threading.Thread(target=loop, daemon=True)
        self.watcher.start()

    # -- WP FUNC2a --
    def screen(self, seen):
        """What the app shows of the agent's work (AIVIS: the "agent working" strip, the device it
        names, whether that one is shown): each change to screen.jsonl, a few screenshots a step."""
        ai = self.ctl.get("AiController", "work") or {}
        now = {"working": self.ctl.get("AiController", "working"), "text": ai.get("text", ""),
               "device": ai.get("device", ""), "elsewhere": ai.get("elsewhere", False), "job": ai.get("job", "")}
        if now == seen["last"]:
            return
        seen["last"] = now
        step = getattr(self, "step_label", "setup")
        with open(self.dir / "screen.jsonl", "a") as f:
            f.write(json.dumps({"t": time.time(), "time": time.strftime("%H:%M:%S"), "step": step, **now}) + "\n")
        n = seen["shots"].get(step, 0)
        if now["working"] and now["text"] and n < 3:
            seen["shots"][step] = n + 1
            self.ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{self.name}", prefix=f"screen-{step}-{n + 1}-")

    # -- WP FUNC2c -- what the agent must never get or pass (FSTORY S7): the installer's PIN, its
    # recovery code, and (read at the end) the hash and salt kept beside the profile
    def secret(self, *values):
        path = self.dir / "secrets.json"
        have = json.loads(path.read_text()) if path.exists() else []
        path.write_text(json.dumps(have + [v for v in values if v and v not in have], indent=1))

    def secret_files(self):
        for f in list(self.dir.glob("home/**/limits.pin.json")) + list(self.dir.glob("home/**/limits.recovery.json")):
            try:
                rec = json.loads(f.read_text())
            except (OSError, ValueError):
                continue
            self.secret(*[rec.get(k) for k in ("salt", "hash") if isinstance(rec.get(k), str)])

    # -- WP SHOW -- the film: the host's ffmpeg on the container's Xvfb, from now until down()
    def record(self):
        w, h = SCREEN.split("x")
        self.ffmpeg = subprocess.Popen(
            ["ffmpeg", "-nostdin", "-loglevel", "error", "-f", "x11grab", "-draw_mouse", "0", "-framerate",
             os.environ.get("FUNCTEST_FPS", "20"), "-video_size", f"{w}x{h}", "-i", str(self.dir / "x11/X99"),
             "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", str(self.dir / "screen.mp4")],
            stdout=subprocess.DEVNULL, stderr=open(self.dir / "ffmpeg.log", "w"))
        self.stamp("record")
        log("filming the screen:", self.dir / "screen.mp4")

    def stamp(self, kind, **data):
        if getattr(self, "ffmpeg", None):
            with open(self.dir / "record.jsonl", "a") as f:
                f.write(json.dumps({"t": round(time.time(), 2), "kind": kind, **data}) + "\n")

    def stamp_calls(self, path, done):
        """The agent's tool calls as they come (stream-json has no times)."""
        seen = 0
        while not done.wait(0.3):
            try:
                lines = path.read_text().splitlines()
            except OSError:
                continue
            for line in lines[seen:]:
                try:
                    m = json.loads(line)
                except ValueError:
                    break  # a line still being written
                seen += 1
                for c in (m.get("message") or {}).get("content") or [] if m.get("type") == "assistant" else []:
                    if c.get("type") == "tool_use":
                        self.stamp("tool", tool=c["name"].replace("mcp__dspi__", ""), args=c.get("input"))
                    elif c.get("type") == "text":
                        self.stamp("text", text=c["text"])

    def event(self, kind, data):
        with open(self.dir / "user.jsonl", "a") as f:
            # -- WP INTI -- `at` (epoch s): a grant's window is judged against the agents' calls
            f.write(json.dumps({"time": time.strftime("%H:%M:%S"), "at": round(time.time(), 3), "kind": kind,
                                "data": data}) + "\n")

    # ------------------------------------------------------------- the agent

    def ask(self, label, prompt, timeout=3000):
        meta = json.loads(self.meta.read_text())
        n = len(meta["steps"]) + 1
        cmd = [AGENT, "-p", prompt, "--mcp-config", str(self.dir / "agent/mcp.json"), "--strict-mcp-config",
               "--allowedTools", "mcp__dspi", "--tools", os.environ.get("FUNCTEST_TOOLS", "ToolSearch,ListMcpResourcesTool,ReadMcpResourceTool"),
               "--setting-sources", "project", "--model", MODEL, "--output-format", "stream-json", "--verbose"]
        # A run is one session of its own: never resumed by another run, nothing remembered
        if meta["session"]:
            cmd += ["--resume", meta["session"]]
        else:
            meta["session"] = str(uuid.uuid4())
            cmd += ["--session-id", meta["session"]]
        path = self.dir / f"transcript-{n}-{label}.jsonl"
        self.step_label = label  # -- WP FUNC2a --
        log(f"step {n} ({label}): the agent works ...")
        start = time.time()
        env = dict(os.environ, MCP_TOOL_TIMEOUT="120000", MCP_TIMEOUT="60000",
                   CLAUDE_CODE_DISABLE_AUTO_MEMORY="1", CLAUDE_CODE_DISABLE_CLAUDE_MDS="1")
        self.stamp("prompt", label=label, text=prompt)  # -- WP SHOW --
        calls_done = threading.Event()
        if getattr(self, "ffmpeg", None):
            threading.Thread(target=self.stamp_calls, args=(path, calls_done), daemon=True).start()
        with open(path, "w") as f, open(self.dir / "agent/stderr.log", "a") as err:
            try:
                p = subprocess.run(cmd, cwd=self.dir / "agent", stdout=f, stderr=err, stdin=subprocess.DEVNULL,
                                   timeout=timeout, env=env)
                code = p.returncode
            except subprocess.TimeoutExpired:
                code = "timeout"
        time.sleep(0.5)
        calls_done.set()
        self.stamp("answer", label=label)
        result, session, cost = "", meta["session"], None
        for line in path.read_text().splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            session = m.get("session_id") or session
            if m.get("type") == "result":
                result = m.get("result", "")
                cost = m.get("total_cost_usd")  # -- WP FUNC2a --
        meta["session"] = session
        meta["steps"].append({"n": n, "label": label, "prompt": prompt, "code": code,
                              "seconds": round(time.time() - start), "answer": result,
                              "start": start, "end": time.time(), "cost_usd": cost})
        self.meta.write_text(json.dumps(meta, indent=1))
        (self.dir / f"answer-{n}-{label}.md").write_text(f"# {label}\n\n> {prompt}\n\n{result}\n")
        self.save(f"{n}-{label}")
        (self.dir / f"transcript-{n}-{label}.md").write_text(readable(path))
        log(f"step {n} ended ({code}) after {round(time.time() - start)} s:", tools_used(path))
        return code

    def replay(self, label, calls, player):
        """A step of a replay script, in place of the agent."""
        meta = json.loads(self.meta.read_text())
        n = len(meta["steps"]) + 1
        self.step_label = label  # -- WP FUNC2a --
        log(f"step {n} ({label}): the replay plays {len(calls)} calls ...")
        start = time.time()
        try:
            done, code = player.step(calls), 0
        except (RuntimeError, ValueError) as e:
            done, code = [], f"replay: {e}"
        meta["steps"].append({"n": n, "label": label, "prompt": "(replay)", "code": code,
                              "seconds": round(time.time() - start), "answer": "", "replay": done,
                              "start": start, "end": time.time()})
        self.meta.write_text(json.dumps(meta, indent=1))
        self.save(f"{n}-{label}")
        log(f"step {n} ended ({code}) after {round(time.time() - start)} s:",
            [(x["tool"], x["got"]) for x in done if x["wrong"]] or "as the script says")
        return code

    # -- WP FUNC2a --
    def probe(self, label, calls):
        """Direct MCP calls after a step, as another client (FSTORY S9: the harness tries the
        path itself): judged like a replay's, kept in the step's `probe`."""
        if not getattr(self, "prober", None):
            self.prober = replay.Player(self, client="functest-probe")
        try:
            done = self.prober.step(calls)
        except (RuntimeError, ValueError) as e:
            done = [{"tool": "probe", "args": {}, "want": "ok", "got": str(e), "wrong": True, "note": "", "answer": ""}]
        meta = json.loads(self.meta.read_text())
        meta["steps"][-1]["probe"] = done
        self.meta.write_text(json.dumps(meta, indent=1))
        log(f"probe after {label}:", [(x["tool"], x["got"]) for x in done if x["wrong"]] or "as the story says")

    # -------------------------------------------------------------- the state

    def save(self, label):
        time.sleep(1.0)
        d = self.dir / "state" / label
        d.mkdir(parents=True, exist_ok=True)
        for f in ("truth.json", "oracle.jsonl", "config-oracle.jsonl"):
            if (self.dir / "sim" / f).exists():
                shutil.copy(self.dir / "sim" / f, d / f)
        if (d / "truth.json").exists():
            t = json.loads((d / "truth.json").read_text())
            for dev in t["devices"]:
                (d / f"config-{dev['name']}.json").write_text(json.dumps(dev["state"], indent=1))
        activity = self.ctl.get("AiController", "activity")
        (d / "activity.json").write_text(json.dumps(activity, indent=1))
        # -- WP FUNC2b -- the shown device's presets, snapshots and names as the app shows them
        (d / "app.json").write_text(json.dumps(self.app_view(), indent=1))
        for f in self.dir.glob("app-*.json") if label == "end" else []:
            shutil.copy(f, d / f.name)
        subprocess.run([sys.executable, str(HERE / "judge.py"), str(d), str(self.dir / "scenario.json")],
                       stdout=open(d / "judge.txt", "w"))

    def history(self):
        for d in self.ctl.get("DeviceRegistry", "devices"):
            self.ctl.call("DeviceRegistry", "activate", d["id"])
            time.sleep(1)
            name = d.get("title") or d.get("label") or d["id"]
            (self.dir / f"history-{name}.json").write_text(
                json.dumps(self.ctl.get("HistoryController", "entries"), indent=1))
            (self.dir / f"app-{name}.json").write_text(json.dumps(self.app_view(), indent=1))  # -- WP FUNC2b --

    # -- WP FUNC2b --
    def app_view(self):
        """The shown device as the app shows it: preset slots (from 0; "Empty", "Preset n" or the
        name, "*" on the active one while unsaved), start-up, snapshots, A/B, names."""
        ctl, v = self.ctl, {}
        try:
            v["device"] = next((d.get("title") or d.get("label") for d in ctl.get("DeviceRegistry", "devices")
                                if d.get("active")), None)
            v["deviceName"] = ctl.get("NamesController", "deviceName")
            v["presets"] = [ctl.call("SidebarController", "presetTitle", i) for i in range(10)]
            v["occupied"] = [ctl.call("SidebarController", "presetOccupied", i) for i in range(10)]
            for k in ("activeSlot", "startupMode", "defaultSlot"):
                v[k] = ctl.get("DeviceModel", f"presets.{k}")
            v["unsaved"] = ctl.get("SidebarController", "unsaved")
            v["sources"] = ctl.get("NamesController", "sources")
            v["snapshots"] = ctl.get("SnapshotsController", "snapshots")
            v["ab"] = {k: ctl.get("SnapshotsController", k) for k in ("abActive", "abSide", "abAName", "abBName")}
            # What the preset box on screen says (Modern's top bar, Classic's sidebar)
            for box in ("TopBar.preset", "GlobalControls.preset"):
                try:
                    v["presetShown"] = ctl.get(box, "shownName")
                    break
                except RuntimeError:
                    pass
        except (OSError, RuntimeError) as e:
            v["error"] = str(e)
        return v

    def down(self):
        self.stop.set()
        if getattr(self, "user", None):
            self.user.join()
        try:
            self.history()
            self.ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{self.name}", prefix="end-")
            if getattr(self, "ffmpeg", None):  # -- WP SHOW --
                self.stamp("end")
                self.ffmpeg.send_signal(signal.SIGINT)
                try:
                    self.ffmpeg.wait(30)
                except subprocess.TimeoutExpired:
                    self.ffmpeg.kill()
            self.save("end")
            self.secret_files()  # -- WP FUNC2c --
            self.ctl.send(op="quit")
            time.sleep(3)
        except (OSError, RuntimeError) as e:
            log("down:", e)
        subprocess.run(["podman", "rm", "-f", self.container], capture_output=True)

    def verdict(self):
        """The three layers (checks.py)."""
        if not self.meta.exists():  # -- WP FUNC2a -- it never came up
            print("the run did not start", flush=True)
            return False
        ok, text = checks.judge_run(self.dir)
        print(text, flush=True)
        return ok


def tools_used(path):
    n = {}
    for line in pathlib.Path(path).read_text().splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        for c in (m.get("message") or {}).get("content") or [] if m.get("type") == "assistant" else []:
            if c.get("type") == "tool_use":
                k = c["name"].replace("mcp__dspi__", "")
                n[k] = n.get(k, 0) + 1
    return dict(sorted(n.items(), key=lambda x: -x[1]))


def readable(path, cut=2500):
    """A transcript to read: what the agent said, called and got."""
    out = []
    for line in pathlib.Path(path).read_text().splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("type") not in ("assistant", "user"):
            continue
        content = (m.get("message") or {}).get("content") or []
        if isinstance(content, str):
            content = [{"type": "text", "text": content}]
        for c in content:
            if c.get("type") == "text":
                out.append(f"**{m['type']}:** {c['text']}\n")
            elif c.get("type") == "thinking" and c.get("thinking"):
                out.append(f"*thinking:* {c['thinking']}\n")
            elif c.get("type") == "tool_use":
                out.append(f"→ `{c['name'].replace('mcp__dspi__', '')}` `{json.dumps(c['input'])[:cut]}`\n")
            elif c.get("type") == "tool_result":
                t = c.get("content")
                if isinstance(t, list):
                    t = "\n".join(x.get("text", "") for x in t if isinstance(x, dict))
                t = str(t)
                mark = "✗" if c.get("is_error") else "←"
                out.append(f"{mark} {t[:cut]}{' …' if len(t) > cut else ''}\n")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["run", "up", "ask", "save", "down", "verdict"])
    ap.add_argument("text", nargs="*")
    ap.add_argument("--name", default="run")
    ap.add_argument("--story", default=str(HERE / "stories/home-theatre.toml"))
    ap.add_argument("--confirm", default=None, choices=["never_ask", "ask_risky", "ask_everything"])
    ap.add_argument("--steps", default="", help="which of the story's steps, 1,2,3 (default all)")
    ap.add_argument("--replay", default="", help="a replay script in place of the agent")
    ap.add_argument("--label", default="ask")
    ap.add_argument("--keep", action="store_true", help="leave the app running")
    ap.add_argument("--layout", choices=["modern", "classic"], help="the app's layout (default: the story's, else modern)")
    ap.add_argument("--record", action="store_true", help="film the app on a screen of its own (WP SHOW)")
    a = ap.parse_args()
    if a.record:  # -- WP SHOW --
        global SCREEN
        SCREEN = SCREEN or "1600x1000"
        os.environ["FUNCTEST_RECORD"] = "1"
    if a.layout:  # -- WP FUNC2b --
        os.environ["FUNCTEST_LAYOUT"] = a.layout
    run = Run(a.name)
    if a.what == "verdict":
        sys.exit(0 if run.verdict() else 1)
    # -- WP FUNC2a -- a stopped run still takes its container down (finally)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    scripted = json.loads(pathlib.Path(a.replay).read_text()) if a.replay else {}
    confirm = a.confirm or scripted.get("confirm") \
        or tomllib.loads(pathlib.Path(a.story).read_text()).get("confirm") or "never_ask"
    if a.what == "up":
        try:
            run.up(confirm, a.story)
        except BaseException:
            subprocess.run(["podman", "rm", "-f", run.container], capture_output=True)
            raise
        return
    wanted = [int(x) for x in a.steps.split(",") if x]
    if a.what == "run":
        ok = False
        try:
            run.up(confirm, a.story)
            run.watch()
            script = scripted.get("steps")
            player = replay.Player(run) if script is not None else None
            for i, step in enumerate(run.story().get("step", []), 1):
                if (wanted and i not in wanted) or (script is not None and step["label"] not in script):
                    continue
                run.user.do_all(step.get("user", []), step["label"])  # -- WP FUNC2a -- P9a
                code = run.replay(step["label"], script[step["label"]], player) if script is not None \
                    else run.ask(step["label"], step["prompt"])
                run.user.join()
                if code == 0 and step.get("probe"):
                    run.probe(step["label"], step["probe"])  # -- WP FUNC2a --
                if code != 0:
                    break
            if run.story().get("after"):
                run.user.do_all(run.story()["after"], "after")
                run.save("after")
        finally:
            if not a.keep:
                run.down()
            ok = run.verdict()
        sys.exit(0 if ok else 1)
    run.watch()
    if a.what == "ask":
        for i, step in enumerate(run.story().get("step", []), 1):
            if i in wanted:
                run.ask(step["label"], step["prompt"])
        if not wanted:
            run.ask(a.label, " ".join(a.text))
        run.verdict()
    elif a.what == "save":
        run.save(a.text[0] if a.text else a.label)
    elif a.what == "down":
        run.down()
        run.verdict()


if __name__ == "__main__":
    import checks
    import replay
    import user
    main()
