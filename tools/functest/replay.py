#!/usr/bin/env python3
"""The replay agent: MCP tool calls from a file, no model (README.md).

    replay.py RUN_DIR > replay/NAME.json      a recorded agent run as a replay script

A script is {"steps": {label: [[tool, args, "ok" | "error", note?, not?, must?], ...]}}: `not` is a pattern
the answer must not have ("" for none), `must` one it must have (-- WP FUNC2b --); ["wait", {"device": d}]
waits until that device's measuring ends, in place of an agent's polling. A measurement's id in the
arguments is "$M1", "$M2", ...: the ones this run filed, in the order they were filed.
-- WP FUNC2b -- "$SNAP:<name>" is the id of the call's device's newest snapshot of that name.
-- WP FUNC2a -- ["sleep", {"s": n}] waits n seconds (a user's drag lets go meanwhile); "$USER:<device>"
is the id of that device's newest change the user made (dspi_get_history, `mine` false, no agent).
A script may name its `story` (a file in stories/) and its `confirm` setting.
-- WP FUNC2c -- [tool, args, want, note, not, must]: `must` is a pattern the answer must have
(`not` may be "" then).
"""
import json
import pathlib
import re
import sys
import time

from checks import tool_calls

# What an agent reads changes nothing: left out of a conversion
READS = ("dspi_get_", "dspi_list_", "dspi_measure_status", "dspi_measure_preflight", "dspi_system_measure_preflight")


def convert(run):
    """A recorded run's calls as a script."""
    run = pathlib.Path(run)
    meta = json.loads((run / "run.json").read_text())
    ids, steps = {}, {}
    for s in meta["steps"]:
        calls, last_start = [], None
        path = run / f"transcript-{s['n']}-{s['label']}.jsonl"
        text = path.read_text() if path.exists() else ""
        for i in filed(text):
            ids.setdefault(i, f"$M{len(ids) + 1}")
        for tool, args, ok in step_calls(run, s):
            if tool.startswith(READS):
                if tool == "dspi_measure_status" and last_start:
                    calls.append(["wait", {"device": last_start}])
                    last_start = None
                continue
            args = json.loads(replace(json.dumps(args), ids))
            calls.append([tool, args, "ok" if ok else "error"])
            if tool in ("dspi_measure_start", "dspi_measure_rolloff_start", "dspi_tune_sum_check", "dspi_tune_verify",
                        "dspi_detect_speakers") and ok:  # -- WP ADETS --
                last_start = args.get("device", "")
        steps[s["label"]] = calls
    return {"from": meta["name"], "confirm": meta["confirm"], "steps": steps}


def replace(text, ids):
    for k, v in sorted(ids.items(), key=lambda x: -len(x[0])):
        text = text.replace(f'"{k}"', f'"{v}"')
    return text


def filed(text):
    """The measurement ids a transcript saw filed, in order."""
    out = []
    for i in re.findall(r'filed_measurement_id\\*":\\*"([0-9-]+)', text):
        if i not in out:
            out.append(i)
    return out


def step_calls(run, step):
    """tool_calls of one step's transcript."""
    import shutil
    import tempfile
    d = pathlib.Path(tempfile.mkdtemp(dir=run))
    try:
        src = run / f"transcript-{step['n']}-{step['label']}.jsonl"
        if src.exists():
            shutil.copy(src, d / src.name)
        return tool_calls(d)
    finally:
        shutil.rmtree(d)


