"""WP SCOPE1: edge extraction and pairwise timing analysis of the DSPi scope marker.

Pure numpy, no device access: the capture side (capture.py) hands over sample
arrays, this module turns them into rising-edge times and compares units.

Model.  Every unit raises its marker pin when output sample M, a multiple of N
on the shared time base, leaves for the output (firmware
Documentation/Bench/scope_marker.md).  Edges of different units for the same M
should coincide; the time from a reference unit's edge to another unit's
nearest edge is that unit's audio offset.  Edges repeat every N / fs (85.33 ms
for N 4096 at 48 kHz), so an offset is only defined modulo the period: a
match farther than `match_frac` of the period is treated as no match.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np

RATES = (48000, 96000)


# ---------------------------------------------------------------------------
# Edges
# ---------------------------------------------------------------------------

def rising_edges_digital(bits, dt_ns: float, t0_ns: float = 0.0) -> np.ndarray:
    """Rising edges of a 0/1 sample array, in ns.

    The edge lies between the last low and the first high sample; it is placed
    half a sample interval before the first high sample (error within +-dt/2).
    A high level at the very first sample is not an edge (its start is unknown).
    """
    b = np.asarray(bits).astype(bool)
    if b.size < 2:
        return np.zeros(0)
    idx = np.flatnonzero(b[1:] & ~b[:-1]) + 1
    return t0_ns + (idx - 0.5) * dt_ns


def rising_edges_analog(v, threshold: float, dt_ns: float, hysteresis: float = 0.0,
                        t0_ns: float = 0.0) -> np.ndarray:
    """Rising threshold crossings of an analog trace, linearly interpolated, in ns.

    With hysteresis h a new rising edge needs the trace to have been below
    threshold - h since the previous one (noise on a slow edge counts once).
    """
    x = np.asarray(v, dtype=float)
    if x.size < 2:
        return np.zeros(0)
    lo = threshold - hysteresis
    out = []
    armed = x[0] < lo
    for i in range(1, x.size):
        if not armed:
            if x[i] < lo:
                armed = True
            continue
        if x[i - 1] < threshold <= x[i]:
            frac = (threshold - x[i - 1]) / (x[i] - x[i - 1])
            out.append(t0_ns + (i - 1 + frac) * dt_ns)
            armed = False
    return np.asarray(out)


def bits_of_port(port_values, bit: int) -> np.ndarray:
    """One digital channel (bit 0..7) of a ps2000a digital port buffer (int16 per sample)."""
    return (np.asarray(port_values).astype(np.int32) >> bit) & 1


# ---------------------------------------------------------------------------
# Pairing
# ---------------------------------------------------------------------------

@dataclass
class Match:
    t_ref_ns: np.ndarray            # reference edge times that found a partner
    offset_ns: np.ndarray           # partner - reference
    missing: int                    # reference edges (inside the capture) with no partner
    extra: int                      # partner edges no reference edge claimed (inside the capture)


def pair_edges(ref: np.ndarray, other: np.ndarray, period_ns: float, span_ns: tuple[float, float] | None = None,
               match_frac: float = 0.45, match_ns: float | None = None) -> Match:
    """Match each reference edge to the nearest edge of `other` within match_frac * period.

    Each edge of `other` is used once (the closest reference edge wins).  Edges
    within the match window of the capture's ends are not counted as missing or
    extra: their partner may lie outside the capture (wraparound at the ends).
    """
    ref = np.sort(np.asarray(ref, dtype=float))
    oth = np.sort(np.asarray(other, dtype=float))
    win = match_ns if match_ns else match_frac * period_ns
    if span_ns is None:
        allv = np.concatenate([ref, oth]) if ref.size + oth.size else np.zeros(1)
        span_ns = (float(allv.min()), float(allv.max()))
    lo, hi = span_ns[0] + win, span_ns[1] - win

    cand = []                       # (|d|, i_ref, j_oth)
    if oth.size:
        for i, t in enumerate(ref):
            j = int(np.searchsorted(oth, t))
            for k in (j - 1, j):
                if 0 <= k < oth.size and abs(oth[k] - t) <= win:
                    cand.append((abs(oth[k] - t), i, k))
    cand.sort()
    used_r, used_o = set(), set()
    pairs = []
    for _, i, k in cand:
        if i in used_r or k in used_o:
            continue
        used_r.add(i)
        used_o.add(k)
        pairs.append((ref[i], oth[k] - ref[i]))
    pairs.sort()
    missing = sum(1 for i, t in enumerate(ref) if i not in used_r and lo <= t <= hi)
    extra = sum(1 for k, t in enumerate(oth) if k not in used_o and lo <= t <= hi)
    t = np.array([p[0] for p in pairs])
    d = np.array([p[1] for p in pairs])
    return Match(t, d, missing, extra)


def edge_period(edges: np.ndarray, nominal_ns: float) -> float | None:
    """Period from a least-squares fit of edge time against edge number (missing edges allowed)."""
    e = np.sort(np.asarray(edges, dtype=float))
    if e.size < 3:
        return None
    n = np.round((e - e[0]) / nominal_ns)
    if np.unique(n).size < 3:
        return None
    slope, _ = np.polyfit(n, e, 1)
    return float(slope)


def period_bound_ns(edges: np.ndarray, nominal_ns: float, dt_ns: float) -> float:
    """Worst-case error of edge_period() from edge quantisation (each edge within +-dt/2).

    The least-squares slope is sum((k - kbar) * e_k) / sum((k - kbar)^2), so an error
    of at most dt/2 per edge moves it by at most dt/2 * sum|k - kbar| / sum((k - kbar)^2).
    """
    e = np.sort(np.asarray(edges, dtype=float))
    if e.size < 3:
        return float("inf")
    k = np.round((e - e[0]) / nominal_ns)
    c = k - k.mean()
    den = float((c * c).sum())
    return float("inf") if den == 0 else 0.5 * dt_ns * float(np.abs(c).sum()) / den


# ---------------------------------------------------------------------------
# Results
# ---------------------------------------------------------------------------

def ns_to_samples(ns: float, fs: int) -> float:
    return ns * fs / 1e9


@dataclass
class PairResult:
    ref: str
    ch: str
    n: int
    missing: int
    extra: int
    mean_ns: float
    std_ns: float
    p2p_ns: float
    min_ns: float
    max_ns: float
    drift_ns_per_s: float           # slope of offset against time inside the data set
    period_ppm: float               # channel's edge period against the reference's (nan: unknown)
    threshold_ns: float
    verdict: str                    # PASS / FAIL
    reasons: list[str] = field(default_factory=list)
    period_tol_ppm: float = float("nan")   # quantisation bound of period_ppm (nan: sample interval unknown)

    def samples(self, fs: int) -> float:
        return ns_to_samples(self.mean_ns, fs)

    def row(self) -> dict:
        d = {
            "ref": self.ref, "ch": self.ch, "n": self.n, "missing": self.missing, "extra": self.extra,
            "mean_ns": _f(self.mean_ns), "std_ns": _f(self.std_ns), "p2p_ns": _f(self.p2p_ns),
            "min_ns": _f(self.min_ns), "max_ns": _f(self.max_ns),
        }
        for fs in RATES:
            d[f"mean_samples_{fs // 1000}k"] = _f(ns_to_samples(self.mean_ns, fs), 4)
            d[f"p2p_samples_{fs // 1000}k"] = _f(ns_to_samples(self.p2p_ns, fs), 4)
        d.update({"drift_ns_per_s": _f(self.drift_ns_per_s), "period_ppm": _f(self.period_ppm, 3),
                  "period_tol_ppm": _f(self.period_tol_ppm, 3),
                  "threshold_ns": _f(self.threshold_ns), "verdict": self.verdict, "reasons": "; ".join(self.reasons)})
        return d


def _f(x: float, nd: int = 1) -> str:
    return "nan" if x is None or (isinstance(x, float) and math.isnan(x)) else f"{x:.{nd}f}"


@dataclass
class Limits:
    fs: int = 48000                 # rate the threshold is counted in
    threshold_samples: float = 1.0  # pass: every offset within +-this
    max_period_ppm: float = 10.0    # a unit whose edge period differs more is not locked
    max_missing_frac: float = 0.1   # more missing edges than this fails

    @property
    def threshold_ns(self) -> float:
        return self.threshold_samples * 1e9 / self.fs


def summarise(ref: str, ch: str, t_ref: np.ndarray, off: np.ndarray, missing: int, extra: int,
              period_ppm: float, limits: Limits, period_tol_ppm: float = float("nan")) -> PairResult:
    n = int(off.size)
    reasons = []
    if n == 0:
        mean = std = p2p = mn = mx = drift = float("nan")
        reasons.append("no matched edges")
    else:
        mean, std = float(off.mean()), float(off.std())
        mn, mx = float(off.min()), float(off.max())
        p2p = mx - mn
        drift = float(np.polyfit(t_ref / 1e9, off, 1)[0]) if n >= 3 and np.ptp(t_ref) > 0 else float("nan")
        thr = limits.threshold_ns
        if max(abs(mn), abs(mx)) > thr:
            reasons.append(f"offset beyond +-{limits.threshold_samples:g} sample(s) at {limits.fs} Hz "
                           f"(worst {max(abs(mn), abs(mx)):.0f} ns)")
    # Not locked only beyond the limit plus what edge quantisation alone can produce
    # (review scope1 T1: 4-5 edges at 7 us per sample give +-10 ppm by themselves).
    tol = 0.0 if math.isnan(period_tol_ppm) else period_tol_ppm
    if not math.isnan(period_ppm) and abs(period_ppm) > limits.max_period_ppm + tol:
        reasons.append(f"not locked: edge period {period_ppm:+.2f} ppm from the reference"
                       + (f" (quantisation allows +-{tol:.2f})" if tol else ""))
    total = n + missing
    if total and missing / total > limits.max_missing_frac:
        reasons.append(f"{missing} of {total} edges missing")
    return PairResult(ref, ch, n, missing, extra, mean, std, p2p, mn, mx, drift, period_ppm,
                      limits.threshold_ns, "FAIL" if reasons else "PASS", reasons, period_tol_ppm)


@dataclass
class Segment:
    """One capture window: edge times (ns, on the scope's time axis) per channel."""
    edges: dict[str, np.ndarray]
    span_ns: tuple[float, float]
    host_time_s: float = 0.0        # wall-clock start of the capture (long runs)
    match_ns: float | None = None   # match window override (triggered windows: half the window)
    dt_ns: float | None = None      # sample interval (edge quantisation +-dt/2); None: unknown


def analyse(segments: list[Segment], channels: list[str], ref: str, period_ns: float,
            limits: Limits, match_frac: float = 0.45) -> list[PairResult]:
    """Pairwise offsets of every channel against `ref`, pooled over the segments.

    Edge times of different segments are put on one axis by the segments'
    host times (seconds), so a long run's drift shows in drift_ns_per_s.
    """
    out = []
    for ch in channels:
        if ch == ref:
            continue
        ts, ds = [], []
        missing = extra = 0
        ppms, tols = [], []
        for seg in segments:
            r = seg.edges.get(ref, np.zeros(0))
            o = seg.edges.get(ch, np.zeros(0))
            m = pair_edges(r, o, period_ns, seg.span_ns, match_frac, seg.match_ns)
            ts.append(m.t_ref_ns + seg.host_time_s * 1e9)
            ds.append(m.offset_ns)
            missing += m.missing
            extra += m.extra
            pr, po = edge_period(r, period_ns), edge_period(o, period_ns)
            if pr and po:
                ppms.append((po / pr - 1.0) * 1e6)
                if seg.dt_ns:
                    tols.append((period_bound_ns(r, period_ns, seg.dt_ns) +
                                 period_bound_ns(o, period_ns, seg.dt_ns)) / pr * 1e6)
        t = np.concatenate(ts) if ts else np.zeros(0)
        d = np.concatenate(ds) if ds else np.zeros(0)
        ppm = float(np.median(ppms)) if ppms else float("nan")
        # every segment's estimate carries its own bound: the median of them bounds the median
        tol = float(max(tols)) if tols and len(tols) == len(ppms) else float("nan")
        out.append(summarise(ref, ch, t, d, missing, extra, ppm, limits, tol))
    return out


def long_run_drift(times_s: list[float], means_ns: list[float]) -> float:
    """ns per hour, least squares over the captures' mean offsets (nan with fewer than 2)."""
    t = np.asarray(times_s, dtype=float)
    m = np.asarray(means_ns, dtype=float)
    ok = ~np.isnan(m)
    if ok.sum() < 2 or np.ptp(t[ok]) == 0:
        return float("nan")
    return float(np.polyfit(t[ok], m[ok], 1)[0] * 3600.0)


def text_summary(results: list[PairResult], labels: dict[str, str] | None = None, title: str = "") -> str:
    labels = labels or {}
    lines = [title] if title else []
    for r in results:
        a = labels.get(r.ref, r.ref)
        b = labels.get(r.ch, r.ch)
        if r.n:
            lines.append(
                f"{r.verdict}  {b} vs {a}: {r.mean_ns:+.0f} ns = {r.samples(48000):+.3f} smp@48k / "
                f"{r.samples(96000):+.3f} smp@96k; jitter std {r.std_ns:.0f} ns, p2p {r.p2p_ns:.0f} ns; "
                f"n {r.n}, missing {r.missing}, drift {r.drift_ns_per_s:+.1f} ns/s"
                + ("" if math.isnan(r.period_ppm) else f", period {r.period_ppm:+.3f} ppm")
                + ("" if math.isnan(r.period_tol_ppm) else f" (+-{r.period_tol_ppm:.3f} from quantisation)"))
        else:
            lines.append(f"{r.verdict}  {b} vs {a}: no matched edges (missing {r.missing})")
        for why in r.reasons:
            lines.append(f"      - {why}")
        if not math.isnan(r.period_tol_ppm) and r.period_tol_ppm > 1.0:
            lines.append(f"      note: edge quantisation alone allows +-{r.period_tol_ppm:.1f} ppm on the period: the "
                         "not-locked check is weak here (more --periods, a faster timebase or --mode window)")
    verdict = "PASS" if results and all(r.verdict == "PASS" for r in results) else "FAIL"
    lines.append(f"OVERALL {verdict} (threshold +-{results[0].threshold_ns:.0f} ns)" if results else "OVERALL FAIL (no pairs)")
    return "\n".join(lines)
