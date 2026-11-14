"""WP SCOPE1: PicoScope 2000A (ps2000a driver) capture of the DSPi scope markers.

Runs only inside the alink-scope container with the scope passed in (README).
The driver module is injected (`lib`), so the tests drive this code with a fake.

Two capture shapes (the CLI's default `auto` picks span when its block resolves
edges to 100 ns, else window):
  span    one free-running block that covers `periods` marker periods at the
          fastest timebase the memory allows (needs memory: a 2206B MSO with
          32 MS covers 4 periods of 85 ms at 8 ns... 12 ns).
  window  repeated blocks triggered on the reference channel's rising edge,
          each a short window around it at the fastest timebase: for small
          memories (2205A MSO, 48 kS) or for the best resolution.
Digital channels D0..D15 (ports 0 and 1) with the logic threshold for 3.3 V
CMOS; without digital hardware, the analog channels A and B.
"""

from __future__ import annotations

import ctypes
import time
from dataclasses import dataclass

import numpy as np

from . import analysis

PICO_OK = 0
PICO_INVALID_TIMEBASE = 0x0E

INFO_KEYS = {               # PICO_INFO
    "driver_version": 0,
    "usb_version": 1,
    "hardware_version": 2,
    "variant": 3,
    "batch_and_serial": 4,
    "cal_date": 5,
    "kernel_version": 6,
    "digital_hardware_version": 7,
    "analogue_hardware_version": 8,
    "firmware_version_1": 9,
    "firmware_version_2": 0x0A,
}

DIGITAL_PORT0 = 0x80        # PS2000A_DIGITAL_PORT0 (D0-D7); PORT1 = 0x81 (D8-D15)
CH_A, CH_B = 0, 1
DC = 1
RANGE_5V = 8                # PS2000A_5V
RATIO_NONE = 0
LOGIC_FULL_SCALE_V = 5.0    # SetDigitalPort logicLevel: -32767..32767 = -5..+5 V
CONDITION_DONT_CARE, CONDITION_TRUE = 0, 1
DIGITAL_RISING = 3


class ScopeError(Exception):
    pass


def check(status: int, what: str) -> None:
    if status != PICO_OK:
        raise ScopeError(f"{what} failed: PICO_STATUS 0x{status:08X}")


def logic_level_counts(volts: float) -> int:
    """SetDigitalPort's logicLevel for a threshold in volts (1.5 V for 3.3 V CMOS)."""
    c = int(round(volts / LOGIC_FULL_SCALE_V * 32767))
    return max(-32767, min(32767, c))


def parse_channel(name: str) -> tuple[str, int]:
    """'D5' -> ('D', 5); 'A' -> ('A', 0)."""
    n = name.strip().upper()
    if n in ("A", "B"):
        return n, CH_A if n == "A" else CH_B
    if n.startswith("D") and n[1:].isdigit() and 0 <= int(n[1:]) <= 15:
        return "D", int(n[1:])
    raise ValueError(f"unknown channel {name!r} (D0..D15, A, B)")


@dataclass
class Block:
    """One capture: per-channel edges in ns plus where it lies."""
    segment: analysis.Segment
    dt_ns: float
    samples: int
    timebase: int
    raw: dict[str, np.ndarray] | None = None


