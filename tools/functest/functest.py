#!/usr/bin/env python3
"""The function test: a real AI agent, or a replay of one, tunes a simulated home
theatre through the desktop app's MCP server (README.md). Host side: standard
library only; the app runs in a container with no USB, no sound device and no
network. run.sh is the way in.

    functest.py run  --name r1 [--story F] [--confirm never_ask|ask_risky] [--steps 1,2,3] [--replay F] [--keep]
    functest.py up   --name r1 ...         the app, set up, left running
    functest.py ask  --name r1 --steps 1   the story's prompt, or: ask --name r1 --label l "the words"
    functest.py save --name r1 label       the state now
    functest.py down --name r1             History, screenshots, stop, the verdict
"""
import argparse
import json
import os
import pathlib
import shutil
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
WORK = CACHE / "work" / BRANCH
AGENT = os.environ.get("FUNCTEST_AGENT", "claude2")
MODEL = os.environ.get("FUNCTEST_MODEL", "opus")
CONTAINER_WORK = "/work"


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
        shutil.copy(HERE / "scenario.json", self.dir / "scenario.json")
        shutil.copy(story, self.dir / "story.toml")
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
        for k, v in env.items():
            cmd += ["-e", f"{k}={v}"]
        cmd += [image(), "bash", "-c",
                f"exec {CONTAINER_WORK}/build-functest/uconsole-for-dspi > {c}/app.log 2>&1"]
        subprocess.run(cmd, check=True, capture_output=True)
        log("the app starts in", self.container)
        ctl = self.ctl
        ctl.wait("the app's socket", lambda: ctl.get("AiController", "port"), 60)
        ctl.set("LayoutController", "layout", "modern")
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
        for dev, shown in zip(scenario["devices"], devices):
            self.layout(shown["id"], dev, dev["name"] not in unconfirmed)
        ctl.call("DeviceRegistry", "activate", devices[0]["id"])
        # The user turns agents on and chooses the questions
        ctl.set("AiController", "confirmMode", confirm)
        ctl.set("AiController", "showReads", True)
        ctl.call("AiController", "setEnabled", True)
        ctl.wait("AI agents on", lambda: ctl.get("AiController", "running"))
        log("AI agents on:", ctl.get("AiController", "addresses"), "questions:", confirm)
        self.meta.write_text(json.dumps({"name": self.name, "confirm": confirm, "unconfirmed": unconfirmed,
                                         "devices": devices, "session": None, "steps": []}, indent=1))
        (self.dir / "agent/mcp.json").write_text(json.dumps({"mcpServers": {"dspi": {
            "command": "podman", "args": ["exec", "-i", self.container,
                                          f"{CONTAINER_WORK}/build-functest/uconsole-for-dspi", "--mcp"]}}}, indent=1))
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
        log(f"{dev['name']}: speaker setup {ctl.get('SpeakersController', 'status')}")

    # ------------------------------------------------------- the user's answers

    def watch(self, answer=True):
        """Answer the app's questions as the user: pairing, and each change (allowed)."""
        seen = set()

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
                        self.event("question", q)
                        if answer:
                            time.sleep(1.5)  # a person reads it
                            self.ctl.call("AiController", "answer", q["id"], True)
                except (OSError, RuntimeError) as e:
                    if not self.stop.is_set():
                        self.event("watch", str(e))
                        time.sleep(2)
                self.stop.wait(0.4)

        self.watcher = threading.Thread(target=loop, daemon=True)
        self.watcher.start()

    def event(self, kind, data):
        with open(self.dir / "user.jsonl", "a") as f:
            f.write(json.dumps({"time": time.strftime("%H:%M:%S"), "kind": kind, "data": data}) + "\n")

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
        log(f"step {n} ({label}): the agent works ...")
        start = time.time()
        env = dict(os.environ, MCP_TOOL_TIMEOUT="120000", MCP_TIMEOUT="60000",
                   CLAUDE_CODE_DISABLE_AUTO_MEMORY="1", CLAUDE_CODE_DISABLE_CLAUDE_MDS="1")
        with open(path, "w") as f, open(self.dir / "agent/stderr.log", "a") as err:
            try:
                p = subprocess.run(cmd, cwd=self.dir / "agent", stdout=f, stderr=err, stdin=subprocess.DEVNULL,
                                   timeout=timeout, env=env)
                code = p.returncode
            except subprocess.TimeoutExpired:
                code = "timeout"
        result, session = "", meta["session"]
        for line in path.read_text().splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            session = m.get("session_id") or session
            if m.get("type") == "result":
                result = m.get("result", "")
        meta["session"] = session
        meta["steps"].append({"n": n, "label": label, "prompt": prompt, "code": code,
                              "seconds": round(time.time() - start), "answer": result})
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
        log(f"step {n} ({label}): the replay plays {len(calls)} calls ...")
        start = time.time()
        try:
            done, code = player.step(calls), 0
        except (RuntimeError, ValueError) as e:
            done, code = [], f"replay: {e}"
        meta["steps"].append({"n": n, "label": label, "prompt": "(replay)", "code": code,
                              "seconds": round(time.time() - start), "answer": "", "replay": done})
        self.meta.write_text(json.dumps(meta, indent=1))
        self.save(f"{n}-{label}")
        log(f"step {n} ended ({code}) after {round(time.time() - start)} s:",
            [(x["tool"], x["got"]) for x in done if x["wrong"]] or "as the script says")
        return code

    # -------------------------------------------------------------- the state

    def save(self, label):
        time.sleep(1.0)
        d = self.dir / "state" / label
        d.mkdir(parents=True, exist_ok=True)
        for f in ("truth.json", "oracle.jsonl"):
            if (self.dir / "sim" / f).exists():
                shutil.copy(self.dir / "sim" / f, d / f)
        if (d / "truth.json").exists():
            t = json.loads((d / "truth.json").read_text())
            for dev in t["devices"]:
                (d / f"config-{dev['name']}.json").write_text(json.dumps(dev["state"], indent=1))
        activity = self.ctl.get("AiController", "activity")
        (d / "activity.json").write_text(json.dumps(activity, indent=1))
        subprocess.run([sys.executable, str(HERE / "judge.py"), str(d), str(self.dir / "scenario.json")],
                       stdout=open(d / "judge.txt", "w"))

    def history(self):
        for d in self.ctl.get("DeviceRegistry", "devices"):
            self.ctl.call("DeviceRegistry", "activate", d["id"])
            time.sleep(1)
            name = d.get("title") or d.get("label") or d["id"]
            (self.dir / f"history-{name}.json").write_text(
                json.dumps(self.ctl.get("HistoryController", "entries"), indent=1))

    def down(self):
        self.stop.set()
        try:
            self.history()
            self.ctl.send(op="grab", dir=f"{CONTAINER_WORK}/runs/{self.name}", prefix="end-")
            self.save("end")
            self.ctl.send(op="quit")
            time.sleep(3)
        except (OSError, RuntimeError) as e:
            log("down:", e)
        subprocess.run(["podman", "rm", "-f", self.container], capture_output=True)

    def verdict(self):
        """The three layers (checks.py)."""
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
    ap.add_argument("--confirm", default="never_ask", choices=["never_ask", "ask_risky", "ask_everything"])
    ap.add_argument("--steps", default="", help="which of the story's steps, 1,2,3 (default all)")
    ap.add_argument("--replay", default="", help="a replay script in place of the agent")
    ap.add_argument("--label", default="ask")
    ap.add_argument("--keep", action="store_true", help="leave the app running")
    a = ap.parse_args()
    run = Run(a.name)
    if a.what == "verdict":
        sys.exit(0 if run.verdict() else 1)
    if a.what in ("run", "up"):
        run.up(a.confirm, a.story)
        if a.what == "up":
            return
    run.watch()
    wanted = [int(x) for x in a.steps.split(",") if x]
    if a.what == "run":
        ok = False
        try:
            script = json.loads(pathlib.Path(a.replay).read_text())["steps"] if a.replay else None
            player = replay.Player(run) if script else None
            for i, step in enumerate(run.story().get("step", []), 1):
                if (wanted and i not in wanted) or (script and step["label"] not in script):
                    continue
                code = run.replay(step["label"], script[step["label"]], player) if script else run.ask(step["label"], step["prompt"])
                if code != 0:
                    break
        finally:
            if not a.keep:
                run.down()
            ok = run.verdict()
        sys.exit(0 if ok else 1)
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
    main()
