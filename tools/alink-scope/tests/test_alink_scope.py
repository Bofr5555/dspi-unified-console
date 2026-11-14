"""WP SCOPE1: unit tests of the alink scope tool with synthetic edges (no scope).

Run: tools/alink-scope/scope.sh test   (in the alink-scope container)
"""

from __future__ import annotations

import os
import sys
import tempfile
import unittest

import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.dirname(__file__))

from alink_scope import analysis, capture, cli  # noqa: E402
from synth import PERIOD_NS, FakePs2000a, marker_times, square  # noqa: E402

LIM = analysis.Limits()          # +-1 sample at 48 kHz = 20833 ns
SPAN = (0.0, 4.2 * PERIOD_NS)


def tmpdir():
    base = os.environ.get("ALINK_SCOPE_TMP") or os.path.join(os.path.expanduser("~"), ".cache", "alink-scope-tests")
    os.makedirs(base, exist_ok=True)
    return tempfile.TemporaryDirectory(dir=base)


def seg(units: dict[str, dict], span=SPAN, host=0.0):
    return analysis.Segment({k: marker_times(start_ns=span[0], end_ns=span[1], **v) for k, v in units.items()},
                            span, host)


def by_ch(results):
    return {r.ch: r for r in results}


class EdgesTest(unittest.TestCase):
    def test_wp_scope1_digital_edges(self):
        bits = np.array([1, 1, 0, 0, 1, 1, 1, 0, 1])
        e = analysis.rising_edges_digital(bits, 10.0, 100.0)
        np.testing.assert_allclose(e, [135.0, 175.0])    # a high first sample is not an edge

    def test_wp_scope1_port_bits(self):
        port = np.array([0b0000_0100, 0b1000_0100, 0], dtype=np.int16)
        np.testing.assert_array_equal(analysis.bits_of_port(port, 2), [1, 1, 0])
        np.testing.assert_array_equal(analysis.bits_of_port(port, 7), [0, 1, 0])

    def test_wp_scope1_analog_edges_interpolated_with_hysteresis(self):
        v = np.array([0, 0, 1, 3, 2.9, 3.1, 0, 0, 4, 4], dtype=float)
        e = analysis.rising_edges_analog(v, 1.5, 10.0, hysteresis=0.5)
        np.testing.assert_allclose(e, [22.5, 70 + 1.5 / 4 * 10])   # the 2.9 dip does not re-arm


