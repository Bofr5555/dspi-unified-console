"""WP SCOPE1: command line of the alink scope tool.

  info                          open the scope, print its unit info, close
  capture [options]             capture marker edges, analyse, write CSV + summary
  analyse EDGES.csv [options]   analyse a saved edges.csv again (no scope)

See tools/alink-scope/README.md.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import signal
import sys
import time

import numpy as np

from . import analysis

LABEL_HELP = "name a channel's unit, e.g. --label D0=k340 (repeatable)"


def _limits(a) -> analysis.Limits:
    return analysis.Limits(fs=a.fs, threshold_samples=a.threshold_samples, max_period_ppm=a.max_period_ppm,
                           max_missing_frac=a.max_missing_frac)


def _labels(items) -> dict[str, str]:
    out = {}
    for it in items or []:
        k, _, v = it.partition("=")
        if not v:
            raise SystemExit(f"--label {it!r}: expected CH=NAME")
        out[k.strip().upper()] = v.strip()
    return out


def _period_ns(a) -> float:
    return a.n * 1e9 / a.fs


def _common(p: argparse.ArgumentParser) -> None:
    p.add_argument("--channels", default="D0,D1", help="channels to use, comma separated (D0..D15, or A,B analog)")
    p.add_argument("--ref", help="reference channel (default: the first)")
    p.add_argument("--n", type=int, default=4096, help="marker period in samples (firmware ALINK_SCOPE_MARKER_N)")
    p.add_argument("--fs", type=int, default=48000, help="the units' sample rate (marker period; threshold unit)")
    p.add_argument("--threshold-samples", type=float, default=1.0, help="pass: every offset within +-this many samples")
    p.add_argument("--max-period-ppm", type=float, default=10.0, help="edge period mismatch that means 'not locked'")
    p.add_argument("--max-missing-frac", type=float, default=0.1, help="fail when more edges than this are missing")
    p.add_argument("--label", action="append", help=LABEL_HELP)
    p.add_argument("--out", default=None, help="output directory (default ./alink-scope-<time>)")


def _channels(a) -> tuple[list[str], str]:
    chans = [c.strip().upper() for c in a.channels.split(",") if c.strip()]
    if len(chans) < 2:
        raise SystemExit("need at least two channels")
    ref = (a.ref or chans[0]).upper()
    if ref not in chans:
        raise SystemExit(f"--ref {ref} is not in --channels")
    return chans, ref


class Writer:
    """edges.csv, pairs.csv, summary.txt and run.log in one directory."""

    PAIR_FIELDS = ["capture", "host_time"] + list(analysis.PairResult("", "", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, "").row())

    def __init__(self, out: str):
        os.makedirs(out, exist_ok=True)
        self.out = out
        self.edges = open(os.path.join(out, "edges.csv"), "a", newline="")
        self.pairs = open(os.path.join(out, "pairs.csv"), "a", newline="")
        self.ew = csv.writer(self.edges)
        self.pw = csv.DictWriter(self.pairs, fieldnames=self.PAIR_FIELDS)
        if self.edges.tell() == 0:
            self.ew.writerow(["capture", "host_time", "span_start_ns", "span_end_ns", "match_ns", "channel", "t_ns"])
        if self.pairs.tell() == 0:
            self.pw.writeheader()
        self.log = open(os.path.join(out, "run.log"), "a")

    def segment(self, cap: int, seg: analysis.Segment) -> None:
        m = "" if seg.match_ns is None else f"{seg.match_ns:.3f}"
        wrote = False
        for ch, ts in seg.edges.items():
            for t in ts:
                self.ew.writerow([cap, f"{seg.host_time_s:.6f}", f"{seg.span_ns[0]:.3f}", f"{seg.span_ns[1]:.3f}", m, ch,
                                  f"{t:.3f}"])
                wrote = True
        if not wrote:   # keep the capture visible (all edges missing)
            self.ew.writerow([cap, f"{seg.host_time_s:.6f}", f"{seg.span_ns[0]:.3f}", f"{seg.span_ns[1]:.3f}", m, "", ""])
        self.edges.flush()

    def results(self, cap, host_time: float, results: list[analysis.PairResult]) -> None:
        for r in results:
            row = r.row()
            row.update(capture=cap, host_time=f"{host_time:.3f}")
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


def read_edges(path: str) -> list[tuple[int, analysis.Segment]]:
    segs: dict[int, dict] = {}
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            cap = int(row["capture"])
            s = segs.setdefault(cap, {"host": float(row["host_time"]),
                                      "span": (float(row["span_start_ns"]), float(row["span_end_ns"])),
                                      "match": float(row["match_ns"]) if row.get("match_ns") else None,
                                      "edges": {}})
            if row["channel"]:
                s["edges"].setdefault(row["channel"], []).append(float(row["t_ns"]))
    out = []
    for cap in sorted(segs):
        s = segs[cap]
        e = {k: np.asarray(v) for k, v in s["edges"].items()}
        out.append((cap, analysis.Segment(e, s["span"], s["host"], s["match"])))
    return out


def final_report(segs: list[tuple[int, analysis.Segment]], chans, ref, a, labels, per_capture) -> str:
    """Pooled result over every capture, plus the long-run drift of the capture means."""
    lim = _limits(a)
    period = _period_ns(a)
    pooled = analysis.analyse([s for _, s in segs], chans, ref, period, lim)
    title = (f"alink scope: {len(segs)} capture(s), ref {labels.get(ref, ref)}, N {a.n} at {a.fs} Hz "
             f"(period {period / 1e6:.3f} ms)")
    text = analysis.text_summary(pooled, labels, title)
    if len(per_capture) >= 2:
        lines = ["long run (capture means):"]
        for ch in chans:
            if ch == ref:
                continue
            t = [h for h, res in per_capture for r in res if r.ch == ch]
            m = [r.mean_ns for h, res in per_capture for r in res if r.ch == ch]
            fails = sum(1 for h, res in per_capture for r in res if r.ch == ch and r.verdict != "PASS")
            ok = [x for x in m if not np.isnan(x)]
            span = (max(ok) - min(ok)) if ok else float("nan")
            lines.append(f"  {labels.get(ch, ch)}: drift {analysis.long_run_drift(t, m):+.1f} ns/hour, "
                         f"mean range {span:.0f} ns, failed captures {fails}/{len(m)}")
        text += "\n" + "\n".join(lines)
    failed = sum(1 for _, res in per_capture if any(r.verdict != "PASS" for r in res))
    if failed:
        text = text.replace("OVERALL PASS", "OVERALL FAIL")
        text += f"\n{failed} of {len(per_capture)} capture(s) failed: OVERALL FAIL"
    return text


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def cmd_analyse(a) -> int:
    chans, ref = _channels(a)
    labels = _labels(a.label)
    segs = read_edges(a.edges)
    out = a.out or os.path.join(os.path.dirname(os.path.abspath(a.edges)), "reanalysis")
    os.makedirs(out, exist_ok=True)
    lim = _limits(a)
    per = []
    with open(os.path.join(out, "pairs.csv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=Writer.PAIR_FIELDS)
        w.writeheader()
        for cap, s in segs:
            res = analysis.analyse([s], chans, ref, _period_ns(a), lim)
            per.append((s.host_time_s, res))
            for r in res:
                row = r.row()
                row.update(capture=cap, host_time=f"{s.host_time_s:.3f}")
                w.writerow(row)
    text = final_report(segs, chans, ref, a, labels, per)
    with open(os.path.join(out, "summary.txt"), "w") as f:
        f.write(text + "\n")
    print(text)
    return 0 if "OVERALL PASS" in text else 1


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
    chans, ref = _channels(a)
    labels = _labels(a.label)
    out = a.out or f"alink-scope-{time.strftime('%Y%m%d-%H%M%S')}"
    w = Writer(out)
    capture, sc = _open_scope()
    stop = {"now": False}
    signal.signal(signal.SIGINT, lambda *_: stop.update(now=True))
    signal.signal(signal.SIGTERM, lambda *_: stop.update(now=True))
    per, segs = [], []
    try:
        info = sc.info()
        w.note("unit " + json.dumps(info))
        analog = a.analog
        if not analog and not sc.is_mso(info):
            w.note(f"variant {info.get('variant', '?')!r} has no digital inputs: falling back to analog A, B")
            analog = True
        if analog:
            chans = [c for c in chans if c in ("A", "B")] or ["A", "B"]
            ref = ref if ref in chans else chans[0]
        try:
            sc.setup(chans, a.level, analog)
        except capture.ScopeError as e:
            if analog:
                raise
            w.note(f"digital setup refused ({e}): falling back to analog A, B")
            analog, chans, ref = True, ["A", "B"], "A"
            sc.setup(chans, a.level, analog)
        w.note(f"channels {chans}, ref {ref}, mode {a.mode}, {'analog' if analog else 'digital'}, "
               f"logic threshold {a.level} V")
        period = _period_ns(a)
        lim = _limits(a)
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
                        bl.append(sc.capture_window(chans, ref, a.window_us * 1000.0, analog, a.level))
                    except capture.ScopeError as e:
                        w.note(f"capture {cap}: window: {e}")
                cap_segs = [b.segment for b in bl]
                dt = bl[0].dt_ns if bl else float("nan")
            else:
                b = sc.capture_span(chans, a.periods * period, a.max_samples, analog, a.level)
                cap_segs = [b.segment]
                dt = b.dt_ns
            for s in cap_segs:
                w.segment(cap, s)
                segs.append((cap, s))
            res = analysis.analyse(cap_segs, chans, ref, period, lim)
            w.results(cap, t_cap, res)
            per.append((t_cap, res))
            w.note(f"capture {cap}: dt {dt:.1f} ns; " + "; ".join(
                f"{labels.get(r.ch, r.ch)} {r.verdict} {r.mean_ns:+.0f} ns p2p {r.p2p_ns:.0f} n {r.n}" for r in res))
            cap += 1
            if not a.every or time.time() + a.every > t_end:
                break
            while not stop["now"] and time.time() < t_cap + a.every:
                time.sleep(0.2)
    finally:
        sc.close()
    text = final_report(segs, chans, ref, a, labels, per)
    w.summary(text)
    return 0 if "OVERALL PASS" in text else 1


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="alink-scope", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("info", help="print the scope's unit info")
    c = sub.add_parser("capture", help="capture and analyse")
    _common(c)
    c.add_argument("--mode", choices=("span", "window"), default="span",
                   help="span: one block over --periods marker periods; window: triggered windows around ref edges")
    c.add_argument("--periods", type=float, default=4.0, help="span mode: marker periods per block")
    c.add_argument("--max-samples", type=int, default=50_000_000, help="span mode: samples per block at most")
    c.add_argument("--windows", type=int, default=20, help="window mode: triggered windows per capture")
    c.add_argument("--window-us", type=float, default=200.0, help="window mode: window length (ref edge in the middle)")
    c.add_argument("--level", type=float, default=1.5, help="logic threshold in volts (1.5 for 3.3 V CMOS)")
    c.add_argument("--analog", action="store_true", help="use analog A and B instead of the digital inputs")
    c.add_argument("--every", type=float, default=0.0, help="long run: repeat every K seconds (0: once)")
    c.add_argument("--duration", type=float, default=3600.0, help="long run: total seconds")
    an = sub.add_parser("analyse", help="analyse a saved edges.csv")
    an.add_argument("edges")
    _common(an)
    a = p.parse_args(argv)
    if a.cmd == "info":
        return cmd_info(a)
    if a.cmd == "capture":
        return cmd_capture(a)
    return cmd_analyse(a)


if __name__ == "__main__":
    sys.exit(main())
