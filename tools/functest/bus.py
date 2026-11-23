#!/usr/bin/env python3
"""The host audio bus (WP FBUS, FSTORY 3S.2, piece P11): the computer's output devices and the
streams playing to them, as the simulator reads them, and the levels it logs. Test-only.

A run's bus is `sim/bus.json` (the app's DSPI_SIM_OUT); the simulator reads it again whenever it
changes and writes `sim/bus.jsonl`: events, and one `tick` per 100 ms while anything plays (what
plays where, dBFS at each DSPi input and output, the limiter, dB SPL per driver and at the seat,
clipping). Programme in the damage oracle goes to `sim/oracle.jsonl` (`how: programme`).
`dspi_get_host_audio` reads the same file (the outputs, the default, what plays, OS volumes).

The bus file (every key camelCase; defaults in brackets):

    {"os": "linux"|"windows"|"macos" [linux], "audioSystem": "PipeWire 1.6" [""],
     "micSplAt0Dbfs": 115,           # dB SPL at the seat that 0 dBFS at the simulated mic is
     "outputs": [{"name", "id" [name], "serial" ["": text holding a USB serial], "channels",
                  "rateHz" [48000], "default" [false], "volumePct" [100], "muted" [false],
                  "hardwareVolume" [false: the OS scales samples; true: the OS volume is the DSPi's
                                    user volume, both ways (UAC)],
                  "map": [{"channel", "serial", "input"}]   # 0-based; none: plays nowhere
                 }],
     "streams": [{"id", "app", "title" [""], "channels", "lufs" [-14], "truePeakDbtp" [-1],
                  "content": music|jazz|edm|film|speech|game|pink|sweep|tone:HZ [music],
                  "state": play|pause [play], "output" (an output's name or id), "volumePct" [100],
                  "muted" [false], "channelDb" [per channel dB on top, -200 silent],
                  "envelope": [{"atS", "lufs", "truePeakDbtp"?}], "startedUnixS",
                  "testSignal" [false: a sweep/tone file played through the OS]}]}

Several writers (the harness, the stand-ins P12-P15) share the file: every change goes through
`Bus.edit()` (a lock file, then an atomic replace).

    bus.py RUN_DIR show                                     the bus file
    bus.py RUN_DIR play ID --app A --output O [--title T --channels 2 --lufs -14 --peak -1 --content jazz]
    bus.py RUN_DIR pause|resume|stop ID
    bus.py RUN_DIR move ID OUTPUT
    bus.py RUN_DIR volume ID PCT            an app's own volume
    bus.py RUN_DIR output-volume OUTPUT PCT the OS volume of an output device
    bus.py RUN_DIR default OUTPUT
    bus.py RUN_DIR levels                   the last tick, short

Replay scripts (replay.py) and stories use it through `replay_call`:

    ["bus_setup", TABLE, "ok"]                   the bus from a table (as a story's [bus], see setup)
    ["bus", {"do": "play", "id": ..., ...}, "ok"]  any Bus method by name, with its arguments
    ["bus_expect", {"device": "Fronts", "output": 1, "rms_dbfs": [-37.3, -35.3], "within_s": 5}, "ok"]
        waits until the latest tick has it: `output` or `input` (1-based) of `device`, with any of
        rms_dbfs / peak_dbfs / spl_db / limiter_db as [low, high], "silent": true, "clip": bool;
        or {"stream": ID, "reaches": ["Fronts in 1", ...]} (exactly), or {"seat_spl_db": [lo, hi]}
"""
import contextlib
import fcntl
import json
import os
import pathlib
import sys
import time


def serial_text(serial):
    """What udev shows for a DSPi's USB serial (`Vendor_Product_Serial`): how the app matches it."""
    return f"WeebLabs_DSPi_{serial}"


