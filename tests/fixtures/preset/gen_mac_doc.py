#!/usr/bin/env python3
"""Build mac-swift-v32.dspipreset strictly from the Swift reference.

No document written by a real Mac was available, so this follows the macOS
Console's `PresetDocument.capture(from:)` and `PresetDocumentFile.encode`
(`DSPi Console/PresetDocumentTransfer.swift`, `PresetDocument.swift`, branch
ext-channel-116 at 48cf7c5) line by line, over the device state in
`../device/dspstate_v32.json`, with a tuning set on top so that applying the
document has to move something. Run from this directory, no arguments.
"""
import decimal, json, re, struct

s = json.load(open("../device/dspstate_v32.json"))
MAX_MATRIX_INPUTS = 8  # Constants.swift:812
BASE_MATRIX_INPUTS = 2  # Constants.swift:813
SPDIF_RX_NUM_INPUTS = 4  # Constants.swift:901

# A tuning, as if the user had set it before exporting. Inputs and outputs get
# values that differ per channel, so a misplaced channel shows.
for w in range(s["numChannels"]):
    s["filters"][w][0] = dict(s["filters"][w][0], filterType=1, freq=100.0 + w, q=1.25, gain=3.0)
    s["filters"][w][3] = dict(s["filters"][w][3], filterType=2, freq=120.5 + w, q=0.5, gain=-2.5, bypass=True)
    s["channelDelays"][w] = 0.5 * (w % 3)
pdm = s["numChannels"] - 1
s["xover"][pdm][0] = dict(s["xover"][pdm][0], filterType=34, freq=80.0)
s["inputPreampDb"][0] = s["inputPreampDb"][1] = -3.0
s["outputGainDb"][0] = -1.5
s["outputGainDb"][8] = -4.0
s["outputDelayMs"][1] = 1.5
# An output's delay is also its channel's delay: the firmware keeps the two equal
# (vendor_commands.c, REQ_SET_OUTPUT_DELAY), so a real export always shows that.
for o in range(s["numOutputChannels"]):
    s["channelDelays"][s["numInputChannels"] + o] = s["outputDelayMs"][o]
s["limiterEnabled"][0] = True
s["limiterThresholdDb"][0] = -6.0
s["limiterReleaseMs"][0] = 120.0
s["matrixRouting"][2][3] = True
s["matrixGain"][2][3] = -6.0
s["matrixInvert"][2][3] = True
s["bypass"] = False
s["masterVolumeDb"] = -18.0
s["userVolumeDb"] = -12.0
s["loudnessEnabled"] = True
s["loudnessRefSpl"] = 80.0
s["loudnessIntensity"] = 60.0
s["loudnessOutputMask"] = 3
s["crossfeedEnabled"] = True
s["crossfeedPreset"] = 1
s["crossfeedOutputPairMask"] = 3
s["levellerSpeed"] = 1
s["levellerAmount"] = 25.0
s["psybassCutoffHz"] = 90.0
s["psybassOutputMask"] = 0x100
# The K340's map: a device on the fork, so `crossfeedPairMapSupported`.
PAIR_MAP = [0x02, 0x13, 0, 0]


def fl(x):
    """Swift prints a Float's shortest digits, and whole values without `.0`."""
    x32 = struct.unpack("f", struct.pack("f", float(x)))[0]
    for p in range(1, 10):
        t = "%.*g" % (p, x32)
        if struct.unpack("f", struct.pack("f", float(t)))[0] == x32:
            break
    t = format(decimal.Decimal(t), "f")  # Below 2^24, Swift never uses an exponent.
    return "@F" + t + "@"


def band(b):  # BandBlock.init(_ p: FilterParams)
    return {"type": b["filterType"], "freqHz": fl(b["freq"]), "q": fl(b["q"]), "gain": fl(b["gain"]),
            "qp": fl(b["qp"]), "bypass": b["bypass"]}


n_in, n_out = s["numInputChannels"], s["numOutputChannels"]
channels = []
for i in range(n_in):  # channelBlock, .input
    channels.append({
        "channelId": i if i < BASE_MATRIX_INPUTS else 11 + (i - BASE_MATRIX_INPUTS),
        "inputIndex": i, "isOutput": False, "eqChannel": i, "name": s["channelNames"][i],
        "delayMs": fl(s["channelDelays"][i]), "gainDb": fl(0), "muted": False, "enabled": True,
        "eq": [band(b) for b in s["filters"][i]], "crossover": [],
    })