class PairTest(unittest.TestCase):
    def test_wp_scope1_aligned_units_pass(self):
        r = by_ch(analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=5000),
                                          "D2": dict(offset_ns=-12000)})], ["D0", "D1", "D2"], "D0", PERIOD_NS, LIM))
        self.assertEqual(r["D1"].verdict, "PASS")
        self.assertAlmostEqual(r["D1"].mean_ns, 5000, places=3)
        self.assertAlmostEqual(r["D2"].mean_ns, -12000, places=3)
        self.assertAlmostEqual(r["D2"].samples(48000), -0.576, places=3)
        self.assertAlmostEqual(r["D2"].samples(96000), -1.152, places=3)
        self.assertEqual((r["D1"].n, r["D1"].missing, r["D1"].extra), (5, 0, 0))

    def test_wp_scope1_wraparound_at_capture_ends(self):
        # B leads A by 3 us and the capture starts 1 us after a B edge but before
        # A's: B's first edge and A's last edge have no partner inside the capture.
        a0 = 1e6 + 20 * PERIOD_NS
        span = (a0 - 2000.0, a0 + 3 * PERIOD_NS + 1000.0)
        s = seg({"A": dict(offset_ns=0), "B": dict(offset_ns=-3000)}, span)
        self.assertEqual(s.edges["A"].size, 4)
        self.assertEqual(s.edges["B"].size, 3)
        r = analysis.analyse([s], ["A", "B"], "A", PERIOD_NS, LIM)[0]
        self.assertEqual((r.n, r.missing, r.extra, r.verdict), (3, 0, 0, "PASS"))
        self.assertAlmostEqual(r.mean_ns, -3000, places=3)

    def test_wp_scope1_offset_near_half_period_is_not_matched(self):
        r = analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=0.49 * PERIOD_NS)})],
                             ["D0", "D1"], "D0", PERIOD_NS, LIM)[0]
        self.assertEqual(r.n, 0)
        self.assertEqual(r.verdict, "FAIL")

    def test_wp_scope1_missing_edges(self):
        span = (0.0, 30 * PERIOD_NS)
        few = by_ch(analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=800, missing=(7,)),
                                           "D2": dict(offset_ns=0, missing=(3,))}, span)],
                                     ["D0", "D1", "D2"], "D0", PERIOD_NS, LIM))
        self.assertEqual((few["D1"].missing, few["D1"].verdict), (1, "PASS"))
        self.assertEqual(few["D2"].extra, 0)
        # the reference misses one: the other unit's edge is extra, not an error in its offset
        self.assertAlmostEqual(few["D1"].mean_ns, 800, places=3)
        many = analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=0, missing=tuple(range(0, 30, 2)))},
                                     span)], ["D0", "D1"], "D0", PERIOD_NS, LIM)[0]
        self.assertEqual(many.verdict, "FAIL")
        self.assertTrue(any("missing" in x for x in many.reasons))
        refmiss = analysis.analyse([seg({"D0": dict(offset_ns=0, missing=(5,)), "D1": dict(offset_ns=0)}, span)],
                                   ["D0", "D1"], "D0", PERIOD_NS, LIM)[0]
        self.assertEqual((refmiss.extra, refmiss.missing, refmiss.verdict), (1, 0, "PASS"))

    def test_wp_scope1_unit_out_of_lock_fails(self):
        span = (0.0, 12 * PERIOD_NS)
        r = by_ch(analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=2000),
                                         "D2": dict(offset_ns=2000, ppm=25.0)}, span)],
                                   ["D0", "D1", "D2"], "D0", PERIOD_NS, LIM))
        self.assertEqual(r["D1"].verdict, "PASS")
        self.assertEqual(r["D2"].verdict, "FAIL")
        self.assertAlmostEqual(r["D2"].period_ppm, 25.0, delta=0.01)
        self.assertTrue(any("not locked" in x for x in r["D2"].reasons))
        self.assertAlmostEqual(r["D2"].drift_ns_per_s, 25.0 * 1e3, delta=50)   # 25 ppm = 25 us/s

    def test_wp_scope1_threshold_and_jitter(self):
        span = (0.0, 40 * PERIOD_NS)
        r = by_ch(analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=15000, jitter_ns=4000),
                                         "D2": dict(offset_ns=22000)}, span)], ["D0", "D1", "D2"], "D0",
                                   PERIOD_NS, LIM))
        self.assertEqual(r["D1"].verdict, "PASS")
        self.assertLessEqual(r["D1"].p2p_ns, 4000)
        self.assertGreater(r["D1"].p2p_ns, 2500)
        self.assertAlmostEqual(r["D1"].std_ns, 4000 / np.sqrt(12), delta=400)
        self.assertEqual(r["D2"].verdict, "FAIL")         # 22 us > 1 sample (20.8 us)
        half = analysis.analyse([seg({"D0": dict(offset_ns=0), "D2": dict(offset_ns=22000)}, span)], ["D0", "D2"],
                                "D0", PERIOD_NS, analysis.Limits(fs=48000, threshold_samples=2))[0]
        self.assertEqual(half.verdict, "PASS")

    def test_wp_scope1_long_run_drift(self):
        t = [0, 600, 1200, 1800, 2400, 3000]
        self.assertAlmostEqual(analysis.long_run_drift(t, [100, 110, 120, 130, 140, 150]), 60.0, places=6)
        self.assertTrue(np.isnan(analysis.long_run_drift([5], [1.0])))

    def test_wp_scope1_window_segments(self):
        # triggered windows: ref edge at 0, +-100 us; a unit 150 us late is outside: missing
        segs = []
        for i in range(5):
            segs.append(analysis.Segment({"D0": np.array([0.0]), "D1": np.array([1200.0 + i]),
                                          "D2": np.array([150_000.0])}, (-100_000.0, 100_000.0), 10.0 * i,
                                         match_ns=95_000.0))
        r = by_ch(analysis.analyse(segs, ["D0", "D1", "D2"], "D0", PERIOD_NS, LIM))
        self.assertEqual((r["D1"].n, r["D1"].verdict), (5, "PASS"))
        self.assertAlmostEqual(r["D1"].mean_ns, 1202.0)
        self.assertEqual((r["D2"].n, r["D2"].missing, r["D2"].verdict), (0, 5, "FAIL"))

    def test_wp_scope1_summary_text(self):
        res = analysis.analyse([seg({"D0": dict(offset_ns=0), "D1": dict(offset_ns=1000)})], ["D0", "D1"], "D0",
                               PERIOD_NS, LIM)
        txt = analysis.text_summary(res, {"D0": "k340", "D1": "pico2"})
        self.assertIn("PASS  pico2 vs k340: +1000 ns = +0.048 smp@48k / +0.096 smp@96k", txt)
        self.assertIn("OVERALL PASS", txt)