class Bus:
    def __init__(self, run_dir):
        self.dir = pathlib.Path(run_dir)
        self.path = self.dir / "sim" / "bus.json"
        self.log = self.dir / "sim" / "bus.jsonl"

    # ------------------------------------------------------------ the file

    def read(self):
        try:
            return json.loads(self.path.read_text())
        except (OSError, ValueError):
            return {"outputs": [], "streams": []}

    @contextlib.contextmanager
    def edit(self):
        """The bus, to change in place; written when the block ends."""
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.path.with_suffix(".lock"), "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            bus = self.read()
            bus.setdefault("outputs", [])
            bus.setdefault("streams", [])
            yield bus
            tmp = self.path.with_suffix(f".{os.getpid()}.new")
            tmp.write_text(json.dumps(bus, indent=1))
            os.replace(tmp, self.path)
        # The simulator looks every 100 ms and keys on the file's time and size
        time.sleep(0.15)

    def write(self, bus):
        with self.edit() as b:
            b.clear()
            b.update(bus)

    # ------------------------------------------------------------ outputs

    def stream(self, bus, sid):
        for s in bus["streams"]:
            if s["id"] == sid:
                return s
        raise KeyError(f"no stream {sid!r} on the bus")

    def output(self, bus, name):
        for o in bus["outputs"]:
            if name in (o["name"], o.get("id")):
                return o
        raise KeyError(f"no output {name!r} on the bus")

    def add_output(self, **o):
        with self.edit() as b:
            b["outputs"] = [x for x in b["outputs"] if x["name"] != o["name"]] + [o]

    def output_volume(self, output, pct, muted=None):
        with self.edit() as b:
            o = self.output(b, output)
            o["volumePct"] = pct
            if muted is not None:
                o["muted"] = muted

    def default(self, output):
        with self.edit() as b:
            self.output(b, output)
            for o in b["outputs"]:
                o["default"] = output in (o["name"], o.get("id"))

    # ------------------------------------------------------------ streams

    def play(self, id, app, output, **kw):
        """A stream starts (or restarts) on `output`."""
        s = {"id": id, "app": app, "output": output, "channels": kw.pop("channels", 2), "state": "play",
             "startedUnixS": time.time()}
        s.update(kw)
        with self.edit() as b:
            b["streams"] = [x for x in b["streams"] if x["id"] != id] + [s]

    def set_stream(self, id, **fields):
        with self.edit() as b:
            self.stream(b, id).update(fields)

    def pause(self, id):
        self.set_stream(id, state="pause")

    def resume(self, id):
        self.set_stream(id, state="play")

    def move(self, id, output):
        self.set_stream(id, output=output)

    def volume(self, id, pct):
        self.set_stream(id, volumePct=pct)

    def stop(self, id):
        with self.edit() as b:
            b["streams"] = [x for x in b["streams"] if x["id"] != id]

    # ------------------------------------------------------------ the log

    def ticks(self):
        try:
            lines = self.log.read_text().splitlines()
        except OSError:
            return []
        out = []
        for line in lines:
            try:
                x = json.loads(line)
            except ValueError:
                continue
            if x.get("kind") == "tick":
                out.append(x)
        return out

    def last(self):
        t = self.ticks()
        return t[-1] if t else None

    def events(self):
        try:
            return [x for x in map(json.loads, self.log.read_text().splitlines()) if x.get("kind") != "tick"]
        except (OSError, ValueError):
            return []


def setup(run_dir, table):
    """The bus from a story's `[bus]` table (none: nothing). Besides the file's own keys, an output
    may say `usb = "<device name>"` (that scenario device on USB: its serial text, every input one
    to one, `channels` its input count) or `to = [[channel, "<device name>", input], ...]`
    (1-based channel and input, as a person writes them)."""
    if not table:
        return
    run_dir = pathlib.Path(run_dir)
    scenario = json.loads((run_dir / "scenario.json").read_text())
    serial = {d["name"]: d["serial"] for d in scenario["devices"]}
    outputs = []
    for o in table.get("outputs", []):
        o = dict(o)
        if "usb" in o:
            dev = o.pop("usb")
            n = o.setdefault("channels", 8)
            o.setdefault("serial", serial_text(serial[dev]))
            o["map"] = [{"channel": c, "serial": serial[dev], "input": c} for c in range(n)]
        if "to" in o:
            o["map"] = [{"channel": c - 1, "serial": serial[d], "input": i - 1} for c, d, i in o.pop("to")]
        outputs.append(o)
    bus = {k: v for k, v in table.items() if k not in ("outputs",)}
    bus["outputs"] = outputs
    bus.setdefault("streams", [])
    Bus(run_dir).write(bus)


# ---------------------------------------------------------------- replays

def _within(v, rng):
    return v is not None and rng[0] <= v <= rng[1]


