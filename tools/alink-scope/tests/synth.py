"""WP SCOPE1 tests: synthetic marker edges and a fake ps2000a driver."""

from __future__ import annotations

import ctypes

import numpy as np

PERIOD_NS = 4096 * 1e9 / 48000      # N 4096 at 48 kHz: 85.33 ms


def marker_times(offset_ns: float, start_ns: float, end_ns: float, period_ns: float = PERIOD_NS,
                 jitter_ns: float = 0.0, ppm: float = 0.0, missing: tuple[int, ...] = (), phase0_ns: float = 1e6,
                 seed: int = 1) -> np.ndarray:
    """Rising edges of one unit: k * period * (1 + ppm) + offset (+ jitter), inside [start, end)."""
    rng = np.random.default_rng(seed)
    p = period_ns * (1 + ppm * 1e-6)
    k0 = int(np.floor((start_ns - phase0_ns - offset_ns) / p)) - 1
    k1 = int(np.ceil((end_ns - phase0_ns - offset_ns) / p)) + 1
    out = []
    for k in range(k0, k1 + 1):
        if k in missing:
            continue
        t = phase0_ns + k * p + offset_ns + (rng.uniform(-jitter_ns / 2, jitter_ns / 2) if jitter_ns else 0.0)
        if start_ns <= t < end_ns:
            out.append(t)
    return np.asarray(out)


def square(edges: np.ndarray, start_ns: float, dt_ns: float, n: int, width_ns: float = PERIOD_NS / 2) -> np.ndarray:
    """0/1 samples, high for width_ns after each edge (sample i at start + i*dt)."""
    t = start_ns + np.arange(n) * dt_ns
    out = np.zeros(n, dtype=np.int16)
    for e in edges:
        out[(t >= e) & (t < e + width_ns)] = 1
    return out


class TriggerConditions(ctypes.Structure):
    _pack_ = 1
    _fields_ = [(n, ctypes.c_int32) for n in
                ("channelA", "channelB", "channelC", "channelD", "external", "aux", "pulseWidthQualifier", "digital")]


class DigitalDirections(ctypes.Structure):
    _pack_ = 1
    _fields_ = [("channel", ctypes.c_int32), ("direction", ctypes.c_int32)]


class _Arr:
    def __init__(self, ptr, n):
        self.view = np.ctypeslib.as_array(ptr, shape=(n,))


class FakePs2000a:
    """Enough of picosdk's ps2000a for capture.Ps2000a, driven by synthetic units.

    units: {bit_index (0..15) -> edge offset ns}; timebase n gives 2**n * 2 ns for
    n < 3 and (n - 2) * 16 ns after (the 2206B's table), at most `memory` samples.
    """

    def __init__(self, units: dict[int, float], variant="2206B MSO", memory=2_000_000, start_ns=3.3e6,
                 min_tb=0, digital_ok=True, trig_at_ns=None, ppm: dict[int, float] | None = None,
                 max_tb: int | None = None):
        self.units = units
        self.variant = variant
        self.memory = memory
        self.start_ns = start_ns
        self.min_tb = min_tb
        self.digital_ok = digital_ok
        self.ppm = ppm or {}
        self.max_tb = max_tb
        self.tb_calls = []
        self.calls = []
        self.ports = {}
        self.analog_on = {}
        self.buffers = {}
        self.run = None
        self.trigger = None
        # the wrapper's structure types (picosdk/ps2000a.py), used by trigger_digital_rising()
        self.PS2000A_TRIGGER_CONDITIONS = TriggerConditions
        self.PS2000A_DIGITAL_CHANNEL_DIRECTIONS = DigitalDirections

    @staticmethod
    def _set(ref, v):
        ref._obj.value = v

    def ps2000aOpenUnit(self, h, serial):
        self._set(h, 7)
        return 0

    def ps2000aCloseUnit(self, h):
        self.calls.append("close")
        return 0

    def ps2000aStop(self, h):
        return 0

    def ps2000aGetUnitInfo(self, h, buf, n, req, code):
        vals = {3: self.variant.encode(), 4: b"JO123/0042", 0: b"2.2.159"}
        if code not in vals:
            return 0x2A
        ctypes.memmove(buf, vals[code], len(vals[code]))
        return 0

    def ps2000aSetChannel(self, h, ch, on, coupling, rng, off):
        self.analog_on[ch] = on
        return 0

    def ps2000aMaximumValue(self, h, ref):
        self._set(ref, 32512)
        return 0

    def ps2000aSetDigitalPort(self, h, port, on, level):
        if not self.digital_ok:
            return 0x11
        self.ports[port] = (on, level)
        return 0

    def ps2000aMemorySegments(self, h, n, ref):
        self._set(ref, self.memory // n)
        return 0

    def dt(self, tb):
        return float(2 ** tb * 2) if tb < 3 else float((tb - 2) * 16)

    def ps2000aGetTimebase2(self, h, tb, n, dt_ref, oversample, mx_ref, seg):
        self.tb_calls.append(tb)
        if tb < self.min_tb or n > self.memory or (self.max_tb is not None and tb > self.max_tb):
            return 0x0E
        self._set(dt_ref, self.dt(tb))
        self._set(mx_ref, self.memory if tb < 1000 else self.memory // 2)   # a smaller limit at slow timebases
        return 0

    def ps2000aSetTriggerChannelConditions(self, h, cond, n):
        self.trigger = None if n == 0 else self.trigger
        return 0

    def ps2000aSetTriggerDigitalPortProperties(self, h, dirs, n):
        self.trigger = dirs._obj.channel
        return 0

    def ps2000aRunBlock(self, h, pre, post, tb, os_, ind, seg, cb, par):
        self.run = (pre, post, tb)
        return 0

    def ps2000aIsReady(self, h, ref):
        self._set(ref, 1)
        return 0

    def ps2000aSetDataBuffer(self, h, src, ptr, n, seg, mode):
        self.buffers[src] = _Arr(ptr, n)
        return 0

    def _edges(self, bit, start, end):
        return marker_times(self.units[bit], start, end, ppm=self.ppm.get(bit, 0.0), seed=bit + 1)

    def ps2000aGetValues(self, h, start, nref, ratio, mode, seg, ovf):
        pre, post, tb = self.run
        n = pre + post
        dt = self.dt(tb)
        t0 = self.start_ns
        if self.trigger is not None:          # put the trigger channel's next edge at sample `pre`
            e = self._edges(self.trigger, self.start_ns, self.start_ns + 2 * PERIOD_NS)
            t0 = e[0] - pre * dt
        for src, arr in self.buffers.items():
            word = np.zeros(n, dtype=np.int16)
            if src >= 0x80:
                for bit in self.units:
                    if bit // 8 == src - 0x80:
                        e = self._edges(bit, t0 - PERIOD_NS, t0 + n * dt)
                        word |= (square(e, t0, dt, n) << (bit % 8)).astype(np.int16)
            else:
                e = self._edges(src, t0 - PERIOD_NS, t0 + n * dt)
                word = (square(e, t0, dt, n) * 26000).astype(np.int16)  # 4 V on the 5 V range
            arr.view[:n] = word
        self._set(nref, n)
        self.start_ns += n * dt + 7.77e6      # the next block starts later
        return 0