class CaptureTest(unittest.TestCase):
    def test_wp_scope1_logic_level_and_channels(self):
        self.assertEqual(capture.logic_level_counts(1.5), 9830)
        self.assertEqual(capture.logic_level_counts(9.0), 32767)
        self.assertEqual(capture.parse_channel("d12"), ("D", 12))
        self.assertEqual(capture.parse_channel("B"), ("B", 1))
        with self.assertRaises(ValueError):
            capture.parse_channel("D16")
        self.assertEqual(capture.Ps2000a.ports(["D0", "D9", "D3"]), {0, 1})

    def test_wp_scope1_timebase_search(self):
        fake = FakePs2000a({0: 0.0}, memory=1_000_000, min_tb=1)
        sc = capture.Ps2000a(fake)
        sc.open()
        tb, dt = sc.choose_timebase(4 * PERIOD_NS, 1_000_000)
        self.assertGreaterEqual(dt * 1_000_000, 4 * PERIOD_NS)
        self.assertLess(fake.dt(tb - 1) * 1_000_000, 4 * PERIOD_NS)   # the fastest that covers it
        self.assertEqual(sc.fastest_timebase(1000), (1, 4.0))           # tb 0 invalid here
        with self.assertRaises(capture.ScopeError):
            sc.choose_timebase(1e18, 1_000_000, tb_max=1000)

    def test_wp_scope1_span_capture_digital(self):
        fake = FakePs2000a({0: 0.0, 1: 4000.0, 9: -7000.0}, memory=600_000)
        sc = capture.Ps2000a(fake, sleep=lambda s: None)
        sc.open()
        info = sc.info()
        self.assertTrue(sc.is_mso(info))
        sc.setup(["D0", "D1", "D9"], 1.5, analog=False)
        self.assertEqual(fake.ports[0x80], (1, 9830))
        self.assertEqual(fake.ports[0x81], (1, 9830))
        self.assertEqual(fake.analog_on, {0: 0, 1: 0})
        b = sc.capture_span(["D0", "D1", "D9"], 3 * PERIOD_NS, 10_000_000, False, 1.5)
        self.assertLessEqual(b.samples, 600_000)
        r = by_ch(analysis.analyse([b.segment], ["D0", "D1", "D9"], "D0", PERIOD_NS, LIM))
        self.assertAlmostEqual(r["D1"].mean_ns, 4000, delta=b.dt_ns)
        self.assertAlmostEqual(r["D9"].mean_ns, -7000, delta=b.dt_ns)
        self.assertEqual(r["D9"].verdict, "PASS")
        sc.close()
        self.assertIn("close", fake.calls)

    def test_wp_scope1_window_capture(self):
        fake = FakePs2000a({0: 0.0, 2: 1500.0}, memory=48_000)
        sc = capture.Ps2000a(fake, sleep=lambda s: None)
        sc.open()
        sc.setup(["D0", "D2"], 1.5, analog=False)
        segs = [sc.capture_window(["D0", "D2"], "D0", 20_000.0, False, 1.5).segment for _ in range(3)]
        self.assertEqual(fake.trigger, 0)
        r = analysis.analyse(segs, ["D0", "D2"], "D0", PERIOD_NS, LIM)[0]
        self.assertEqual(r.n, 3)
        self.assertAlmostEqual(r.mean_ns, 1500, delta=4.0)

    def test_wp_scope1_analog_fallback(self):
        fake = FakePs2000a({0: 0.0, 1: 2500.0}, variant="2206B", memory=400_000)
        sc = capture.Ps2000a(fake, sleep=lambda s: None)
        sc.open()
        self.assertFalse(sc.is_mso(sc.info()))
        sc.setup(["A", "B"], 1.5, analog=True)
        self.assertEqual(fake.analog_on, {0: 1, 1: 1})
        b = sc.capture_span(["A", "B"], 2.5 * PERIOD_NS, 10_000_000, True, 1.5)
        r = analysis.analyse([b.segment], ["A", "B"], "A", PERIOD_NS, LIM)[0]
        self.assertAlmostEqual(r.mean_ns, 2500, delta=b.dt_ns)


def quantised(edges, t0, dt):
    """What the digital edge extractor reports: the first sample at or after the edge, less dt/2."""
    return t0 + (np.ceil((np.asarray(edges) - t0) / dt) - 0.5) * dt


