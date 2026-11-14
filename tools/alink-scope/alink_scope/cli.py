"""WP SCOPE1: command line of the alink scope tool.

  info                           open the scope, print its unit info, close
  capture [options]              capture marker edges, analyse, write CSV + summary
  analyse EDGES.csv [options]    analyse a saved edges.csv again (no scope)
  compare RUN_DIR... [--out D]   tabulate offset, jitter and drift across runs (e.g. cable types)

Every run carries a free-text --label (e.g. the cable) and per-channel unit names
(--unit D0=k340); both go into meta.json, the CSV files and the summary.
See tools/alink-scope/README.md.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import os
import signal
import sys
import time
from dataclasses import dataclass

import numpy as np

from . import analysis


# ---------------------------------------------------------------------------
# Run description
# ---------------------------------------------------------------------------

@dataclass
class Run:
    label: str
    units: dict[str, str]           # channel -> unit name
    chans: list[str]
    ref: str
    n: int
    fs: int
    limits: analysis.Limits

    @property
    def period_ns(self) -> float:
        return self.n * 1e9 / self.fs

    def unit(self, ch: str) -> str:
        return self.units.get(ch, ch)

    def meta(self) -> dict:
        return {"label": self.label, "units": self.units, "channels": self.chans, "ref": self.ref, "n": self.n,
                "fs": self.fs, "threshold_samples": self.limits.threshold_samples,
                "max_period_ppm": self.limits.max_period_ppm, "max_missing_frac": self.limits.max_missing_frac}

    @classmethod
    def from_meta(cls, m: dict) -> "Run":
        lim = analysis.Limits(fs=int(m["fs"]), threshold_samples=float(m["threshold_samples"]),
                              max_period_ppm=float(m["max_period_ppm"]), max_missing_frac=float(m["max_missing_frac"]))
        return cls(m.get("label", ""), dict(m.get("units", {})), list(m["channels"]), m["ref"], int(m["n"]),
                   int(m["fs"]), lim)


def _units(items) -> dict[str, str]:
    out = {}
    for it in items or []:
        k, _, v = it.partition("=")
        if not v:
            raise SystemExit(f"--unit {it!r}: expected CH=NAME, e.g. D0=k340")
        out[k.strip().upper()] = v.strip()
    return out


def run_from_args(a) -> Run:
    chans = [c.strip().upper() for c in a.channels.split(",") if c.strip()]
    if len(chans) < 2:
        raise SystemExit("need at least two channels")
    ref = (a.ref or chans[0]).upper()
    if ref not in chans:
        raise SystemExit(f"--ref {ref} is not in --channels")
    lim = analysis.Limits(fs=a.fs, threshold_samples=a.threshold_samples, max_period_ppm=a.max_period_ppm,
                          max_missing_frac=a.max_missing_frac)
    return Run(a.label or "", _units(a.unit), chans, ref, a.n, a.fs, lim)


def _common(p: argparse.ArgumentParser) -> None:
    p.add_argument("--channels", default="D0,D1", help="channels to use, comma separated (D0..D15, or A,B analog)")
    p.add_argument("--ref", help="reference channel (default: the first)")
    p.add_argument("--label", help="free text for this run, e.g. the cable: --label 'active 10 m, brand X'")
    p.add_argument("--unit", action="append", help="name a channel's unit: --unit D0=k340 (repeatable)")
    p.add_argument("--n", type=int, default=4096, help="marker period in samples (firmware ALINK_SCOPE_MARKER_N)")
    p.add_argument("--fs", type=int, default=48000, help="the units' sample rate (marker period; threshold unit)")
    p.add_argument("--threshold-samples", type=float, default=1.0, help="pass: every offset within +-this many samples")
    p.add_argument("--max-period-ppm", type=float, default=10.0, help="edge period mismatch that means 'not locked'")
    p.add_argument("--max-missing-frac", type=float, default=0.1, help="fail when more edges than this are missing")
    p.add_argument("--out", default=None, help="output directory (default ./alink-scope-<time>)")


# ---------------------------------------------------------------------------
# Files
# ---------------------------------------------------------------------------

PAIR_FIELDS = (["capture", "host_time", "run_label", "ref_unit", "unit"]
               + list(analysis.PairResult("", "", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, "").row()))
EDGE_FIELDS = ["capture", "host_time", "span_start_ns", "span_end_ns", "match_ns", "dt_ns", "run_label", "unit",
               "channel", "t_ns"]


def pair_rows(run: Run, cap, host_time: float, results: list[analysis.PairResult]):
    for r in results:
        row = r.row()
        row.update(capture=cap, host_time=f"{host_time:.3f}", run_label=run.label, ref_unit=run.unit(r.ref),
                   unit=run.unit(r.ch))
        yield row


class Writer:
    """meta.json, edges.csv, pairs.csv, summary.txt and run.log in one directory."""

    def __init__(self, out: str, run: Run):
        os.makedirs(out, exist_ok=True)
        self.out, self.run = out, run
        with open(os.path.join(out, "meta.json"), "w") as f:
            json.dump(run.meta(), f, indent=1)
        self.edges = open(os.path.join(out, "edges.csv"), "a", newline="")
        self.pairs = open(os.path.join(out, "pairs.csv"), "a", newline="")
        self.ew = csv.writer(self.edges)
        self.pw = csv.DictWriter(self.pairs, fieldnames=PAIR_FIELDS)
        if self.edges.tell() == 0:
            self.ew.writerow(EDGE_FIELDS)
        if self.pairs.tell() == 0:
            self.pw.writeheader()
        self.log = open(os.path.join(out, "run.log"), "a")

    def segment(self, cap: int, seg: analysis.Segment) -> None:
        head = [cap, f"{seg.host_time_s:.6f}", f"{seg.span_ns[0]:.3f}", f"{seg.span_ns[1]:.3f}",
                "" if seg.match_ns is None else f"{seg.match_ns:.3f}",
                "" if seg.dt_ns is None else f"{seg.dt_ns:.4f}", self.run.label]
        wrote = False
        for ch, ts in seg.edges.items():
            for t in ts:
                self.ew.writerow(head + [self.run.unit(ch), ch, f"{t:.3f}"])
                wrote = True
        if not wrote:   # keep the capture visible (all edges missing)
            self.ew.writerow(head + ["", "", ""])
        self.edges.flush()

    def results(self, cap, host_time: float, results: list[analysis.PairResult]) -> None:
        for row in pair_rows(self.run, cap, host_time, results):
            self.pw.writerow(row)
        self.pairs.flush()

    def note(self, text: str) -> None:
        line = f"{time.strftime('%Y-%m-%d %H:%M:%S')} {text}"
        print(line, flush=True)
        self.log.write(line + "\n")
        self.log.flush()

    def summary(self, text: str) -> None:
        with open(os.path.join(self.out, "summary.txt"), "w") as f:
            f.write(text + "\n")
        print(text, flush=True)

    def close(self) -> None:
        for f in (self.edges, self.pairs, self.log):
            f.close()


def read_edges(path: str) -> list[tuple[int, analysis.Segment]]:
    segs: dict[int, dict] = {}
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            cap = int(row["capture"])
            s = segs.setdefault(cap, {"host": float(row["host_time"]),
                                      "span": (float(row["span_start_ns"]), float(row["span_end_ns"])),
                                      "match": float(row["match_ns"]) if row.get("match_ns") else None,
                                      "dt": float(row["dt_ns"]) if row.get("dt_ns") else None,
                                      "edges": {}})
            if row["channel"]:
                s["edges"].setdefault(row["channel"], []).append(float(row["t_ns"]))
    out = []
    for cap in sorted(segs):
        s = segs[cap]
        e = {k: np.asarray(v) for k, v in s["edges"].items()}
        out.append((cap, analysis.Segment(e, s["span"], s["host"], s["match"], s["dt"])))
    return out


# ---------------------------------------------------------------------------
# Evaluation
# ---------------------------------------------------------------------------

@dataclass
class Evaluation:
    pooled: list[analysis.PairResult]
    per_capture: list[tuple[float, list[analysis.PairResult]]]
    long_drift_ns_h: dict[str, float]       # channel -> ns/hour of the capture means
    failed_captures: dict[str, int]
    captures: int

    @property
    def passed(self) -> bool:
        return bool(self.pooled) and all(r.verdict == "PASS" for r in self.pooled) and \
            not any(self.failed_captures.values())


def evaluate(run: Run, segs: list[tuple[int, analysis.Segment]],
             per_capture: list[tuple[float, list[analysis.PairResult]]] | None = None) -> Evaluation:
    if per_capture is None:
        per_capture = []
        for cap in sorted({c for c, _ in segs}):
            cs = [s for c, s in segs if c == cap]
            per_capture.append((cs[0].host_time_s, analysis.analyse(cs, run.chans, run.ref, run.period_ns, run.limits)))
    pooled = analysis.analyse([s for _, s in segs], run.chans, run.ref, run.period_ns, run.limits)
    drift, failed = {}, {}
    for ch in run.chans:
        if ch == run.ref:
            continue
        t = [h for h, res in per_capture for r in res if r.ch == ch]
        m = [r.mean_ns for h, res in per_capture for r in res if r.ch == ch]
        drift[ch] = analysis.long_run_drift(t, m)
        failed[ch] = sum(1 for h, res in per_capture for r in res if r.ch == ch and r.verdict != "PASS")
    return Evaluation(pooled, per_capture, drift, failed, len(per_capture))


def report(run: Run, ev: Evaluation) -> str:
    title = (f"alink scope: run '{run.label}', {ev.captures} capture(s), ref {run.unit(run.ref)} ({run.ref}), "
             f"N {run.n} at {run.fs} Hz (period {run.period_ns / 1e6:.3f} ms)")
    text = analysis.text_summary(ev.pooled, run.units, title)
    if ev.captures >= 2:
        lines = ["long run (capture means):"]
        for ch in run.chans:
            if ch == run.ref:
                continue
            m = [r.mean_ns for h, res in ev.per_capture for r in res if r.ch == ch and not math.isnan(r.mean_ns)]
            span = (max(m) - min(m)) if m else float("nan")
            lines.append(f"  {run.unit(ch)}: drift {ev.long_drift_ns_h[ch]:+.1f} ns/hour, mean range {span:.0f} ns, "
                         f"failed captures {ev.failed_captures[ch]}/{ev.captures}")
        text += "\n" + "\n".join(lines)
    failed = sum(1 for _, res in ev.per_capture if any(r.verdict != "PASS" for r in res))
    if failed:
        text = text.replace("OVERALL PASS", "OVERALL FAIL")
        text += f"\n{failed} of {ev.captures} capture(s) failed: OVERALL FAIL"
    return text


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def _run_for_edges(a) -> Run:
    """Command-line values; channels, ref, units, label, N and fs default to the run's meta.json."""
    meta_path = os.path.join(os.path.dirname(os.path.abspath(a.edges)), "meta.json")
    if os.path.exists(meta_path):
        with open(meta_path) as f:
            m = json.load(f)
        if a.channels is None:
            a.channels = ",".join(m["channels"])
        a.ref = a.ref or m["ref"]
        a.label = a.label if a.label is not None else m.get("label", "")
        a.unit = a.unit or [f"{k}={v}" for k, v in m.get("units", {}).items()]
        a.n = a.n or int(m["n"])
        a.fs = a.fs or int(m["fs"])
    a.channels = a.channels or "D0,D1"
    a.n = a.n or 4096
    a.fs = a.fs or 48000
    return run_from_args(a)