for o in range(n_out):  # channelBlock, .output
    w = n_in + o
    channels.append({
        "channelId": 10 if o == n_out - 1 else 2 + o,  # PresetChannelID, RP2350
        "outputIndex": o, "isOutput": True, "eqChannel": w, "name": s["channelNames"][w],
        "gainDb": fl(s["outputGainDb"][o]), "muted": s["outputMuted"][o], "enabled": s["outputEnabled"][o],
        "outputDelayMs": fl(s["outputDelayMs"][o]),
        "limiter": {"enabled": s["limiterEnabled"][o], "thresholdDb": fl(s["limiterThresholdDb"][o]),
                    "releaseMs": fl(s["limiterReleaseMs"][o]), "linkGroup": s["limiterLinkGroup"][o]},
        "crossover": [band(b) for b in s["xover"][w]],
        "delayMs": fl(s["channelDelays"][w]), "eq": [band(b) for b in s["filters"][w]],
    })

matrix = []
for i in range(MAX_MATRIX_INPUTS):
    for o in range(n_out):
        matrix.append({"input": i, "output": o, "enabled": s["matrixRouting"][i][o],
                       "invert": s["matrixInvert"][i][o], "gainDb": fl(s["matrixGain"][i][o])})

u, sh, t = s["upmix"], s["subharm"], s["tube"]
# The firmware's parameter order: dspi_proto's UPMIX_PARAM_* and SUBHARM_PARAM_*,
# which `dspstate_v32.json` holds its `upmix` and `subharm` tables in.
UP = dict(enabled=0, center=1, surround=2, strength=3, width=4, threshold=5, attack=6, release=7,
          det_hpf=8, delay=9, hpf=10, lpf=11, decorr=12, presence=13)
