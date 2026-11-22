#!/usr/bin/env python3
"""Judge a saved state of the function test from the simulator's own responses:

    judge.py STATE_DIR SCENARIO.json [--svg]

Reads truth.json (computed from the devices' images, not measured). Prints each
output's filters, gain, delay and arrival, each input channel's response at the
seat (band levels, flatness), and what is wrong. Writes response.svg beside it.
"""
import json
import math
import pathlib
import sys

NAMES = ["L", "R", "C", "LFE", "Ls", "Rs"]
TYPES = {0: "off", 1: "peak", 2: "low shelf", 3: "high shelf", 4: "low-pass", 5: "high-pass", 6: "notch",
         7: "all-pass", 11: "Linkwitz"}


def band(freqs, dbs, lo, hi):
    """The mean power level of a band, dB."""
    v = [10 ** (d / 10) for f, d in zip(freqs, dbs) if lo <= f <= hi and d > -190]
    return 10 * math.log10(sum(v) / len(v)) if v else float("-inf")


def smooth(freqs, dbs, octave=1 / 3):
    out = []
    for f in freqs:
        out.append(band(freqs, dbs, f * 2 ** (-octave / 2), f * 2 ** (octave / 2)))
    return out


def corner(freqs, dbs, ref, side):
    """Where a response is 6 dB under `ref`, seen from its passband."""
    idx = range(len(freqs)) if side == "low" else range(len(freqs) - 1, -1, -1)
    for i in idx:
        if dbs[i] >= ref - 6:
            return freqs[i]
    return None


def xo_text(state, o):
    n_in = state["numInputChannels"]
    out = []
    for kind, bank in (("xo", state["xover"]), ("peq", state["filters"])):
        for b in bank[n_in + o]:
            if b["filterType"] == 0 or b["bypass"]:
                continue
            t = b["filterType"]
            name = TYPES.get(t, f"type {t}")
            out.append(f"{kind}:{name} {b['freq']:.0f} Hz"
                       + (f" {b['gain']:+.1f} dB Q{b['q']:.2f}" if kind == "peq" else ""))
    return "; ".join(out) or "none"


