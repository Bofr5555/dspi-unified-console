#!/usr/bin/env python3
"""WP LK3: tables from the `RESULT ...` lines of tools/esp-rig/scenarios.sh.

    python3 -I tools/esp-rig/summary.py LOG [LOG...]

One row per (scenario, baud, shaping, client, clients), with every run's value
in the cell, in run order, so the spread shows.
"""
import re
import sys
from collections import OrderedDict

FIELDS = {
    "rust": [
        ("first meter s", "first_meter"),
        ("usable s", "usable"),
        ("after select s", "after_select"),
        ("select s", "select"),
        ("reads done s", "reads_done"),
        ("freezes", "freezes"),
        ("longest gap s", "longest_gap_s"),
        ("drops", "drops"),
        ("re-read max s", "reread_max_s"),
        ("re-read failed", "reread_failed"),
        ("bridge 10 s closes", "unwritable_closes"),
        ("queue-full reports", "queue_drop_reports"),
        ("failed", "failed"),
    ],
    "ts": [
        ("first meter s", "first_meter"),
        ("usable s", "usable"),
        ("freezes", "freezes"),
        ("longest gap ms", "longest_gap_ms"),
        ("closes", "closes"),
        ("re-read max ms", "reread_max_ms"),
        ("re-read failed", "reread_failed"),
        ("bridge 10 s closes", "unwritable_closes"),
        ("queue-full reports", "queue_drop_reports"),
        ("errors", "errors"),
    ],
}


def parse(line):
    out = {}
    for m in re.finditer(r"(\w+)=(\[[^\]]*\]|\S+)", line):
        out[m.group(1)] = m.group(2)
    # The shaping may hold spaces (netem's own words) or underscores.
    m = re.search(r"shape=(.*?) (?:rep|who|clients)=", line)
    if m:
        out["shape"] = m.group(1)
    return out


def main(paths):
    rows = OrderedDict()
    for path in paths:
        with open(path, encoding="utf-8") as f:
            for line in f:
                if not line.startswith("RESULT "):
                    continue
                r = parse(line[len("RESULT "):])
                key = (
                    r.get("scenario", "?"),
                    r.get("baud", "?"),
                    r.get("shape", "?").replace("_", " "),
                    r.get("who", "?"),
                    r.get("clients", "1"),
                )
                rows.setdefault(key, []).append(r)
    by_kind = OrderedDict()
    for key, runs in rows.items():
        kind = "ts" if key[3] == "ts" else "rust"
        by_kind.setdefault((key[0], kind), []).append((key, runs))
    for (scenario, kind), entries in by_kind.items():
        cols = FIELDS[kind]
        print(f"\n### {scenario}, {'TS client' if kind == 'ts' else 'our client'}\n")
        print("| baud | shaping | client | n | " + " | ".join(c for c, _ in cols) + " |")
        print("|---" * (4 + len(cols)) + "|")
        for key, runs in entries:
            cells = []
            for _, field in cols:
                vals = [r.get(field, "-") for r in runs]
                cells.append(", ".join(vals))
            print(f"| {key[1]} | {key[2]} | {key[3]} | {key[4]} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main(sys.argv[1:])
