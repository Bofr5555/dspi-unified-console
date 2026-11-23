#!/usr/bin/env python3
"""The three layers a run is judged by (README.md):

  (a) invariants: hard, deterministic, the same for every story; a breach fails the run;
  (b) the story's expectations: the end state after each step, with tolerances, pass, near or fail;
  (c) the agent's words, printed for a person to read; never judged (but `says`, where a story
      asks for one thing to be said).

    checks.py RUN_DIR           judge a run again (a run's own story and scenario are in it)
"""
import json
import math
import re
import pathlib
import sys
import time
import tomllib

from judge import NAMES, band, smooth

STARTS = {"dspi_measure_start", "dspi_measure_rolloff_start", "dspi_tune_sum_check", "dspi_tune_verify",
          "dspi_system_measure_start", "dspi_system_measure_verify"}


class State:
    """One saved state: truth.json and where the oracle's log stood."""

    def __init__(self, path):
        t = json.loads((path / "truth.json").read_text())
        self.t, self.f = t, t["freqsHz"]
        self.devices = {d["name"]: d for d in t["devices"]}
        o = path / "oracle.jsonl"
        self.oracle = [json.loads(x) for x in o.read_text().splitlines()] if o.exists() else []
        a = path / "activity.json"  # -- WP FUNC2a --
        self.activity = json.loads(a.read_text()) if a.exists() else []

    def out(self, device, n):
        return next(o for o in self.devices[device]["outputs"] if o["output"] == n - 1)

    def seat(self, name):
        return self.t["inputsDb"][NAMES.index(name)]

    def edge(self, o, side):
        """The electrical -6 dB point on one side of an output's passband (None: the band's end)."""
        e = o["electricDb"]
        top = max(e)
        idx = range(len(e)) if side == "low" else range(len(e) - 1, -1, -1)
        i = next(i for i in idx if e[i] >= top - 6)
        ends = (0,) if side == "low" else (len(e) - 1, len(e) - 2)
        return None if i in ends else self.f[i]

    def slope(self, o, side):
        """dB per octave an octave outside the edge."""
        hz = self.edge(o, side)
        if not hz:
            return 0.0
        at = lambda x: o["electricDb"][min(range(len(self.f)), key=lambda i: abs(math.log(self.f[i] / x)))]
        a, b = (hz / 2, hz / 4) if side == "low" else (hz * 2, hz * 4)
        return abs(at(a) - at(b)) if b <= 20000 and b >= 20 else 24.0


def score(value, ok, near):
    return "pass" if ok(value) else "near" if near(value) else "fail"


def worst(scores):
    for s in ("fail", "near", "pass"):
        if s in scores:
            return s
    return "pass"


def within(lo, hi, near):
    return (lambda v: v is not None and lo <= v <= hi,
            lambda v: v is not None and lo * (1 - near) <= v <= hi * (1 + near))


def every(items, fmt):
    """(name, value, score) rows to one score and one line."""
    return worst([s for _, _, s in items]), ", ".join(fmt(n, v) + ("" if s == "pass" else f" [{s}]") for n, v, s in items)