def main():
    d = pathlib.Path(sys.argv[1])
    t = json.loads((d / "truth.json").read_text())
    f = t["freqsHz"]
    problems = []
    arrivals = {}
    for dev in t["devices"]:
        s = dev["state"]
        print(f"\n== {dev['name']}: user volume {dev['userVolumeDb']:.1f} dB, master {dev['masterVolumeDb']:.1f} dB")
        for o in dev["outputs"]:
            n = o["output"]
            e = o["electricDb"]
            top = max(e)
            lo, hi = corner(f, e, top, "low"), corner(f, e, top, "high")
            routes = ",".join(NAMES[i] if i < 6 else f"in{i + 1}" for i in o["inputs"]) or "-"
            inv = " inverted" if o["inverted"] else ""
            print(f"  out {n + 1} {o['name']:<20} {o['role']:<5} in {routes:<10}{inv} gain {o['gainDb']:+.1f} "
                  f"delay {o['delayMs']:.2f} arrives {o['arrivalMs']:.2f} ms  electric {lo:.0f}-{hi:.0f} Hz top {top:+.1f}")
            print(f"        {xo_text(s, n)}")
            arrivals.setdefault((dev["name"], o["speaker"]), []).append((o["name"], o["arrivalMs"]))
            at = lambda hz: e[min(range(len(f)), key=lambda i: abs(f[i] - hz))]
            if o["role"] == "high":
                if at(500) - top > -12:
                    problems.append(f"{o['name']}: only {at(500) - top:.1f} dB at 500 Hz electrically")
                if not o["inputs"]:
                    problems.append(f"{o['name']}: nothing routed")
            elif not o["inputs"]:
                problems.append(f"{o['name']}: nothing routed")
            if not o["enabled"]:
                problems.append(f"{o['name']}: off or muted")
    print("\n== at the seat, by input channel (1/3-octave; levels are band means, dB)")
    levels = {}
    for i, dbs in enumerate(t["inputsDb"][:6]):
        if max(dbs) < -150:
            print(f"  {NAMES[i]:<4} silent")
            continue
        sm = smooth(f, dbs)
        bands = [("25-80", 25, 80), ("80-250", 80, 250), ("250-1k", 250, 1000), ("1k-4k", 1000, 4000), ("4k-16k", 4000, 16000)]
        b = {n: band(f, dbs, lo, hi) for n, lo, hi in bands}
        mid = band(f, dbs, 300, 3000)
        levels[NAMES[i]] = mid
        dev = [x - mid for x, hz in zip(sm, f) if 100 <= hz <= 12000]
        rng = max(dev) - min(dev)
        std = math.sqrt(sum(x * x for x in dev) / len(dev) - (sum(dev) / len(dev)) ** 2)
        print(f"  {NAMES[i]:<4} mid {mid:6.1f}  " + "  ".join(f"{k} {v:6.1f}" for k, v in b.items())
              + f"   100-12k: spread {rng:.1f} dB, sd {std:.1f} dB")
    mains = {k: v for k, v in levels.items() if k in ("L", "R", "C", "Ls", "Rs")}
    if len(mains) > 1:
        m = sorted(mains.values())[len(mains) // 2]
        print("  channel levels against the median: " + ", ".join(f"{k} {v - m:+.1f}" for k, v in mains.items()))
        for k, v in mains.items():
            if abs(v - m) > 1.5:
                problems.append(f"{k}: {v - m:+.1f} dB against the other channels")
    print("\n== arrivals at the seat, ms")
    every = []
    for (dev, sp), a in arrivals.items():
        print(f"  {dev}/{sp}: " + ", ".join(f"{n} {x:.2f}" for n, x in a)
              + f"   (within the speaker {max(x for _, x in a) - min(x for _, x in a):.2f})")
        every += [x for _, x in a]
    if every:
        print(f"  across all: {min(every):.2f} to {max(every):.2f} (spread {max(every) - min(every):.2f} ms)")
    print("\n== problems" if problems else "\n== no problem seen")
    for p in problems:
        print("  -", p)
    oracle = d / "oracle.jsonl"
    if oracle.exists():
        lines = [json.loads(x) for x in oracle.read_text().splitlines()]
        n = {}
        for x in lines:
            n[x["kind"]] = n.get(x["kind"], 0) + 1
        print("\n== oracle:", n)
        for x in lines:
            if x["kind"] in ("VIOLATION", "WARN"):
                print("  ", x["kind"], x["device"], x.get("driver"), x.get("how"), "; ".join(x.get("why", [])))
    svg(d, t)


COLOURS = ["#2a6fdb", "#d1495b", "#2e8b57", "#8a6d3b", "#7b5cc4", "#c77d1e"]


def svg(d, t):
    """Each input channel at the seat, and each driver's own sum."""
    w, h, left, top = 900, 420, 50, 24
    f = t["freqsHz"]
    x = lambda hz: left + (math.log10(hz) - math.log10(20)) / 3 * (w - left - 160)
    y = lambda db: top + (10 - db) / 70 * (h - top - 40)
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" font-family="sans-serif" font-size="12">'
             f'<rect width="{w}" height="{h}" fill="#fff"/>'
             f'<text x="{left}" y="16">At the seat, 1/3-octave: {t["scenario"]}, {d.name}</text>']
    for hz in (20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000):
        parts.append(f'<line x1="{x(hz):.0f}" y1="{top}" x2="{x(hz):.0f}" y2="{h - 40}" stroke="#ddd"/>'
                     f'<text x="{x(hz):.0f}" y="{h - 24}" text-anchor="middle">{hz if hz < 1000 else str(hz // 1000) + "k"}</text>')
    for db in range(-60, 11, 10):
        parts.append(f'<line x1="{left}" y1="{y(db):.0f}" x2="{w - 160}" y2="{y(db):.0f}" stroke="#ddd"/>'
                     f'<text x="{left - 6}" y="{y(db) + 4:.0f}" text-anchor="end">{db}</text>')
    for i, dbs in enumerate(t["inputsDb"][:6]):
        if max(dbs) < -150:
            continue
        sm = smooth(f, dbs)
        pts = " ".join(f"{x(a):.1f},{y(max(-60, min(10, b))):.1f}" for a, b in zip(f, sm))
        parts.append(f'<polyline points="{pts}" fill="none" stroke="{COLOURS[i]}" stroke-width="1.6"/>'
                     f'<text x="{w - 150}" y="{top + 20 + 18 * i}" fill="{COLOURS[i]}">{NAMES[i]}</text>')
    parts.append("</svg>")
    (d / "response.svg").write_text("".join(parts))


if __name__ == "__main__":
    main()
