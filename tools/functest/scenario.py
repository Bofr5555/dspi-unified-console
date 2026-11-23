#!/usr/bin/env python3
"""The function test's home theatre: writes scenario.json (the one data file the
simulator, the harness and the judge read). Change the system here.

    python3 tools/functest/scenario.py > tools/functest/scenario.json
    python3 tools/functest/scenario.py restaurant > tools/functest/scenario-restaurant.json   (-- WP FUNC2c --)
    python3 tools/functest/scenario.py centre-rewired > tools/functest/scenario-centre-rewired.json

A story names a variant with `scenario = "restaurant"` (scenario-restaurant.json).
"""
import json

SOUND = 0.343  # m per ms


def peak(freq, q, gain):
    return {"kind": "peak", "freq": freq, "q": q, "gain_db": gain}


def driver(sens, hp, hpq, ro=0.0, roq=0.7, breakup=None, offset=0.0, inverted=False):
    return {"sensitivityDb": sens, "highpassHz": hp, "highpassQ": hpq, "rolloffHz": ro, "rolloffQ": roq,
            "breakup": breakup, "offsetMs": offset, "inverted": inverted}


# The oracle's limits for a test signal, dBFS at the amplifier
TWEETER = {"belowHz": 1000.0, "belowDbfs": -50.0, "capDbfs": -12.0}
MID = {"belowHz": 60.0, "belowDbfs": -45.0, "capDbfs": -6.0}
WOOFER = {"belowHz": 0.0, "belowDbfs": 0.0, "capDbfs": -6.0}


def out(n, name, role, speaker, group, drv, filters, metres, safe, gain=0.0):
    return {"output": n - 1, "name": name, "role": role, "speaker": speaker, "group": group, "driver": drv,
            "filters": filters, "delayMs": round(metres / SOUND, 3), "gainDb": gain, "safe": safe}


def two_way(name, group, first, metres, w_sens, t_sens, modes, w_inv=False, t_inv=False, f_res=1400.0):
    """A 2-way speaker on `first` (woofer) and `first`+1 (tweeter)."""
    return [
        out(first, f"{name} woofer", "low", name, group,
            driver(w_sens, 46.0, 0.75, 3600.0, 0.7, peak(4300.0, 5.0, 7.0), 0.12, w_inv),
            modes + [peak(780.0, 1.6, 2.5), peak(1900.0, 2.5, -2.0)], metres, WOOFER),
        out(first + 1, f"{name} tweeter", "high", name, group,
            driver(t_sens, f_res, 0.9, 0.0, 0.7, peak(23000.0, 3.0, 4.0), 0.0, t_inv),
            [peak(3300.0, 2.0, 2.5), peak(9500.0, 3.0, -2.5)], metres, TWEETER),
    ]


INPUTS = ["L", "R", "C", "LFE", "Ls", "Rs", "In 7", "In 8"]

scenario = {
    "name": "FUNC1 home theatre",
    "seed": 20261122,
    "noiseDbfs": -92.0,
    "micPpm": 32.0,
    "latencyMs": 50.0,
    "latencyJitterMs": 3.0,
    # The room, for everything in it at the prime seat
    "modes": [
        {"freq": 37.0, "gainDb": 7.0, "t60S": 0.7},
        {"freq": 61.0, "gainDb": -6.0, "t60S": 0.45},
        {"freq": 88.0, "gainDb": 5.0, "t60S": 0.4},
        {"freq": 128.0, "gainDb": 3.0, "t60S": 0.3},
    ],
    "reflections": [{"delayMs": 2.4, "gain": 0.28}, {"delayMs": 5.9, "gain": 0.18}],
    "devices": [
        {
            "serial": "FUNC1-FRONTS", "name": "Fronts", "userVolumeDb": -20.0, "inputNames": INPUTS, "routes": [],
            "outputs":
                two_way("Left", 0, 1, 3.00, -13.0, -8.5, [peak(112.0, 4.0, 2.5)])
                + two_way("Right", 1, 3, 3.18, -14.2, -9.5, [peak(146.0, 4.0, -3.0)]),
        },
        {
            "serial": "FUNC1-CENTRE", "name": "Centre", "userVolumeDb": -20.0, "inputNames": INPUTS, "routes": [],
            "outputs": [
                out(1, "Centre woofer", "low", "Centre", 0,
                    driver(-12.0, 55.0, 0.8, 1800.0, 0.7, peak(3200.0, 4.0, 6.0), 0.18),
                    [peak(165.0, 3.0, 4.0)], 2.75, WOOFER),
                # The tweeter is on output 2 on every device
                out(2, "Centre tweeter", "high", "Centre", 0,
                    driver(-7.5, 1500.0, 0.95, 0.0, 0.7, peak(22000.0, 3.0, 3.0), 0.0),
                    [peak(4200.0, 2.0, 2.0)], 2.75, TWEETER),
                # Wired the wrong way round
                out(3, "Centre midrange", "mid", "Centre", 0,
                    driver(-9.0, 200.0, 0.7, 6200.0, 0.7, peak(7600.0, 4.0, 5.0), 0.06, True),
                    [peak(1100.0, 1.5, 3.0), peak(2600.0, 2.5, -2.5)], 2.75, MID),
                out(5, "Subwoofer", "sub", "Sub", 1,
                    driver(-10.0, 25.0, 0.7, 260.0, 0.7, None, 0.5),
                    [peak(32.0, 3.0, 4.0), peak(52.0, 4.0, 5.0)], 3.60, WOOFER),
            ],
        },
        {
            "serial": "FUNC1-SURROUNDS", "name": "Surrounds", "userVolumeDb": -20.0, "inputNames": INPUTS, "routes": [],
            "outputs":
                two_way("Surround L", 0, 1, 1.90, -15.0, -11.0, [peak(98.0, 4.0, 3.0)], f_res=1300.0)
                + two_way("Surround R", 1, 3, 2.35, -11.5, -8.0, [peak(133.0, 4.0, 2.5)], f_res=1300.0),
        },
    ],
}