class QuantisationTest(unittest.TestCase):
    """Review scope1 T1: a 48 kS scope in span mode must not call locked units 'not locked'."""

    CASES = ((4, 4 * PERIOD_NS / 48_000), (4, 7008.0), (8, 8 * PERIOD_NS / 48_000))   # periods, dt ns

    def trials(self, use_dt: bool, n=300):
        rng = np.random.default_rng(7)
        fails = 0
        for periods, dt in self.CASES:
            for _ in range(n):
                t0 = rng.uniform(0, PERIOD_NS)
                span = (t0, t0 + periods * PERIOD_NS)
                edges = {}
                for ch in ("D0", "D1", "D2"):
                    e = marker_times(rng.uniform(-10_000, 10_000), span[0], span[1], phase0_ns=rng.uniform(0, 1e6))
                    edges[ch] = quantised(e, t0, dt)
                sg = analysis.Segment(edges, span, 0.0, dt_ns=dt if use_dt else None)
                for r in analysis.analyse([sg], ["D0", "D1", "D2"], "D0", PERIOD_NS, LIM):
                    fails += any("not locked" in x for x in r.reasons)
        return fails

    def test_wp_scope1_48ks_span_no_false_not_locked(self):
        self.assertEqual(self.trials(use_dt=True), 0)
        self.assertGreater(self.trials(use_dt=False, n=100), 0)   # the review's finding, without the bound

    def test_wp_scope1_48ks_fake_scope_span(self):
        fails = 0
        for k in range(30):
            fake = FakePs2000a({0: 0.0, 1: 3000.0 + 37 * k, 2: -8000.0}, memory=48_000, start_ns=1e6 + 1.234e6 * k)
            sc = capture.Ps2000a(fake, sleep=lambda s: None)
            sc.open()
            sc.setup(["D0", "D1", "D2"], 1.5, analog=False)
            b = sc.capture_span(["D0", "D1", "D2"], 4 * PERIOD_NS, 50_000_000, False, 1.5)
            self.assertGreater(b.dt_ns, 5000)
            self.assertEqual(b.segment.dt_ns, b.dt_ns)
            for r in analysis.analyse([b.segment], ["D0", "D1", "D2"], "D0", PERIOD_NS, LIM):
                fails += r.verdict != "PASS"
                self.assertGreater(r.period_tol_ppm, LIM.max_period_ppm)   # the check is weak here, and says so
        self.assertEqual(fails, 0)

    def test_wp_scope1_out_of_lock_still_caught_with_fine_dt(self):
        span = (0.0, 4 * PERIOD_NS)
        e = {"D0": quantised(marker_times(0, *span), 0.0, 12.0),
             "D1": quantised(marker_times(2000, *span, ppm=25.0), 0.0, 12.0)}
        r = analysis.analyse([analysis.Segment(e, span, 0.0, dt_ns=12.0)], ["D0", "D1"], "D0", PERIOD_NS, LIM)[0]
        self.assertLess(r.period_tol_ppm, 1.0)
        self.assertTrue(any("not locked" in x for x in r.reasons))

    def test_wp_scope1_auto_mode(self):
        self.assertEqual(cli.auto_mode(48_000, 48_000, 4 * PERIOD_NS, False), "window")
        self.assertEqual(cli.auto_mode(32_000_000, 32_000_000, 4 * PERIOD_NS, False), "span")
        self.assertEqual(cli.auto_mode(48_000, 48_000, 4 * PERIOD_NS, True), "span")   # no digital trigger


class TimebaseClampTest(unittest.TestCase):
    """Review scope1 T2: the timebase search stays within what the driver accepts."""

    def test_wp_scope1_timebase_ceiling(self):
        fake = FakePs2000a({0: 0.0}, memory=1_000_000, max_tb=5000)
        sc = capture.Ps2000a(fake)
        sc.open()
        with self.assertRaises(capture.ScopeError) as cm:
            sc.choose_timebase(1e18, 1_000_000)
        self.assertIn("longest accepted: timebase", str(cm.exception))
        self.assertLessEqual(max(fake.tb_calls), 2 * 5000)       # stopped at the first refusal above
        tb, dt = sc.choose_timebase(4 * PERIOD_NS, 1_000_000)
        self.assertLessEqual(tb, 5000)
        self.assertGreaterEqual(dt * 1_000_000, 4 * PERIOD_NS)
        self.assertLess(fake.dt(tb - 1) * 1_000_000, 4 * PERIOD_NS)

    def test_wp_scope1_samples_clamped_to_driver(self):
        fake = FakePs2000a({0: 0.0, 1: 0.0}, memory=2000)          # tb >= 1000 holds only half the memory
        sc = capture.Ps2000a(fake, sleep=lambda s: None)
        sc.open()
        sc.setup(["D0", "D1"], 1.5, analog=False)
        b = sc.capture_span(["D0", "D1"], 2 * PERIOD_NS, 50_000_000, False, 1.5)
        self.assertGreaterEqual(b.timebase, 1000)
        self.assertLessEqual(b.samples, 1000)