SH = dict(enabled=0, mask=1, low=2, high=3, top=4, boost=5, select=6, depth=7, hold=8, ceiling=9, link=10)
doc = {
    "schemaVersion": 1,
    "meta": {
        "name": "Mac reference", "savedUtc": "2026-11-06T10:00:00Z", "appVersion": "1.0",
        "platform": "RP2350", "firmwareVersion": "1.1.6 beta 5",
        "wireFormatVersion": s["formatVersion"], "inputChannelCount": n_in, "outputChannelCount": n_out,
        "masterVolumeMode": s["masterVolumeMode"], "outputConfigMode": s["outputConfigMode"],
    },
    "global": {
        "inputPreampsDb": [fl(x) for x in s["inputPreampDb"]], "bypass": s["bypass"],
        "masterVolumeDb": fl(s["masterVolumeDb"]), "userVolumeDb": fl(s["userVolumeDb"]),
        "inputSource": s["inputSource"], "lgSoundSyncEnabled": s["lgSoundSyncEnabled"],
        "inputPairLinked": [True, False, False, False],  # DSPViewModel.inputPairCount = 4
    },
    "loudness": {"enabled": s["loudnessEnabled"], "refSpl": fl(s["loudnessRefSpl"]),
                 "intensityPct": fl(s["loudnessIntensity"]), "outputMask": s["loudnessOutputMask"]},
    "crossfeed": {"enabled": s["crossfeedEnabled"], "preset": s["crossfeedPreset"], "freqHz": fl(s["crossfeedFreq"]),
                  "feedDb": fl(s["crossfeedFeed"]), "itd": s["crossfeedItd"],
                  "outputPairMask": s["crossfeedOutputPairMask"], "pairMap": PAIR_MAP},
    "leveller": {"enabled": s["levellerEnabled"], "speed": s["levellerSpeed"], "lookahead": s["levellerLookahead"],
                 "amountPct": fl(s["levellerAmount"]), "maxGainDb": fl(s["levellerMaxGainDb"]),
                 "gateDb": fl(s["levellerGateDb"]), "detectorMask": s["levellerDetectorMask"],
                 "applyMask": s["levellerApplyMask"]},
    "psybass": {"enabled": s["psybassEnabled"], "cutoffHz": fl(s["psybassCutoffHz"]),
                "harmonicsDb": fl(s["psybassHarmonicsDb"]), "driveDb": fl(s["psybassDriveDb"]),
                "characterPct": fl(s["psybassCharacterPct"]), "originalDb": fl(s["psybassOriginalDb"]),
                "outputMask": s["psybassOutputMask"]},
    "upmix": {"enabled": u[UP["enabled"]] != 0, "centerMode": int(u[UP["center"]]),
              "surroundMode": int(u[UP["surround"]]), "strengthPct": fl(u[UP["strength"]]),
              "centerWidthPct": fl(u[UP["width"]]), "thresholdPct": fl(u[UP["threshold"]]),
              "attackMs": fl(u[UP["attack"]]), "releaseMs": fl(u[UP["release"]]),
              "detectorHpfHz": fl(u[UP["det_hpf"]]), "surroundDelayMs": fl(u[UP["delay"]]),
              "surroundHpfHz": fl(u[UP["hpf"]]), "surroundLpfHz": fl(u[UP["lpf"]]),
              "decorrPct": fl(u[UP["decorr"]]), "presenceDb": fl(u[UP["presence"]])},
    "subharm": {"enabled": sh[SH["enabled"]] != 0, "lowDb": fl(sh[SH["low"]]), "highDb": fl(sh[SH["high"]]),
                "boostDb": fl(sh[SH["boost"]]), "outputMask": int(sh[SH["mask"]]), "topDb": fl(sh[SH["top"]]),
                "selectMode": int(sh[SH["select"]]), "selectDepthPct": fl(sh[SH["depth"]]),
                "selectHoldMs": fl(sh[SH["hold"]]), "ceilingDb": fl(sh[SH["ceiling"]]),
                "linkPairs": sh[SH["link"]] != 0},
    "tube": {"enabled": t[0] != 0, "outputMask": int(t[1]), "tubeType": int(t[2]), "driveDb": fl(t[3]),
             "biasPct": fl(t[4]), "asymDb": fl(t[5]), "hardnessPct": fl(t[6]), "sagPct": fl(t[7]),
             "rectifier": int(t[8]), "xfmrEnabled": t[9] != 0, "xfmrDamping": fl(t[10]), "xfmrResHz": fl(t[11]),
             "mixPct": fl(t[12]), "trimDb": fl(t[13])},
    "channels": channels,
    "matrix": matrix,
    "io": {  # ioBlock
        "outputPins": s["outputPins"][:s["numPinOutputs"]], "outputSlotTypes": s["outputTypes"],
        "i2sBckPin": s["i2sBckPin"], "mckEnabled": s["mckEnabled"], "mckPin": s["mckPin"],
        "mckMultiplier": 256 if s["mckMultiplier"] == 1 else 128, "i2sClockMode": s["i2sClockMode"],
        "i2sClockPinMode": s["i2sClockPinMode"], "i2sBckPinSlave": s["i2sBckPinSlave"],
        "spdifRxPins": s["spdifRxPins"][:3], "spdifRxPin4": s["spdifRxPins"][3],
        "spdifEnabledExt": sum(1 << (i - 1) for i in range(1, SPDIF_RX_NUM_INPUTS)
                               if s["spdifInputsEnabled"] & (1 << i)),
        "i2sRxPins": s["i2sRxPins"], "i2sInputChannels": s["i2sInputChannels"],
        "i2sInputRateHz": [44100, 48000, 96000][s["i2sInputRate"]],
        "adatEnabled": s["adatOutEnabled"], "adatPin": s["adatOutPin"], "adatInputEnabled": s["adatInputEnabled"],
        "adatInputPin": s["adatInputPin"], "adatInputClockMode": s["adatInputClockMode"],
        # `dacHwMute` only when `dacHwMuteSupported`; synthesized Codable omits a nil.
    },
}

# JSONEncoder with [.prettyPrinted, .sortedKeys]: two spaces, ` : `, `\/`, an
# empty array across lines, and numbers bare.
text = json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False, separators=(",", " : "))
text = re.sub(r'"@F([^@]*)@"', r"\1", text)
text = text.replace("/", "\\/")
text = re.sub(r"^( *)(.*)\[\]", lambda m: f"{m[1]}{m[2]}[\n\n{m[1]}]", text, flags=re.M)
open("mac-swift-v32.dspipreset", "w").write(text)
