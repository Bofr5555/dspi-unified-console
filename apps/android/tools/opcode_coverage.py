#!/usr/bin/env python3
"""Opcode coverage check (PLAN-NATIVE-ANDROID.md §4 "Command catalog", §7 step 2).

Three layers must agree on the vendor requests (REQ_* opcodes):

  firmware  firmware/DSPi/config.h at the commits pinned in tools/REFERENCE.toml
            [firmware] (tag_sha and branch_head_sha). CI has no access to the
            firmware repository, so the `#define REQ_*` lines are kept in
            fixtures/firmware_req_opcodes.txt, extracted read-only with
            `git show <sha>:<config_h>` by `--refresh-firmware <clone>`.
  core      `#define REQ_*` of crates/dspi-proto/firmware/config.h, the vendored
            header dspi-proto generates its opcode constants from.
  ffi       the opcodes the core's code reaches from what dspi-core exports
            through UniFFI, the surface tools/gen_kotlin_contract.py reads (a
            name-based call graph over the non-test items of dspi-core,
            dspi-session, dspi-proto and dspi-transport: fns, consts and statics;
            worker threads are reached through the calls that start them).

Limitation of the call graph: it works on names, not types. Items with the
same name in different impls (`parse`, `default`, `new`, `start`, ...) are
merged into one node, and any identifier in a body that names an item is an
edge. It therefore over-approximates reachability: it reliably catches an
opcode whose only path from the exported surface was removed, but "reachable" does not
prove a specific path exists (e.g. RTA 0x08-0x0F show up as reachable from
several roots). `-v` lists the merged names and the exported functions reaching each opcode.

Policy (coordinator decision D6): the TAG pin (tag_sha, the firmware the
reference UI ships) is the failing check; the release-branch head
(branch_head_sha) is reported as non-failing WARNINGs.

Fails (exit 1) on:
  - a firmware opcode at the tag pin missing from the core, or a value
    mismatch, unless listed in FIRMWARE_NOT_IN_CORE;
  - an opcode the core uses that the exported surface cannot reach, unless
    listed in UNREACHABLE_FROM_FFI;
  - a firmware snapshot without the pinned shas, or a stale allowlist entry
    (the gap it excuses is gone).
Opcodes the core defines but never uses are reported, not failed: the bindings
cannot expose what the core does not implement.

Usage:
  python3 tools/opcode_coverage.py                       # the CI check
  python3 tools/opcode_coverage.py -v                    # also list the mapping
  python3 tools/opcode_coverage.py --refresh-firmware ~/prog/DSPi   # host only
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from collections import defaultdict, deque
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOP = ROOT.parent.parent
PROTOCOL = TOP / "crates/dspi-proto/firmware/config.h"
# dspi-transport's fake and mock devices answer opcodes rather than send them.
CORE_FILES = [
    f
    for crate in ("dspi-core", "dspi-session", "dspi-proto", "dspi-transport")
    for f in sorted((TOP / "crates" / crate / "src").glob("*.rs"))
    if f.name not in ("fake.rs", "mock.rs")
]
REFERENCE = ROOT / "tools/REFERENCE.toml"
SNAPSHOT = ROOT / "fixtures/firmware_req_opcodes.txt"

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from gen_kotlin_contract import parse_surface  # noqa: E402

# Firmware opcodes the core does not define. Each entry must still be
# a gap (the check fails when the core catches up, so the entry gets removed).
FIRMWARE_NOT_IN_CORE = {
    n: "0x54-0x57: channel gain and mute at the pins; release/v1.1.6 final reuses them for input-source and device names "
    "(what the core vendors), and the core sends those only when the device reports the feature"
    for n in ("REQ_SET_CHANNEL_GAIN", "REQ_GET_CHANNEL_GAIN", "REQ_SET_CHANNEL_MUTE", "REQ_GET_CHANNEL_MUTE")
}

# Opcodes the core uses that the exported surface deliberately does not reach.
UNREACHABLE_FROM_FFI: dict[str, str] = {}


# ── firmware ─────────────────────────────────────────────────────────────

FW_RX = re.compile(r"^\s*#define\s+(REQ_[A-Z0-9_]+)\s+(0x[0-9A-Fa-f]+|\d+)", re.M)


def reference_firmware() -> dict:
    return tomllib.loads(REFERENCE.read_text())["firmware"]


def pinned_shas(fw: dict) -> list[tuple[str, str]]:
    return [("tag_sha", fw["tag_sha"]), ("branch_head_sha", fw["branch_head_sha"])]


def refresh_firmware(clone: Path) -> int:
    fw = reference_firmware()
    out = [
        "# `#define REQ_*` of firmware/DSPi/config.h at the tools/REFERENCE.toml [firmware] pins.",
        f"# Source: {fw['repo']} (extracted read-only: git show <sha>:{fw['config_h']}).",
        "# Regenerate: python3 tools/opcode_coverage.py --refresh-firmware <firmware clone>",
    ]
    for key, sha in pinned_shas(fw):
        text = subprocess.run(
            ["git", "-C", str(clone), "show", f"{sha}:{fw['config_h']}"],
            check=True, capture_output=True, text=True,
        ).stdout
        out.append(f"[{key} {sha}]")
        out += [f"{name} 0x{int(val, 0):02X}" for name, val in FW_RX.findall(text)]
    SNAPSHOT.write_text("\n".join(out) + "\n")
    print(f"opcode_coverage: wrote {SNAPSHOT.relative_to(ROOT)}")
    return 0


def firmware_snapshot() -> dict[str, dict[str, int]]:
    """{sha: {name: value}} from the snapshot file."""
    sections: dict[str, dict[str, int]] = {}
    cur = None
    for ln in SNAPSHOT.read_text().splitlines():
        if not ln or ln.startswith("#"):
            continue
        if m := re.match(r"^\[(\w+) ([0-9a-f]{40})\]$", ln):
            cur = sections.setdefault(m.group(2), {})
        elif cur is not None:
            name, val = ln.split()
            cur[name] = int(val, 0)
    return sections


# ── core ─────────────────────────────────────────────────────────────────

def core_opcodes() -> dict[str, int]:
    return {n: int(v, 0) for n, v in FW_RX.findall(PROTOCOL.read_text())}


def core_aliases(core: dict[str, int]) -> dict[str, str]:
    """{name used in core code: protocol.rs name}. Some modules declare private
    aliases (`const REQ_SET_GROUP: u8 = 0x20;` for REQ_SET_CS_GROUP)."""
    by_value = defaultdict(list)
    for n, v in core.items():
        by_value[v].append(n)
    rx = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?const (REQ_[A-Z0-9_]+): u8 = (0x[0-9A-Fa-f]+|\d+);", re.M)
    alias = {n: n for n in core}
    for f in CORE_FILES:
        for n, v in rx.findall(f.read_text()):
            canon = by_value.get(int(v, 0), [])
            if n not in core and len(canon) == 1:
                alias[n] = canon[0]
    return alias


def strip(text: str) -> str:
    """Drop comments, string/char literal contents and test modules."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"//[^\n]*", "", text)
    text = re.sub(r'"(?:\\.|[^"\\])*"', '""', text)
    text = re.sub(r"'(?:\\.|[^'\\])'", "' '", text)
    rx = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*mod\s+\w+\s*([;{])")
    while m := rx.search(text):
        end = m.end() if m.group(1) == ";" else block_extent(text, m.end() - 1)
        text = text[:m.start()] + text[end:]
    return text


def block_extent(text: str, open_brace: int) -> int:
    """The offset just past the `}` matching the `{` at `open_brace`."""
    depth = 0
    for i in range(open_brace, len(text)):
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        if depth == 0:
            return i + 1
    return len(text)


def item_extent(text: str, start: int, kind: str) -> int:
    """End offset of the item starting at `start` (fn: its body; const/static: up to `;`)."""
    depth = 0
    i = start
    if kind == "fn":
        # Signature: up to the body's `{` or a declaration's `;`, skipping
        # brackets (`-> [u8; 24]`).
        while i < len(text) and not (depth == 0 and text[i] in "{;"):
            depth += text[i] in "([" and 1 or text[i] in ")]" and -1 or 0
            i += 1
        if i < len(text) and text[i] == ";":
            return i + 1  # trait method declaration
    while i < len(text):
        c = text[i]
        if c in "{[(":
            depth += 1
        elif c in "}])":
            depth -= 1
            if depth == 0 and kind == "fn":
                return i + 1
        elif c == ";" and depth == 0 and kind != "fn":
            return i + 1
        i += 1
    return len(text)


ITEM_RX = re.compile(r"(?<![*'])\b(fn|const|static)\s+([A-Za-z_]\w*)")  # not `*const T`, `'static`


def core_items() -> dict[str, list[str]]:
    """{item name: [bodies]} (names collide across impls; bodies are merged)."""
    items: dict[str, list[str]] = defaultdict(list)
    for f in CORE_FILES:
        text = strip(f.read_text())
        for m in ITEM_RX.finditer(text):
            kind, name = m.groups()
            if name.startswith("REQ_"):
                continue
            items[name].append(text[m.end():item_extent(text, m.end(), kind)])
    return items


def call_graph(items: dict[str, list[str]], alias: dict[str, str]):
    """(protocol.rs opcodes per item, referenced items per item)."""
    names = set(items)
    ops, edges = {}, {}
    for n, bodies in items.items():
        text = "\n".join(bodies)
        ops[n] = {alias[o] for o in re.findall(r"\b(REQ_[A-Z0-9_]+)\b", text) if o in alias}
        edges[n] = (set(re.findall(r"\b([A-Za-z_]\w*)\b", text)) & names) - {n}
    return ops, edges


def ffi_roots(names: set[str]) -> dict[str, set[str]]:
    """{core item: {the exported functions it is}}: the graph starts at the surface."""
    surf = parse_surface()
    exported = surf.methods + surf.free + [f for o in surf.objects.values() for f in o.methods]
    return {f.name: {f.name} for f in exported if f.name in names}


def reachable(roots, edges) -> dict[str, set[str]]:
    """{item: {roots it is reached from}}."""
    seen: dict[str, set[str]] = defaultdict(set)
    for r in roots:
        q = deque([r])
        visited = {r}
        while q:
            n = q.popleft()
            seen[n].add(r)
            for m in edges.get(n, ()):
                if m not in visited:
                    visited.add(m)
                    q.append(m)
    return seen


# ── main ─────────────────────────────────────────────────────────────────

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--refresh-firmware", type=Path, metavar="CLONE",
                    help="re-extract fixtures/firmware_req_opcodes.txt from a local firmware clone (host)")
    ap.add_argument("-v", "--verbose", action="store_true", help="list every opcode and the exported functions reaching it")
    a = ap.parse_args()
    if a.refresh_firmware:
        return refresh_firmware(a.refresh_firmware.expanduser())

    errors: list[str] = []
    core = core_opcodes()

    # 1. firmware vs core
    snap = firmware_snapshot()
    still_missing: set[str] = set()
    warnings: list[str] = []
    for key, sha in pinned_shas(reference_firmware()):
        report = errors if key == "tag_sha" else warnings
        fw = snap.get(sha)
        if fw is None:
            report.append(f"{SNAPSHOT.relative_to(ROOT)} has no section for {key} {sha}: "
                          "run --refresh-firmware on the host")
            continue
        missing = sorted(set(fw) - set(core), key=lambda n: fw[n])
        for n in missing:
            if n in FIRMWARE_NOT_IN_CORE:
                still_missing.add(n)
            else:
                report.append(f"firmware {key} defines {n}=0x{fw[n]:02X}; the core does not")
        for n in sorted(set(fw) & set(core)):
            if fw[n] != core[n]:
                report.append(f"{n}: firmware {key} 0x{fw[n]:02X} != core 0x{core[n]:02X}")
        extra = sorted(set(core) - set(fw))
        print(f"opcode_coverage: firmware {key} {sha[:7]}: {len(fw)} REQ_*, core {len(core)}; "
              f"missing in core: {', '.join(f'{n}=0x{fw[n]:02X}' for n in missing) or 'none'}"
              + (f"; core-only: {', '.join(extra)}" if extra else ""))
    for n in sorted(set(FIRMWARE_NOT_IN_CORE) - still_missing):
        errors.append(f"allowlist FIRMWARE_NOT_IN_CORE[{n}] is stale (no longer a gap): remove it")

    # 2. core vs dspi-ffi reachability
    items = core_items()
    ops, edges = call_graph(items, core_aliases(core))
    used_by_core = set().union(*ops.values()) & set(core)
    roots = ffi_roots(set(items))
    seen = reachable(roots, edges)
    callers: dict[str, set[str]] = defaultdict(set)
    for item, rs in seen.items():
        for op in ops.get(item, ()):
            for r in rs:
                callers[op] |= roots[r]
    unreachable = sorted(used_by_core - set(callers), key=lambda n: core[n])
    for n in unreachable:
        if n not in UNREACHABLE_FROM_FFI:
            users = sorted(i for i, o in ops.items() if n in o)
            errors.append(f"{n}=0x{core[n]:02X} is used by the core ({', '.join(users)}) "
                          "but nothing exported reaches it")
    for n in sorted(set(UNREACHABLE_FROM_FFI) - set(unreachable)):
        errors.append(f"allowlist UNREACHABLE_FROM_FFI[{n}] is stale: remove it")
    unused = sorted(set(core) - used_by_core, key=lambda n: core[n])
    print(f"opcode_coverage: core opcodes from {PROTOCOL.relative_to(TOP)}")
    print(f"opcode_coverage: core uses {len(used_by_core)} of {len(core)} opcodes; the exported surface reaches "
          f"{len(used_by_core & set(callers))} through {len(roots)} core entry points")
    if unused:
        print(f"opcode_coverage: {len(unused)} opcodes defined but unused by the core (info; mostly per-item GETs "
              "the bulk image replaces; -v lists them)")
    if a.verbose:
        merged = sorted((n, len(b)) for n, b in items.items() if len(b) > 1 and n in seen)
        print("  merged names (several items, one node): " + ", ".join(f"{n}×{k}" for n, k in merged))
        print("  unused: " + ", ".join(f"{n}=0x{core[n]:02X}" for n in unused))
        for n in sorted(callers, key=lambda n: core.get(n, 0)):
            if n in core:
                print(f"  0x{core[n]:02X} {n:40} <- {', '.join(sorted(callers[n]))}")

    for w in warnings:
        print(f"opcode_coverage: WARNING (release-branch head, not failing): {w}", file=sys.stderr)
    for e in errors:
        print(f"opcode_coverage: ERROR: {e}", file=sys.stderr)
    if errors:
        return 1
    print("opcode_coverage: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
