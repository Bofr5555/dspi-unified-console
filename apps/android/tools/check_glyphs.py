#!/usr/bin/env python3
"""WP P1EFIX2: no emoji or pictographic glyphs in the Android app's strings.

Owner rule (2026-11-14): the app's text uses words, plain typographic
characters (dashes, arrows used as text, the degree sign, check marks, simple
geometric shapes such as a dropdown's triangle) and the platform's own
widgets, never emoji. "Pictographic" is Unicode's Extended_Pictographic
property (emoji-data.txt), less the typographic characters it also covers
(the copyright, registered and trade mark signs, the information sign, the
double exclamation marks and the arrows), plus the emoji presentation
selector and the zero-width joiner of emoji sequences.

Scans every main source set (Kotlin and res/ XML) under apps/android; the
tests may name a glyph to prove it is gone. Exit 1 with each hit listed.

    python3 tools/check_glyphs.py        (from apps/android; run by the
                                          android-contract step)
"""
import os
import sys

# Extended_Pictographic in the Basic Multilingual Plane (Unicode 15.1
# emoji-data.txt), the typographic exceptions above left out.
BMP = [
    (0x231A, 0x231B), (0x2328, 0x2328), (0x2388, 0x2388), (0x23CF, 0x23CF),
    (0x23E9, 0x23F3), (0x23F8, 0x23FA), (0x24C2, 0x24C2), (0x25AA, 0x25AB),
    (0x25B6, 0x25B6), (0x25C0, 0x25C0), (0x25FB, 0x25FE), (0x2600, 0x2605),
    (0x2607, 0x2612), (0x2614, 0x2685), (0x2690, 0x2705), (0x2708, 0x2712),
    (0x2714, 0x2714), (0x2716, 0x2716), (0x271D, 0x271D), (0x2721, 0x2721),
    (0x2728, 0x2728), (0x2733, 0x2734), (0x2744, 0x2744), (0x2747, 0x2747),
    (0x274C, 0x274C), (0x274E, 0x274E), (0x2753, 0x2755), (0x2757, 0x2757),
    (0x2763, 0x2767), (0x2795, 0x2797), (0x27B0, 0x27B0), (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C), (0x2B50, 0x2B50), (0x2B55, 0x2B55), (0x3030, 0x3030),
    (0x303D, 0x303D), (0x3297, 0x3297), (0x3299, 0x3299),
]
# Every emoji plane block, the presentation selector and the joiner.
OTHER = [(0x1F000, 0x1FAFF), (0x1FC00, 0x1FFFD), (0xFE0F, 0xFE0F), (0x200D, 0x200D)]
RANGES = BMP + OTHER


def banned(c: str) -> bool:
    o = ord(c)
    return any(a <= o <= b for a, b in RANGES)


def main() -> int:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    hits = []
    for dirpath, dirnames, files in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in ("build", "test", "androidTest", ".gradle")]
        rel = os.path.relpath(dirpath, root)
        if os.sep + "src" + os.sep + "main" not in os.sep + rel + os.sep and "/src/main" not in "/" + rel:
            continue
        for name in files:
            is_res = name.endswith(".xml") and os.sep + "res" + os.sep in os.sep + rel + os.sep
            if not (name.endswith(".kt") or is_res):
                continue
            path = os.path.join(dirpath, name)
            with open(path, encoding="utf-8", errors="replace") as f:
                for n, line in enumerate(f, 1):
                    bad = sorted({c for c in line if banned(c)})
                    if bad:
                        codes = " ".join(f"U+{ord(c):04X}" for c in bad)
                        hits.append(f"{os.path.relpath(path, root)}:{n}: {codes}")
    if hits:
        print("check_glyphs: emoji or pictographic glyphs in the app's strings (use words or plain characters):")
        print("\n".join("  " + h for h in hits))
        return 1
    print("check_glyphs: no emoji or pictographic glyphs in the app's strings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