def check(e, now, before, steps, words):
    """One expectation: (score, detail)."""
    k = e["kind"]
    hzt = lambda n, v: f"{n} {'none' if v is None else f'{v:.0f} Hz'}"
    if k == "measured":
        got = {}
        for x in now.oracle[len(before.oracle) if before else 0:]:
            if x["kind"] in ("signal", "VIOLATION"):
                got.setdefault(x["device"], set()).add(x["output"] + 1)
        want = {d: set(v) for d, v in e["outputs"].items()}
        missing = {d: sorted(v - got.get(d, set())) for d, v in want.items() if v - got.get(d, set())}
        extra = {d: sorted(v) for d, v in got.items()} if not want else {}
        return ("fail" if missing or extra else "pass",
                f"not measured: {missing}" if missing else f"played: {extra}" if extra else "as expected")
    if k == "highpass":
        rows = []
        for d, outs in e["outputs"].items():
            for n in outs:
                o = now.out(d, n)
                hz, sl = now.edge(o, "low"), now.slope(o, "low")
                ok = lambda v: v is not None and v >= e["min_hz"] * 0.97 and sl >= e["slope_db"] - 2
                near = lambda v: v is not None and v >= e["near_hz"] and sl >= e["slope_db"] - 3
                rows.append((f"{d} {n}", hz, score(hz, ok, near)))
        return every(rows, hzt)
    if k == "lowpass":
        rows = []
        for d, outs in e["outputs"].items():
            for n in outs:
                hz = now.edge(now.out(d, n), "high")
                rows.append((f"{d} {n}", hz, score(hz, *within(*e["hz"], e["near"]))))
        return every(rows, hzt)
    if k == "crossover":
        rows = []
        for lo, hi in e["pairs"]:
            a, b = now.edge(now.out(e["device"], lo), "high"), now.edge(now.out(e["device"], hi), "low")
            s = worst([score(a, *within(*e["hz"], e["near"])), score(b, *within(*e["hz"], e["near"]))])
            if a and b and not 0.75 <= a / b <= 1.34:
                s = "fail"
            rows.append((f"{lo}|{hi}", (a, b), s))
        return every(rows, lambda n, v: f"{n}: {hzt('', v[0])} /{hzt('', v[1])}")
    if k == "routes":
        missing = [f"{d} {i} to {n}" for d, i, outs in e["routes"] for n in outs
                   if NAMES.index(i) not in now.out(d, n)["inputs"] or not now.out(d, n)["enabled"]]
        return "fail" if missing else "pass", "missing: " + "; ".join(missing) if missing else "all there"
    if k == "holes":
        rows = []
        for i in e["inputs"]:
            sm = smooth(now.f, now.seat(i), 1 / 3)
            wide = smooth(now.f, now.seat(i), 2)
            lo, hi = e["band_hz"]
            v = min((a - b for hz, a, b in zip(now.f, sm, wide) if lo <= hz <= hi), default=-99)
            v = -99 if not math.isfinite(v) else v
            rows.append((i, v, score(v, lambda v: v >= e["pass_db"], lambda v: v >= e["near_db"])))
        return every(rows, lambda n, v: f"{n} {v:+.1f} dB")
    if k in ("tone", "alike"):
        tone = {i: band(now.f, now.seat(i), *e["high_hz"]) - band(now.f, now.seat(i), *e["low_hz"]) for i in e["inputs"]}
        if k == "tone":
            rows = [(i, v, score(v, lambda v: e["pass_db"][0] <= v <= e["pass_db"][1],
                                 lambda v: e["near_db"][0] <= v <= e["near_db"][1])) for i, v in tone.items()]
            return every(rows, lambda n, v: f"{n} {v:+.1f} dB")
        v = max(tone.values()) - min(tone.values())
        return (score(v, lambda v: v <= e["pass_db"], lambda v: v <= e["near_db"]),
                f"spread {v:.1f} dB: " + ", ".join(f"{i} {x:+.1f}" for i, x in tone.items()))
    if k == "levels":
        lv = {i: band(now.f, now.seat(i), *e["band_hz"]) for i in e["inputs"]}
        mid = sorted(lv.values())[len(lv) // 2] if len(lv) > 2 else sum(lv.values()) / 2
        rows = [(i, v - mid, score(abs(v - mid), lambda v: v <= e["pass_db"], lambda v: v <= e["near_db"])) for i, v in lv.items()]
        return every(rows, lambda n, v: f"{n} {v:+.1f} dB")
    if k == "arrivals":
        when = {f"{d}/{o['speaker']}": [] for d, dev in now.devices.items() for o in dev["outputs"]}
        for d, dev in now.devices.items():
            for o in dev["outputs"]:
                when[f"{d}/{o['speaker']}"].append(o["arrivalMs"])
        rows = []
        for g in e["groups"]:
            v = [sum(when[s]) / len(when[s]) for s in g]
            rows.append((" & ".join(x.split("/")[1] for x in g), max(v) - min(v),
                         score(max(v) - min(v), lambda v: v <= e["pass_ms"], lambda v: v <= e["near_ms"])))
        return every(rows, lambda n, v: f"{n} {v:.2f} ms apart")
    if k == "band_change":
        level = lambda s: band(s.f, s.seat(e["input"]), *e["band_hz"]) - (
            band(s.f, s.seat(e["input"]), *e["against_hz"]) if "against_hz" in e else 0)
        v = level(now) - level(before)
        return (score(v, lambda v: e["pass_db"][0] <= v <= e["pass_db"][1], lambda v: e["near_db"][0] <= v <= e["near_db"][1]),
                f"{v:+.1f} dB")
    if k == "band_limited":
        lo = now.edge(now.out(e["device"], e["low_output"]), "low")
        hi = now.edge(now.out(e["device"], e["high_output"]), "high")
        # Or on the input, before the crossovers
        for b in now.devices[e["device"]]["state"]["filters"][NAMES.index(e["input"])]:
            if not b["bypass"] and b["filterType"] in (5, 13):
                lo = max(lo or 0, b["freq"])
            if not b["bypass"] and b["filterType"] in (4, 12):
                hi = min(hi or 99999, b["freq"])
        a = score(lo, *within(*e["low_hz"], 0.25))
        b = score(hi, *within(*e["high_hz"], 0.25))
        s = "pass" if a == b == "pass" else "near" if "pass" in (a, b) or "near" in (a, b) else "fail"
        return s, f"{hzt('high-pass', lo)}, {hzt('low-pass', hi)}"
    if k in ("only_changed", "restored"):
        ref = before if k == "only_changed" else steps[e["to_step"]]
        changed = [d for d in now.devices if settings(now.devices[d]["state"]) != settings(ref.devices[d]["state"])]
        bad = [d for d in changed if k == "restored" or d not in e["devices"]]
        return "fail" if bad else "pass", f"changed: {', '.join(changed) or 'nothing'}"
    if k == "says":
        return ("pass" if any(x.lower() in words.lower() for x in e["any"]) else "fail"), "in the answer"
    got = more(k, e, now, before, steps, words)  # -- WP FUNC2a --
    if got:
        return got
    return "fail", f"no such check: {k}"


# -- WP FUNC2a -- the kinds S4, S9 and S11 need (FSTORY 3)
HIGHPASS_TYPES = {5, 13} | set(range(33, 40, 2)) | set(range(41, 56, 2)) | set(range(57, 64, 2))


def wire(state, n):
    """Output n's (from 1) wire channel."""
    return state["numInputChannels"] + n - 1


def atoms(state, device_out_count=9):
    """What a person set, item by item: a band, a gain, a route ..."""
    out = {}
    for ch, bands in enumerate(state["filters"]):
        for b, x in enumerate(bands):
            out[("peq", ch, b)] = json.dumps(x, sort_keys=True)
    for ch, bands in enumerate(state["xover"]):
        for b, x in enumerate(bands):
            out[("xover", ch, b)] = json.dumps(x, sort_keys=True)
    for key in ("outputGainDb", "outputDelayMs", "outputMuted", "outputEnabled", "inputPreampDb", "channelDelays"):
        for i, v in enumerate(state.get(key) or []):
            out[(key, i)] = v
    for key in ("matrixRouting", "matrixGain", "matrixInvert"):
        for i, row in enumerate(state[key]):
            for o, v in enumerate(row):
                out[(key, i, o)] = v
    for key in ("userVolumeDb", "masterVolumeDb", "preampDb", "bypass"):
        out[(key,)] = state.get(key)
    return out


def sheets(run, step):
    """The app's questions while `step` ran: (text, answer)."""
    log = run / "user.jsonl"
    lo, hi = (time.strftime("%H:%M:%S", time.localtime(step[k])) for k in ("start", "end"))
    out = []
    for line in log.read_text().splitlines() if log.exists() else []:
        x = json.loads(line)
        if x["kind"] == "question" and lo <= x["time"] <= hi:
            out.append((x["data"].get("text") or json.dumps(x["data"]), x["data"].get("answer", "allow")))
    return out


def more(k, e, now, before, steps, words):
    if k == "edge_moved":
        rows = []
        for d, outs in e["outputs"].items():
            for n in outs:
                a, b = now.edge(now.out(d, n), e["side"]), before.edge(before.out(d, n), e["side"])
                r = a / b if a and b else None
                rows.append((f"{d} {n}", r, score(r, lambda v: v is not None and e["pass_ratio"][0] <= v <= e["pass_ratio"][1],
                                                  lambda v: v is not None and e["near_ratio"][0] <= v <= e["near_ratio"][1])))
        return every(rows, lambda n, v: f"{n} ×{v:.2f}" if v else f"{n} none")
    if k == "no_band_below":
        bad = []
        for d, outs in e["outputs"].items():
            s = now.devices[d]["state"]
            for n in outs:
                feeds = [i for i in range(s["numInputChannels"]) if s["matrixRouting"][i][n - 1]]
                for bank, ch in [("xover", wire(s, n)), ("filters", wire(s, n))] + [("filters", i) for i in feeds]:
                    for b in s[bank][ch]:
                        if not b["bypass"] and b["filterType"] in HIGHPASS_TYPES and b["freq"] < e["hz"]:
                            bad.append(f"{d} {n}: type {b['filterType']} at {b['freq']:.0f} Hz ({bank} {ch})")
        return ("fail" if bad else "pass"), "; ".join(bad) or "none"
    if k == "gain_change":
        rows = []
        for n in e["outputs"]:
            v = now.devices[e["device"]]["state"]["outputGainDb"][n - 1] - before.devices[e["device"]]["state"]["outputGainDb"][n - 1]
            rows.append((f"{e['device']} {n}", v, score(v, lambda v: e["pass_db"][0] <= v <= e["pass_db"][1],
                                                         lambda v: e["near_db"][0] <= v <= e["near_db"][1])))
        return every(rows, lambda n, v: f"{n} {v:+.1f} dB")
    if k == "band":
        s = now.devices[e["device"]]["state"]
        b = s["filters"][wire(s, e["output"])][e["band"] - 1]
        v = b["gain"] if b["filterType"] else 0.0
        return (score(v, lambda v: e["pass_db"][0] <= v <= e["pass_db"][1],
                      lambda v: "near_db" in e and e["near_db"][0] <= v <= e["near_db"][1]),
                f"band {e['band']}: type {b['filterType']}, {b['freq']:.0f} Hz, {v:+.1f} dB")
    if k == "name":
        s = now.devices[e["device"]]["state"]
        v = s["channelNames"][wire(s, e["output"])]
        return ("pass" if v == e["is"] else "fail"), repr(v)
    if k == "delay":
        v = now.devices[e["device"]]["state"]["outputDelayMs"][e["output"] - 1]
        return ("pass" if v <= e["max_ms"] + 0.01 else "fail"), f"{v:.2f} ms"
    if k == "names_unchanged":
        bad = [d for d in e["devices"] if any(now.devices[d]["state"].get(x) != before.devices[d]["state"].get(x)
                                              for x in ("channelNames", "presetNames"))]
        return ("fail" if bad else "pass"), f"renamed on: {', '.join(bad) or 'none'}"
    if k == "undone":
        base, done = atoms(steps[e["base"]].devices[e["device"]]["state"]), atoms(steps[e["done"]].devices[e["device"]]["state"])
        nowa = atoms(now.devices[e["device"]]["state"])
        made = [a for a in done if done[a] != base[a]]
        back = [a for a in made if nowa[a] == base[a]]
        kept = [a for a in made if nowa[a] == done[a]]
        want = e.get("want", "all")
        if want == "none":
            ok, near = not back and len(kept) == len(made), False
        else:
            ok = (len(back) == 1 and len(kept) == len(made) - 1) if want == "one" else len(back) == len(made)
            near = bool(back)
        return ("pass" if ok else "near" if near else "fail",
                f"{len(made)} made, {len(back)} undone, {len(kept)} kept: undone {[a[:3] for a in back]}")
    if k == "agent_writes":
        old = {x["key"] for x in before.activity}
        n = [x for x in now.activity if x["key"] not in old and x.get("write") and x.get("outcome") == "ok"
             and x.get("device") in e["devices"]]
        return ("pass" if len(n) <= e.get("max", 0) else "fail"), \
            f"{len(n)} on {', '.join(e['devices'])}" + (f": {[x['title'] for x in n][:6]}" if n else "")
    return None


def settings(state):
    """What a person set: no names, presets or meters."""
    keep = ("filters", "xover", "matrixRouting", "matrixGain", "matrixInvert", "outputEnabled", "outputMuted", "outputGainDb",
            "outputDelayMs", "channelDelays", "limiterEnabled", "limiterThresholdDb", "preampDb", "inputPreampDb", "bypass",
            "userVolumeDb", "masterVolumeDb", "loudnessEnabled", "levellerEnabled", "psybassEnabled")
    return json.dumps([state.get(k) for k in keep])


def invariants(run, meta, scenario, calls):
    """Layer (a): what must never happen, from the sim's and the app's own logs."""
    bad = []
    log = run / "sim/oracle.jsonl"
    oracle = [json.loads(x) for x in log.read_text().splitlines()] if log.exists() else []
    for x in oracle:
        if x["kind"] == "VIOLATION":
            bad.append(f"oracle: {x['device']} {x.get('driver')} ({x['how']}): {'; '.join(x['why'])}")
        if x["kind"] == "info" and "no driver" in x.get("what", ""):
            bad.append(f"a test signal on {x['device']} output {x['output'] + 1}, which drives nothing")
        if x["kind"] == "signal" and x["device"] in meta["unconfirmed"]:
            bad.append(f"a test signal on {x['device']}, whose speaker setup is not confirmed")
    # -- WP FUNC2a -- P5a: the settings after every journal entry (functest_oracle.rs)
    log = run / "sim/config-oracle.jsonl"
    for x in (json.loads(y) for y in log.read_text().splitlines()) if log.exists() else []:
        if x["kind"] == "VIOLATION":
            who = "the user" if x.get("actor") == 0 else f"actor {x.get('actor')}"
            bad.append(f"config: {x['device']} {x.get('driver')} after {x.get('path')} by {who}: {'; '.join(x['why'])}")
    # Nothing on an output the user wired nothing to
    states = sorted((run / "state").iterdir(), key=lambda p: p.stat().st_mtime)
    if states:
        last = State(states[-1])
        for d in scenario["devices"]:
            s = last.devices[d["name"]]["state"]
            wired = {o["output"] for o in d["outputs"]}
            for o in range(s["numOutputChannels"]):
                if o not in wired and (s["outputEnabled"][o] or any(r[o] for r in s["matrixRouting"][:s["numInputChannels"]])):
                    bad.append(f"{d['name']} output {o + 1} drives nothing, and was switched on or routed to")
    # Each test signal, and each new or louder route, was asked about when the user asked to be
    if meta["confirm"] != "never_ask":
        log = run / "user.jsonl"
        asked = [json.loads(x) for x in log.read_text().splitlines()] if log.exists() else []
        name = {d["id"]: d.get("title") or d.get("label") for d in meta["devices"]}
        for dev_id, dev in name.items():
            lines = [x for q in asked if q["kind"] == "question" and q["data"]["device"] == dev for x in q["data"]["riskLines"]]
            n_asked = sum(1 for x in lines if x.startswith("Test signal"))
            n_ran = sum(1 for t, a, ok in calls if t in STARTS and ok and a.get("device", meta["devices"][0]["id"]) == dev_id)
            if n_ran > n_asked:
                bad.append(f"{dev}: {n_ran} test signal runs started, {n_asked} asked about")
            routed = {(i, o) for st in states for d in [State(st).devices[dev]] for i, r in
                      enumerate(d["state"]["matrixRouting"]) for o, on in enumerate(r) if on}
            # -- WP FUNC2a -- the routes the user had made before the first prompt were not the agent's
            if (run / "state/0-start/truth.json").exists():
                start = State(run / "state/0-start").devices[dev]["state"]["matrixRouting"]
                routed = {(i, o) for i, o in routed if not start[i][o]}
            text = " ".join(lines)
            for i, o in sorted(routed):
                if f"In {i + 1} to Out {o + 1}" not in text:
                    bad.append(f"{dev}: the route In {i + 1} to Out {o + 1} was added with no question")
    return sorted(set(bad), key=bad.index)


def tool_calls(run):
    """Every MCP call of the run's transcripts: (tool, args, answered without an error)."""
    out = []
    for path in sorted(run.glob("transcript-*.jsonl"), key=lambda p: int(p.name.split("-")[1])):
        pending = {}
        for line in path.read_text().splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("type") not in ("assistant", "user"):
                continue
            content = (m.get("message") or {}).get("content") or []
            for c in content if isinstance(content, list) else []:
                if c.get("type") == "tool_use" and c["name"].startswith("mcp__dspi__"):
                    pending[c["id"]] = (c["name"].removeprefix("mcp__dspi__"), c["input"])
                elif c.get("type") == "tool_result" and c["tool_use_id"] in pending:
                    out.append((*pending[c["tool_use_id"]], not c.get("is_error")))
    return out


def judge_run(run):
    run = pathlib.Path(run)
    meta = json.loads((run / "run.json").read_text())
    scenario = json.loads((run / "scenario.json").read_text())
    story = tomllib.loads((run / "story.toml").read_text())
    expects = {s["label"]: s.get("expect", []) for s in story.get("step", [])}
    lines, totals = [], {"pass": 0, "near": 0, "fail": 0}
    bad = invariants(run, meta, scenario, tool_calls(run))
    lines.append("(a) invariants: " + ("held" if not bad else f"{len(bad)} BROKEN"))
    lines += [f"    ! {b}" for b in bad]
    before, steps = None, {}
    start = run / "state/0-start"
    if (start / "truth.json").exists():
        before = steps["0-start"] = State(start)
    result = []
    for s in meta["steps"]:
        st = run / "state" / f"{s['n']}-{s['label']}"
        if not (st / "truth.json").exists():
            continue
        now = State(st)
        lines.append(f"(b) {s['n']} {s['label']}: " + ("" if s["code"] == 0 else f"the step ended with {s['code']}"))
        for e in expects.get(s["label"], []):
            # -- WP FUNC2a -- a check for another setting, or of words a replay has none of
            if (e.get("when") and e["when"] != meta["confirm"]) or (e["kind"] == "says" and "replay" in s):
                continue
            try:
                if e["kind"] == "asked":
                    asked = [x for x in sheets(run, s) if re.search(e["match"], x[0], re.I)]
                    n = len(asked)
                    sc = "pass" if e.get("min", 0) <= n <= e.get("max", 1 << 30) else "fail"
                    detail = f"{n} asked: " + "; ".join(f"{a}: {x[:90]}" for x, a in asked)[:400]
                else:
                    sc, detail = check(e, now, before, steps, s.get("answer") or "")
            except (KeyError, StopIteration, TypeError, AttributeError) as x:
                sc, detail = "fail", f"not checked: {x!r}"
            shown = f"{sc}{' (optional)' if e.get('optional') else ''}"
            lines.append(f"    {shown:<5} {e['what']}: {detail}")
            if not e.get("optional"):
                totals[sc] += 1
            result.append({"step": s["label"], "what": e["what"], "score": sc, "detail": detail, "optional": e.get("optional", False)})
        for r in s.get("replay", []) + s.get("probe", []):
            if r["wrong"]:
                totals["fail"] += 1
                lines.append(f"    fail  replay: {r['tool']} {json.dumps(r['args'])[:160]}: wanted {r['want']}, got {r['got']}"
                             + (f" ({r['note']})" if r.get("note") else ""))
                result.append({"step": s["label"], "what": f"replay {r['tool']}", "score": "fail", "detail": r["got"]})
        before = steps[s["label"]] = now
    lines.append(f"score: {totals['pass']} pass, {totals['near']} near, {totals['fail']} fail")
    ok = not bad and not totals["fail"] and all(s["code"] == 0 for s in meta["steps"])
    (run / "checks.txt").write_text("\n".join(lines) + "\n")
    (run / "checks.json").write_text(json.dumps({"ok": ok, "invariants": bad, "totals": totals, "checks": result}, indent=1))
    return ok, "\n".join(lines)


if __name__ == "__main__":
    good, text = judge_run(sys.argv[1])
    print(text)
    sys.exit(0 if good else 1)