def _check(tick, a):
    """Whether one tick has what `a` asks; (ok, what it has)."""
    if tick is None:
        return bool(a.get("silent")), "nothing has played"
    if "stream" in a:
        s = next((x for x in tick["streams"] if x["id"] == a["stream"]), None)
        if s is None:
            return False, f"no stream {a['stream']}"
        ok = sorted(s["reaches"]) == sorted(a.get("reaches", s["reaches"]))
        if "playing" in a:
            ok = ok and s["playing"] == a["playing"]
        return ok, {"reaches": s["reaches"], "playing": s["playing"], "rmsDbfs": s["rmsDbfs"]}
    if "seat_spl_db" in a:
        return _within(tick.get("seatSplDb"), a["seat_spl_db"]), {"seatSplDb": tick.get("seatSplDb")}
    side = "output" if "output" in a else "input"
    rows = tick["outputs" if side == "output" else "inputs"]
    x = next((r for r in rows if r["device"] == a["device"] and r[side] == a[side] - 1), None)
    if x is None or (side == "output" and x.get("generator")):
        return bool(a.get("silent")), "silent"
    if a.get("silent"):
        return x["rmsDbfs"] <= -120, x
    ok = all(_within(x.get(k), a[key]) for key, k in (("rms_dbfs", "rmsDbfs"), ("peak_dbfs", "peakDbfs"),
                                                       ("spl_db", "splDb"), ("limiter_db", "limiterDb"))
             if key in a)
    if "clip" in a:
        ok = ok and x["clip"] == a["clip"]
    return ok, {k: x.get(k) for k in ("rmsDbfs", "peakDbfs", "splDb", "limiterDb", "clip")}


def expect(run_dir, a):
    """Waits until the bus's latest tick has `a` (bus_expect); (ok, what it had last). A tick older
    than the call counts only once nothing plays (the bus logs no more ticks then)."""
    bus = Bus(run_dir)
    end = time.time() + float(a.get("within_s", 5))
    seen = len(bus.ticks())
    got = "no tick"
    while True:
        ticks = bus.ticks()
        late = time.time() >= end
        quiet = bool(ticks) and not any(s["playing"] for s in ticks[-1]["streams"])
        if len(ticks) > seen or quiet or late:
            ok, got = _check(ticks[-1] if ticks else None, a)
            if ok:
                return True, got
        if late:
            return False, got
        time.sleep(0.2)


def replay_call(run_dir, c):
    """One `bus_*` call of a replay script, as replay.Player reports a tool call."""
    tool, args = c[0], dict(c[1])
    want = c[2] if len(c) > 2 else "ok"
    note = c[3] if len(c) > 3 else ""
    try:
        if tool == "bus_setup":
            setup(run_dir, args)
            got, answer = "ok", "set up"
        elif tool == "bus":
            do = args.pop("do")
            getattr(Bus(run_dir), do.replace("-", "_"))(**args)
            got, answer = "ok", do
        elif tool == "bus_expect":
            ok, had = expect(run_dir, args)
            got, answer = ("ok" if ok else "error"), json.dumps(had)
        else:
            raise ValueError(f"no bus call {tool}")
    except (KeyError, TypeError, ValueError, OSError) as e:
        got, answer = "error", str(e)
    return {"tool": tool, "args": c[1], "want": want, "got": got, "wrong": got != want, "note": note,
            "answer": answer[:400]}


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    bus, what, rest = Bus(sys.argv[1]), sys.argv[2], sys.argv[3:]
    opts = {}
    args = []
    i = 0
    while i < len(rest):
        if rest[i].startswith("--"):
            key = {"peak": "truePeakDbtp"}.get(rest[i][2:], rest[i][2:])
            v = rest[i + 1]
            try:
                v = json.loads(v)
            except ValueError:
                pass
            opts[key] = v
            i += 2
        else:
            args.append(rest[i])
            i += 1
    if what == "show":
        print(json.dumps(bus.read(), indent=1))
    elif what == "levels":
        t = bus.last()
        if not t:
            print("nothing has played")
            return
        for s in t["streams"]:
            print(f"{s['app']} {s['id']} -> {s['output'] or 'nowhere'}: {'plays' if s['playing'] else 'paused'} "
                  f"{s['rmsDbfs']} dBFS, reaches {', '.join(s['reaches']) or 'no DSPi'}")
        for x in t["inputs"]:
            print(f"  {x['device']} in {x['input'] + 1}: {x['rmsDbfs']} dBFS RMS, {x['peakDbfs']} peak"
                  f"{' CLIP' if x['clip'] else ''}")
        for x in t["outputs"]:
            print(f"  {x['device']} out {x['output'] + 1} ({x['driver']}): {x['rmsDbfs']} dBFS RMS, "
                  f"{x['peakDbfs']} peak, limiter {x['limiterDb']} dB, {x['splDb']} dB SPL{' CLIP' if x['clip'] else ''}")
        print(f"  seat: {t['seatSplDb']} dB SPL")
    elif what == "play":
        bus.play(args[0], opts.pop("app"), opts.pop("output"), **opts)
    elif what in ("pause", "resume", "stop"):
        getattr(bus, what)(args[0])
    elif what == "move":
        bus.move(args[0], args[1])
    elif what == "volume":
        bus.volume(args[0], float(args[1]))
    elif what == "output-volume":
        bus.output_volume(args[0], float(args[1]))
    elif what == "default":
        bus.default(args[0])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