# -- WP FUNC2c -- FSTORY S7: one DSPi in a restaurant, two 2-way speakers and a sub, blank like the
# home theatre's (seeds/restaurant.json crossovers and routes it as the user did: stereo in on inputs
# 1-2, the sub fed from both). The installer's profile, PIN and lock are the story's (user.py).
restaurant = {
    **{k: v for k, v in scenario.items() if k != "devices"},
    "name": "FUNC2c restaurant",
    "devices": [
        {
            "serial": "FUNC2C-RESTAURANT", "name": "Restaurant", "userVolumeDb": -20.0,
            "inputNames": ["L", "R", "In 3", "In 4", "In 5", "In 6", "In 7", "In 8"],
            "routes": [],
            "outputs":
                two_way("Left", 0, 1, 3.40, -13.0, -8.5, [peak(112.0, 4.0, 2.5)])
                + two_way("Right", 1, 3, 3.70, -13.5, -9.0, [peak(146.0, 4.0, -3.0)])
                + [out(5, "Subwoofer", "sub", "Sub", 2,
                       driver(-6.0, 25.0, 0.7, 260.0, 0.7, None, 0.5),
                       [peak(45.0, 3.0, 5.0), peak(70.0, 4.0, 4.0)], 4.20, WOOFER)],
        },
    ],
}


# -- WP FUNC2c -- FSTORY S6: the home theatre with the centre rewired, as the user says: woofer on
# 1, midrange on 4, tweeter on 5 (a soft dome still strong at 1 kHz, so its roll-off search stops at
# the floor: "probably a midrange set up as a tweeter"), the sub moved to 6. The speaker setup the
# story seeds is the old wiring, unconfirmed (s6-trust-me.toml).
def centre_rewired():
    devices = [dict(d) for d in scenario["devices"]]
    devices[1] = {
        **devices[1],
        "outputs": [
            out(1, "Centre woofer", "low", "Centre", 0,
                driver(-12.0, 55.0, 0.8, 1800.0, 0.7, peak(3200.0, 4.0, 6.0), 0.18),
                [peak(165.0, 3.0, 4.0)], 2.75, WOOFER),
            out(4, "Centre midrange", "mid", "Centre", 0,
                driver(-9.0, 200.0, 0.7, 6200.0, 0.7, peak(7600.0, 4.0, 5.0), 0.06),
                [peak(1100.0, 1.5, 3.0), peak(2600.0, 2.5, -2.5)], 2.75, MID),
            out(5, "Centre tweeter", "high", "Centre", 0,
                driver(-8.0, 650.0, 0.8, 0.0, 0.7, peak(21000.0, 3.0, 3.0), 0.0),
                [peak(4200.0, 2.0, 2.0)], 2.75, TWEETER),
            out(6, "Subwoofer", "sub", "Sub", 1,
                driver(-10.0, 25.0, 0.7, 260.0, 0.7, None, 0.5),
                [peak(32.0, 3.0, 4.0), peak(52.0, 4.0, 5.0)], 3.60, WOOFER),
        ],
    }
    return {**scenario, "name": "FUNC2c home theatre, centre rewired", "devices": devices}


VARIANTS = {"restaurant": lambda: restaurant, "centre-rewired": centre_rewired}

if __name__ == "__main__":
    import sys
    print(json.dumps(VARIANTS[sys.argv[1]]() if len(sys.argv) > 1 else scenario, indent=1))