class Player:
    """Plays a script through the app's relay, as an agent."""

    def __init__(self, run, client="probe"):
        from probe import Agent
        self.agent = Agent(run.name, client)
        self.ids = []

    # -- WP FUNC2a --
    def user_entry(self, device):
        """The id of `device`'s newest change by the user (not this client's, no agent's)."""
        kind, text = self.agent.call("dspi_get_history", device=device, n=50)
        for e in json.loads(text).get("entries", []) if kind == "ok" else []:
            if e.get("actor") == 0:
                return e["id"]
        raise RuntimeError(f"{device}: no change of the user's in the history: {str(text)[:300]}")

    # -- WP FUNC2b --
    def snapshot(self, device, name):
        """The id of `device`'s newest snapshot called `name`."""
        kind, text = self.agent.call("dspi_list_snapshots", device=device)
        found = [x for x in json.loads(text).get("snapshots", []) if x.get("snapshot_name") == name] if kind == "ok" else []
        if not found:
            raise RuntimeError(f"{device}: no snapshot {name!r}: {str(text)[:300]}")
        return max(found, key=lambda x: x.get("saved_ms", 0))["id"]

    def status(self, device, wait_s=0):
        args = {"device": device}
        if wait_s:
            args["wait_s"] = wait_s
        return json.loads(self.agent.call("dspi_measure_status", **args)[1])

    def wait(self, device, seconds=900):
        """Long-polls (wait_s, FFIX) as the guide tells agents to."""
        end = time.time() + seconds
        while time.time() < end:
            s = self.status(device, wait_s=25)
            req = s.get("request") or {}
            if req.get("state") not in ("waiting_for_app", "starting", "running"):
                i = req.get("filed_measurement_id")
                if i and i not in self.ids:
                    self.ids.append(i)
                return req.get("state", ""), req.get("note", "")
        raise RuntimeError(f"{device} still measures after {seconds} s")

    def step(self, calls):
        """Plays one step: what did not go as the script says."""
        out = []
        for c in calls:
            tool, args = c[0], c[1]
            if tool.startswith("bus"):  # -- WP FBUS -- the host audio bus (bus.py)
                import bus
                out.append(bus.replay_call(self.agent.run.dir, c))
                continue
            if tool == "wait":
                state, note = self.wait(args["device"])
                out.append({"tool": tool, "args": args, "want": "ended", "got": f"{state}: {note}",
                            "wrong": state != "ended", "note": "", "answer": note})
                continue
            if tool == "sleep":
                time.sleep(float(args["s"]))
                continue
            if tool == "mcp_read_all":  # -- WP ODEC -- tools, resources and prompts, logged
                n = self.agent.read_all()
                out.append({"tool": tool, "args": args, "want": "ok", "got": "ok" if n else "error",
                            "wrong": not n, "note": "", "answer": f"{n} items"})
                continue
            text = json.dumps(args)
            for dev in re.findall(r'"\$USER:([^"]+)"', text):
                text = text.replace(f'"$USER:{dev}"', str(self.user_entry(dev)))
            for name in re.findall(r'"\$SNAP:([^"]+)"', text):  # -- WP FUNC2b --
                text = text.replace(f'"$SNAP:{name}"', json.dumps(self.snapshot(args["device"], name)))
            for n in range(len(self.ids), 0, -1):
                text = text.replace(f'"$M{n}"', json.dumps(self.ids[n - 1]))
            kind, answer = self.agent.call(tool, **json.loads(text))
            got = "ok" if kind == "ok" else "error"
            banned = len(c) > 4 and c[4] and re.search(c[4], f"{answer} {self.agent.last_news}")
            if banned:
                got = f"{got}, with {banned.group(0)[:60]!r}"
            # -- WP ODEC -- (FUNC2b, FUNC2c the same on the answer alone) a sixth element: a pattern the answer (with the user's news) must have
            if len(c) > 5 and not re.search(c[5], str(answer) + " " + str(self.agent.last_news)):
                got = f"{got}, without {c[5][:60]!r}"
            out.append({"tool": tool, "args": args, "want": c[2], "got": got, "wrong": got != c[2],
                        "note": c[3] if len(c) > 3 else "", "answer": str(answer)[:400]})
        return out


if __name__ == "__main__":
    print(json.dumps(convert(sys.argv[1]), indent=1))