def cmd_analyse(a) -> int:
    run = _run_for_edges(a)
    segs = read_edges(a.edges)
    out = a.out or os.path.join(os.path.dirname(os.path.abspath(a.edges)), "reanalysis")
    os.makedirs(out, exist_ok=True)
    ev = evaluate(run, segs)
    with open(os.path.join(out, "meta.json"), "w") as f:
        json.dump(run.meta(), f, indent=1)
    with open(os.path.join(out, "pairs.csv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=PAIR_FIELDS)
        w.writeheader()
        for (cap, s), (h, res) in zip(sorted({c: s for c, s in segs}.items()), ev.per_capture):
            for row in pair_rows(run, cap, h, res):
                w.writerow(row)
    text = report(run, ev)
    with open(os.path.join(out, "summary.txt"), "w") as f:
        f.write(text + "\n")
    print(text)
    return 0 if ev.passed else 1


COMPARE_FIELDS = ["run", "run_label", "unit", "channel", "ref_unit", "captures", "n", "missing", "mean_ns",
                  "mean_samples_48k", "mean_samples_96k", "std_ns", "p2p_ns", "drift_ns_per_s", "drift_ns_per_hour",
                  "failed_captures", "verdict"]


def compare_rows(run_dirs: list[str]) -> list[dict]:
    rows = []
    for d in run_dirs:
        mp, ep = os.path.join(d, "meta.json"), os.path.join(d, "edges.csv")
        if not (os.path.exists(mp) and os.path.exists(ep)):
            raise SystemExit(f"{d}: no meta.json / edges.csv (not a capture directory)")
        with open(mp) as f:
            run = Run.from_meta(json.load(f))
        ev = evaluate(run, read_edges(ep))
        for r in ev.pooled:
            rows.append({
                "run": os.path.basename(os.path.normpath(d)), "run_label": run.label, "unit": run.unit(r.ch),
                "channel": r.ch, "ref_unit": run.unit(r.ref), "captures": ev.captures, "n": r.n, "missing": r.missing,
                "mean_ns": r.mean_ns, "mean_samples_48k": analysis.ns_to_samples(r.mean_ns, 48000),
                "mean_samples_96k": analysis.ns_to_samples(r.mean_ns, 96000), "std_ns": r.std_ns, "p2p_ns": r.p2p_ns,
                "drift_ns_per_s": r.drift_ns_per_s, "drift_ns_per_hour": ev.long_drift_ns_h.get(r.ch, float("nan")),
                "failed_captures": ev.failed_captures.get(r.ch, 0),
                "verdict": "PASS" if r.verdict == "PASS" and not ev.failed_captures.get(r.ch) else "FAIL",
            })
    return rows


def compare_table(rows: list[dict]) -> str:
    hdr = ["run label", "unit", "vs", "offset ns", "smp@48k", "std ns", "p2p ns", "drift ns/h", "caps", "miss", "verdict"]
    body = []
    for r in rows:
        body.append([r["run_label"] or r["run"], r["unit"], r["ref_unit"], f"{r['mean_ns']:+.0f}",
                     f"{r['mean_samples_48k']:+.3f}", f"{r['std_ns']:.0f}", f"{r['p2p_ns']:.0f}",
                     "-" if math.isnan(r["drift_ns_per_hour"]) else f"{r['drift_ns_per_hour']:+.1f}",
                     str(r["captures"]), str(r["missing"]), r["verdict"]])
    w = [max(len(h), *(len(b[i]) for b in body)) if body else len(h) for i, h in enumerate(hdr)]
    fmt = lambda cols: "  ".join(c.ljust(w[i]) if i < 3 else c.rjust(w[i]) for i, c in enumerate(cols))  # noqa: E731
    return "\n".join([fmt(hdr), fmt(["-" * x for x in w])] + [fmt(b) for b in body])


def cmd_compare(a) -> int:
    rows = compare_rows(a.runs)
    text = compare_table(rows)
    print(text)
    if a.out:
        os.makedirs(a.out, exist_ok=True)
        with open(os.path.join(a.out, "compare.csv"), "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=COMPARE_FIELDS)
            w.writeheader()
            for r in rows:
                w.writerow({k: (f"{v:.3f}" if isinstance(v, float) else v) for k, v in r.items()})
        with open(os.path.join(a.out, "compare.txt"), "w") as f:
            f.write(text + "\n")
    return 0 if rows and all(r["verdict"] == "PASS" for r in rows) else 1


AUTO_SPAN_MAX_DT_NS = 100.0     # span mode only when the block resolves edges to 100 ns or better


def auto_mode(memory: int, samples: int, span_ns: float, analog: bool) -> str:
    """span when its block resolves the edges finely enough (period check and jitter), else window
    (review scope1 T1: a 48 kS scope over 4 periods gives 7 us per sample).  Window mode needs a
    digital reference to trigger on."""
    if analog or (samples > 0 and span_ns / samples <= AUTO_SPAN_MAX_DT_NS):
        return "span"
    return "window"


def _open_scope():
    from . import capture
    sc = capture.Ps2000a(capture.load_driver())
    sc.open()
    return capture, sc


def cmd_info(a) -> int:
    capture, sc = _open_scope()
    try:
        info = sc.info()
        print(json.dumps(info, indent=1))
        print("MSO (digital inputs D0-D15): " + ("yes" if sc.is_mso(info) else "NO - use --analog (A, B)"))
        print(f"max samples (1 segment, current setup): {sc.max_samples(1)}")
    finally:
        sc.close()
    return 0


def cmd_capture(a) -> int:
    run = run_from_args(a)
    out = a.out or f"alink-scope-{time.strftime('%Y%m%d-%H%M%S')}"
    capture, sc = _open_scope()
    stop = {"now": False}
    signal.signal(signal.SIGINT, lambda *_: stop.update(now=True))
    signal.signal(signal.SIGTERM, lambda *_: stop.update(now=True))
    per, segs = [], []
    w = None
    try:
        info = sc.info()
        analog = a.analog
        note = []
        if not analog and not sc.is_mso(info):
            note.append(f"variant {info.get('variant', '?')!r} has no digital inputs: falling back to analog A, B")
            analog = True
        if analog:
            run.chans = [c for c in run.chans if c in ("A", "B")] or ["A", "B"]
            run.ref = run.ref if run.ref in run.chans else run.chans[0]
        try:
            sc.setup(run.chans, a.level, analog)
        except capture.ScopeError as e:
            if analog:
                raise
            note.append(f"digital setup refused ({e}): falling back to analog A, B")
            analog, run.chans, run.ref = True, ["A", "B"], "A"
            sc.setup(run.chans, a.level, analog)
        w = Writer(out, run)
        w.note("unit " + json.dumps(info))
        for n in note:
            w.note(n)
        if a.mode == "auto":
            a.mode = auto_mode(sc.max_samples(1), min(a.max_samples, sc.max_samples(1)), a.periods * run.period_ns,
                               analog)
            w.note(f"mode auto: {a.mode} (memory {sc.max_samples(1)} samples)")
        w.note(f"run '{run.label}', units {run.units}, channels {run.chans}, ref {run.ref}, mode {a.mode}, "
               f"{'analog' if analog else 'digital'}, logic threshold {a.level} V")
        t_end = time.time() + (a.duration if a.every else 0)
        cap = 0
        while not stop["now"]:
            t_cap = time.time()
            if a.mode == "window":
                bl = []
                for _ in range(a.windows):
                    if stop["now"]:
                        break
                    try:
                        bl.append(sc.capture_window(run.chans, run.ref, a.window_us * 1000.0, analog, a.level))
                    except capture.ScopeError as e:
                        w.note(f"capture {cap}: window: {e}")
                cap_segs = [b.segment for b in bl]
                dt = bl[0].dt_ns if bl else float("nan")
            else:
                b = sc.capture_span(run.chans, a.periods * run.period_ns, a.max_samples, analog, a.level)
                cap_segs = [b.segment]
                dt = b.dt_ns
            for s in cap_segs:
                w.segment(cap, s)
                segs.append((cap, s))
            res = analysis.analyse(cap_segs, run.chans, run.ref, run.period_ns, run.limits)
            w.results(cap, t_cap, res)
            weak = [r.period_tol_ppm for r in res if not np.isnan(r.period_tol_ppm)]
            if cap == 0 and weak and max(weak) > run.limits.max_period_ppm:
                w.note(f"warning: at dt {dt:.0f} ns edge quantisation allows +-{max(weak):.1f} ppm on the marker "
                       f"period (limit {run.limits.max_period_ppm:g}): the not-locked check is widened by that; "
                       "for a tight check use --mode window or more --periods")
            per.append((t_cap, res))
            w.note(f"capture {cap}: dt {dt:.1f} ns; " + "; ".join(
                f"{run.unit(r.ch)} {r.verdict} {r.mean_ns:+.0f} ns p2p {r.p2p_ns:.0f} n {r.n}" for r in res))
            cap += 1
            if not a.every or time.time() + a.every > t_end:
                break
            while not stop["now"] and time.time() < t_cap + a.every:
                time.sleep(0.2)
    finally:
        sc.close()
    ev = evaluate(run, segs, per)
    w.summary(report(run, ev))
    w.close()
    return 0 if ev.passed else 1


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="alink-scope", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("info", help="print the scope's unit info")
    c = sub.add_parser("capture", help="capture and analyse")
    _common(c)
    c.add_argument("--mode", choices=("auto", "span", "window"), default="auto",
                   help="span: one block over --periods marker periods; window: triggered windows around ref edges; "
                        "auto (default): span if that block resolves edges to 100 ns, else window")
    c.add_argument("--periods", type=float, default=4.0, help="span mode: marker periods per block")
    c.add_argument("--max-samples", type=int, default=50_000_000, help="span mode: samples per block at most")
    c.add_argument("--windows", type=int, default=20, help="window mode: triggered windows per capture")
    c.add_argument("--window-us", type=float, default=200.0, help="window mode: window length (ref edge in the middle)")
    c.add_argument("--level", type=float, default=1.5, help="logic threshold in volts (1.5 for 3.3 V CMOS)")
    c.add_argument("--analog", action="store_true", help="use analog A and B instead of the digital inputs")
    c.add_argument("--every", type=float, default=0.0, help="long run: repeat every K seconds (0: once)")
    c.add_argument("--duration", type=float, default=3600.0, help="long run: total seconds")
    an = sub.add_parser("analyse", help="analyse a saved edges.csv (settings default to its meta.json)")
    an.add_argument("edges")
    _common(an)
    for act in an._actions:          # None = take it from meta.json
        if act.dest in ("channels", "n", "fs"):
            act.default = None
    cp = sub.add_parser("compare", help="tabulate offset, jitter and drift across capture directories")
    cp.add_argument("runs", nargs="+", help="capture directories (each with meta.json and edges.csv)")
    cp.add_argument("--out", help="also write compare.csv and compare.txt here")
    a = p.parse_args(argv)
    if a.cmd == "info":
        return cmd_info(a)
    if a.cmd == "capture":
        return cmd_capture(a)
    if a.cmd == "compare":
        return cmd_compare(a)
    return cmd_analyse(a)


if __name__ == "__main__":
    sys.exit(main())