def make_run(d, label, offsets, caps=3, units=None, step_ns=100.0):
    """A capture directory as `capture` writes it: meta.json, edges.csv, pairs.csv."""
    run = cli.Run(label, units or {"D0": "k340", "D1": "pico2", "D2": "pico"}, list(offsets), "D0", 4096, 48000,
                  analysis.Limits())
    w = cli.Writer(d, run)
    for cap in range(caps):
        s = seg({ch: dict(offset_ns=o + step_ns * cap * (ch != "D0"), seed=cap + 1) for ch, o in offsets.items()},
                host=600.0 * cap)
        w.segment(cap, s)
        w.results(cap, s.host_time_s, analysis.analyse([s], run.chans, run.ref, run.period_ns, run.limits))
    return run, w


class CliTest(unittest.TestCase):
    def test_wp_scope1_edges_csv_round_trip(self):
        with tmpdir() as d:
            run, w = make_run(d, "active 5 m, brand A", {"D0": 0.0, "D1": 3000.0})
            w.segment(3, analysis.Segment({}, SPAN, 1800.0))     # a capture with no edges at all
            w.close()
            with open(os.path.join(d, "edges.csv")) as f:
                head, first = f.readline(), f.readline()
            self.assertIn("run_label,unit,channel", head)
            self.assertIn("active 5 m, brand A", first)
            self.assertIn("k340,D0", first)
            out = os.path.join(d, "re")
            rc = cli.main(["analyse", os.path.join(d, "edges.csv"), "--out", out])   # settings from meta.json
            with open(os.path.join(out, "summary.txt")) as f:
                text = f.read()
            self.assertEqual(rc, 1)                  # capture 3 has no edges: its pair fails, pooled still fine
            self.assertIn("run 'active 5 m, brand A'", text)
            self.assertIn("pico2 vs k340: +3100 ns", text)
            self.assertIn("long run", text)
            self.assertIn("drift +600.0 ns/hour", text)
            self.assertIn("failed captures 1/4", text)
            with open(os.path.join(out, "pairs.csv")) as f:
                rows = list(cli.csv.DictReader(f))
            self.assertEqual({(r["run_label"], r["ref_unit"], r["unit"]) for r in rows},
                             {("active 5 m, brand A", "k340", "pico2")})

    def test_wp_scope1_compare_runs(self):
        with tmpdir() as d:
            a, b = os.path.join(d, "cable-a"), os.path.join(d, "cable-b")
            make_run(a, "passive 2 m", {"D0": 0.0, "D1": 1000.0, "D2": -500.0}, step_ns=0.0)[1].close()
            make_run(b, "active 15 m", {"D0": 0.0, "D1": 1100.0, "D2": 25_000.0}, step_ns=50.0)[1].close()
            out = os.path.join(d, "cmp")
            rc = cli.main(["compare", a, b, "--out", out])
            self.assertEqual(rc, 1)                  # pico on the 15 m cable is beyond 1 sample
            with open(os.path.join(out, "compare.csv")) as f:
                rows = {(r["run_label"], r["unit"]): r for r in cli.csv.DictReader(f)}
            self.assertEqual(len(rows), 4)
            self.assertAlmostEqual(float(rows[("passive 2 m", "pico2")]["mean_ns"]), 1000.0, places=2)
            self.assertEqual(rows[("passive 2 m", "pico")]["verdict"], "PASS")
            self.assertEqual(rows[("active 15 m", "pico")]["verdict"], "FAIL")
            self.assertAlmostEqual(float(rows[("active 15 m", "pico2")]["drift_ns_per_hour"]), 300.0, places=1)
            with open(os.path.join(out, "compare.txt")) as f:
                table = f.read()
            self.assertIn("run label", table.splitlines()[0])
            self.assertIn("active 15 m", table)
            self.assertIn("+25050", table)


if __name__ == "__main__":
    unittest.main()