class Ps2000a:
    def __init__(self, lib, sleep=time.sleep, clock=time.time):
        self.ps = lib
        self.h = ctypes.c_int16()
        self.sleep = sleep
        self.clock = clock
        self.open_ = False
        self.adc_max = 32512

    # -- unit ---------------------------------------------------------------

    def open(self) -> None:
        check(self.ps.ps2000aOpenUnit(ctypes.byref(self.h), None), "ps2000aOpenUnit")
        self.open_ = True

    def close(self) -> None:
        if self.open_:
            self.ps.ps2000aStop(self.h)
            self.ps.ps2000aCloseUnit(self.h)
            self.open_ = False

    def info(self) -> dict[str, str]:
        out = {}
        for key, code in INFO_KEYS.items():
            buf = ctypes.create_string_buffer(64)
            req = ctypes.c_int16()
            st = self.ps.ps2000aGetUnitInfo(self.h, buf, ctypes.c_int16(len(buf)), ctypes.byref(req), code)
            if st == PICO_OK:
                out[key] = buf.value.decode("ascii", "replace")
        return out

    @staticmethod
    def is_mso(info: dict[str, str]) -> bool:
        return "MSO" in info.get("variant", "").upper()

    # -- channels -------------------------------------------------------------

    def setup(self, channels: list[str], level_v: float, analog: bool) -> None:
        """Digital: ports for the D channels, analog off.  Analog: A/B at 5 V DC."""
        if analog:
            for name in ("A", "B"):
                _, ch = parse_channel(name)
                on = 1 if name in channels else 0
                check(self.ps.ps2000aSetChannel(self.h, ch, on, DC, RANGE_5V, ctypes.c_float(0.0)),
                      f"ps2000aSetChannel {name}")
            mx = ctypes.c_int16()
            if self.ps.ps2000aMaximumValue(self.h, ctypes.byref(mx)) == PICO_OK and mx.value > 0:
                self.adc_max = mx.value
            return
        for ch in (CH_A, CH_B):     # analog off: more memory and the fastest digital timebase
            self.ps.ps2000aSetChannel(self.h, ch, 0, DC, RANGE_5V, ctypes.c_float(0.0))
        ports = self.ports(channels)
        for p in (0, 1):
            check(self.ps.ps2000aSetDigitalPort(self.h, DIGITAL_PORT0 + p, 1 if p in ports else 0,
                                                logic_level_counts(level_v)),
                  f"ps2000aSetDigitalPort {p}")

    @staticmethod
    def ports(channels: list[str]) -> set[int]:
        return {parse_channel(c)[1] // 8 for c in channels if parse_channel(c)[0] == "D"}

    # -- timebase -------------------------------------------------------------

    def max_samples(self, segments: int = 1) -> int:
        n = ctypes.c_int32()
        check(self.ps.ps2000aMemorySegments(self.h, segments, ctypes.byref(n)), "ps2000aMemorySegments")
        return int(n.value)

    TB_MAX = 0xFFFFFFFF             # ps2000aGetTimebase2's timebase is a uint32

    def timebase(self, tb: int, samples: int) -> tuple[float, int] | None:
        """(interval ns, max samples) the driver reports for a timebase; None if it refuses it."""
        dt = ctypes.c_float()
        mx = ctypes.c_int32()
        st = self.ps.ps2000aGetTimebase2(self.h, tb, samples, ctypes.byref(dt), 0, ctypes.byref(mx), 0)
        return (float(dt.value), int(mx.value)) if st == PICO_OK else None

    def interval_ns(self, tb: int, samples: int) -> float | None:
        r = self.timebase(tb, samples)
        return r[0] if r else None

    def choose_timebase(self, span_ns: float, samples: int, tb_max: int | None = None) -> tuple[int, float]:
        """The fastest timebase whose `samples` cover span_ns, within what the driver accepts.

        Doubling from 1 finds the first accepted timebase that covers the span; if the
        driver refuses a timebase above an accepted one, the search stops there (its
        ceiling, review scope1 T2).  A binary search then finds the fastest one between
        the last accepted short one and it; a refused one there counts as too short
        (refusals at the fast end: too few samples or too many channels for that rate).
        """
        top = self.TB_MAX if tb_max is None else min(tb_max, self.TB_MAX)
        lo, hi, hi_dt, accepted, tb = 0, None, None, None, 1
        while True:
            dt = self.interval_ns(tb, samples)
            if dt is None:
                if accepted is not None:
                    break                   # the driver's ceiling: nothing larger to try
            else:
                if dt * samples >= span_ns:
                    hi, hi_dt = tb, dt
                    break
                accepted, lo = tb, tb + 1
            if tb >= top:
                break
            tb = min(top, tb * 2)
        if hi is None:
            longest = "" if accepted is None else f" (longest accepted: timebase {accepted}, " \
                f"{self.interval_ns(accepted, samples) * samples / 1e6:.1f} ms)"
            raise ScopeError(f"no timebase covers {span_ns / 1e6:.1f} ms with {samples} samples{longest}: "
                             "use fewer --periods or --mode window")
        if accepted is None:
            lo = 0
        while lo < hi:
            mid = (lo + hi) // 2
            dt = self.interval_ns(mid, samples)
            if dt is not None and dt * samples >= span_ns:
                hi, hi_dt = mid, dt
            else:
                lo = mid + 1
        return hi, hi_dt

    def fastest_timebase(self, samples: int, tb_max: int = 64) -> tuple[int, float]:
        for tb in range(tb_max):
            dt = self.interval_ns(tb, samples)
            if dt is not None:
                return tb, dt
        raise ScopeError("no valid timebase")

    # -- trigger --------------------------------------------------------------

    def trigger_off(self) -> None:
        self.ps.ps2000aSetTriggerChannelConditions(self.h, None, 0)

    def trigger_digital_rising(self, bit: int) -> None:
        cond = self.ps.PS2000A_TRIGGER_CONDITIONS(*([CONDITION_DONT_CARE] * 7 + [CONDITION_TRUE]))
        check(self.ps.ps2000aSetTriggerChannelConditions(self.h, ctypes.byref(cond), 1),
              "ps2000aSetTriggerChannelConditions")
        dirs = self.ps.PS2000A_DIGITAL_CHANNEL_DIRECTIONS(bit, DIGITAL_RISING)
        check(self.ps.ps2000aSetTriggerDigitalPortProperties(self.h, ctypes.byref(dirs), 1),
              "ps2000aSetTriggerDigitalPortProperties")

    # -- block ----------------------------------------------------------------

    def run_block(self, pre: int, post: int, tb: int, timeout_s: float) -> None:
        indisposed = ctypes.c_int32()
        check(self.ps.ps2000aRunBlock(self.h, pre, post, tb, 0, ctypes.byref(indisposed), 0, None, None),
              "ps2000aRunBlock")
        ready = ctypes.c_int16(0)
        t0 = self.clock()
        while True:
            self.ps.ps2000aIsReady(self.h, ctypes.byref(ready))
            if ready.value:
                return
            if self.clock() - t0 > timeout_s:
                self.ps.ps2000aStop(self.h)
                raise ScopeError(f"no data within {timeout_s:.1f} s (trigger never came?)")
            self.sleep(0.01)

    def fetch(self, sources: list[int], n: int) -> dict[int, np.ndarray]:
        bufs = {}
        for src in sources:
            b = np.zeros(n, dtype=np.int16)
            check(self.ps.ps2000aSetDataBuffer(self.h, src, b.ctypes.data_as(ctypes.POINTER(ctypes.c_int16)),
                                               n, 0, RATIO_NONE), f"ps2000aSetDataBuffer {src:#x}")
            bufs[src] = b
        got = ctypes.c_uint32(n)
        ovf = ctypes.c_int16()
        check(self.ps.ps2000aGetValues(self.h, 0, ctypes.byref(got), 1, RATIO_NONE, 0, ctypes.byref(ovf)),
              "ps2000aGetValues")
        return {k: v[: got.value] for k, v in bufs.items()}

    def edges(self, data: dict[int, np.ndarray], channels: list[str], dt_ns: float, t0_ns: float,
              analog: bool, level_v: float) -> dict[str, np.ndarray]:
        out = {}
        for name in channels:
            kind, num = parse_channel(name)
            if kind == "D":
                bits = analysis.bits_of_port(data[DIGITAL_PORT0 + num // 8], num % 8)
                out[name] = analysis.rising_edges_digital(bits, dt_ns, t0_ns)
            elif analog:
                counts = level_v / 5.0 * self.adc_max
                out[name] = analysis.rising_edges_analog(data[num], counts, dt_ns, 0.1 * counts, t0_ns)
        return out

    def capture_span(self, channels: list[str], span_ns: float, max_samples: int, analog: bool,
                     level_v: float, keep_raw: bool = False) -> Block:
        n = min(self.max_samples(1), max_samples)
        tb, dt = self.choose_timebase(span_ns, n)
        tbi = self.timebase(tb, n)
        if tbi and tbi[1] > 0:
            n = min(n, tbi[1])              # what the driver says this timebase can hold
        n = int(min(n, np.ceil(span_ns / dt)))
        self.trigger_off()
        host = self.clock()
        self.run_block(0, n, tb, timeout_s=5.0 + 2 * span_ns / 1e9)
        sources = self.sources(channels, analog)
        data = self.fetch(sources, n)
        got = min(len(v) for v in data.values())
        e = self.edges(data, channels, dt, 0.0, analog, level_v)
        seg = analysis.Segment(e, (0.0, got * dt), host, dt_ns=dt)
        return Block(seg, dt, got, tb, {f"{k:#x}": v for k, v in data.items()} if keep_raw else None)

    def capture_window(self, channels: list[str], ref: str, window_ns: float, analog: bool,
                       level_v: float, timeout_s: float = 2.0) -> Block:
        kind, num = parse_channel(ref)
        if kind != "D":
            raise ScopeError("window mode triggers on a digital reference channel")
        tb, dt = self.fastest_timebase(1024)
        n = int(min(self.max_samples(1), max(64, np.ceil(window_ns / dt))))
        tb, dt = self.fastest_timebase(n)
        self.trigger_digital_rising(num)
        pre = n // 2
        host = self.clock()
        self.run_block(pre, n - pre, tb, timeout_s)
        data = self.fetch(self.sources(channels, analog), n)
        got = min(len(v) for v in data.values())
        t0 = -pre * dt               # the trigger at t = 0
        e = self.edges(data, channels, dt, t0, analog, level_v)
        seg = analysis.Segment(e, (t0, t0 + got * dt), host, match_ns=0.95 * pre * dt, dt_ns=dt)
        return Block(seg, dt, got, tb)

    def sources(self, channels: list[str], analog: bool) -> list[int]:
        if analog:
            return [parse_channel(c)[1] for c in channels if parse_channel(c)[0] in ("A", "B")]
        return [DIGITAL_PORT0 + p for p in sorted(self.ports(channels))]


def load_driver():
    """The real driver (inside the container only)."""
    from picosdk.ps2000a import ps2000a  # noqa: import loads libps2000a
    return ps2000a
