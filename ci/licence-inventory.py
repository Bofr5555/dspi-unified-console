#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""What goes into the iOS binary, who wrote it, and what licence it says it has.

Facts for the owner and Weeb Labs (PLAN-CLIENTS.md 0.4, WP LC1). Not legal text.

    ci/licence-inventory.py --write     regenerate docs/licensing/IOS-INVENTORY.md
                                        and ios-inventory.json (blames everything)
    ci/licence-inventory.py --check     compare the real iOS build with the
                                        recorded inventory: any change exits 1
    ci/licence-inventory.py --gate      as --check, but a new or removed source
                                        file by a known author only warns. A new
                                        crate, licence, asset, package or outside
                                        contributor still exits 1, and so does a
                                        recorded Swift half with no tree to check
                                        it against (ci/check.sh)

Runs on the host, standard library only, reads git read-only. Cargo runs in the
container (ci/run.sh). The inputs:

  Rust   `cargo tree -p dspi-ffi --no-default-features --target aarch64-apple-ios`, and the crates'
         dep-info and build-script inputs for the file list.
  Swift  the iOS target's members in `DSPi Console.xcodeproj/project.pbxproj`.
         Looked for in --console, then $DSPI_CONSOLE_TREE, then apps/apple in
         this repo (after MR1's Swift half), then a clone that --fetch-console
         makes under ~/.cache/dspi-build/work/licence-inventory/. Read from the
         git ref, never from the working tree.
  Firmware  $DSPI_FIRMWARE_TREE or ~/prog/DSPi, only with --write: the generated
         protocol constants' headers and any bundled .uf2 images.
  Web    (WP WB7) the web console's runtime JavaScript: the packages in
         apps/web/package-lock.json that are not dev-only (Preact), with the
         licence each declares. Its Rust core is apps/web/notices/rust-crates.txt
         (tools/gen_rust_notices.py, checked by the web step).

Re-run --write after each SW wave, and when the gate says to.
"""

from __future__ import annotations

import argparse
import concurrent.futures as futures
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT_DIR = ROOT / "docs/licensing"
OUT_MD = OUT_DIR / "IOS-INVENTORY.md"
OUT_JSON = OUT_DIR / "ios-inventory.json"
# -- WP WB7 --
WEB_LOCK = ROOT / "apps/web/package-lock.json"
# Upstream's DSPi-Link web page, built into `uconsole hub` (crates/dspi-hub/tools/build-link-web.sh)
HUB_PAGE = ROOT / "crates/dspi-hub/assets/web/hub/PROVENANCE.json"
NOTES_MD = OUT_DIR / "ios-inventory-notes.md"

BUILD_CACHE = Path(os.environ.get("DSPI_BUILD_CACHE", Path.home() / ".cache/dspi-build"))
FETCHED_CONSOLE = BUILD_CACHE / "work/licence-inventory/console-fork"
CONSOLE_URL = "https://github.com/Bofr5555/DSPi-Console.git"
CONSOLE_BRANCH = "ext-channel-116"
IMPORTED_CONSOLE = "apps/apple"
XCODEPROJ = "DSPi Console.xcodeproj/project.pbxproj"
IOS_TARGET = "DSPi Console iOS"

# The library the iOS app links, and the slice in an App Store binary. Change
# these when CG3 and SW1 give the iOS build its own features.
CARGO_SELECT = "-p dspi-ffi --no-default-features --target aarch64-apple-ios --locked"

# The old core dspi-core was ported from: Weeb Labs' Console-Linux core, deleted
# from the tree by WP Q3 and still in history here.
OLD_CORE = ("d190535", "crates/dspi-core-linux/src")
FIRMWARE_TAG = "v1.1.6"

WEEB_LABS = "Weeb Labs"
OWNER = "owner (bofr)"
UNCOMMITTED = "uncommitted"
AUTHOR_NAMES = {
    "Weeb Labs": WEEB_LABS,
    "WeebLabs": WEEB_LABS,
    "bofr": OWNER,
    "Bofr5555": OWNER,
    "Not Committed Yet": UNCOMMITTED,
}
KNOWN_HOLDERS = (WEEB_LABS, OWNER)

# Where blame alone understates the origin: git shows who typed the port.
CRATE_ORIGINS = {
    "dspi-core": (
        [WEEB_LABS],
        "Written in the core merge with Weeb Labs' Console-Linux core as its input "
        f"(`git show {OLD_CORE[0]}:{OLD_CORE[1]}`). Blame shows only the porter, so "
        "Weeb Labs is added here whatever a file's blame says.",
    ),
    "dspi-link": (
        [WEEB_LABS],
        "Written by the owner in dspi-android against Weeb Labs' DSPi-Link spec "
        "(MIT) and policy, following the macOS Console's `Link/` as its reference "
        "implementation (`src/lib.rs`). Whether that needs a grant is for them to "
        "judge; Weeb Labs is added here so it is not missed.",
    ),
    "dspi-ffi": ([], "New for the core merge, the owner's, over `dspi-core`."),
    "dspi-proto": (
        [],
        "From DSPi-Terminal, with history. `build.rs` generates its constants "
        "from the vendored firmware headers below.",
    ),
    "dspi-session": ([], "From DSPi-Terminal, with history."),
    "dspi-transport": ([], "From DSPi-Terminal, with history."),
}

# Assets whose origin is outside the tree they sit in. Longest prefix wins.
ASSET_ORIGINS = {
    "assets/onboarding/": (
        "Console fork `ext-channel-116` `48cf7c5`: `whatsnew.json` is `Onboarding/WhatsNew.json` with per-platform placeholders added by WP CG10, "
        "and renders back to it byte for byte on macOS; `getting_started.json` holds the strings of `Onboarding/GettingStartedView.swift` and "
        "`FirmwareInstallUI.swift`, some reworded and some added by WP CG10",
        "`LICENSE` there is GPL-3.0, the README there says MIT, and no file has a header. Treat as uncertain",
        "Weeb Labs + owner (bofr). Blame here shows only the commit that copied them",
    ),
    "DSPi Console/AutoEQ/autoeq_database.json": (
        "AutoEq data",
        "MIT (AutoEq's `LICENSE`); the measurements' sources are uncertain, see the notes",
        "AutoEq's copyright holder, and possibly the measurement sources",
    ),
    "DSPi Console/Link/policy/": (
        "DSPi-Link",
        "MIT (DSPi-Link `LICENSE`, Weeb Labs)",
        "none: MIT, notice only",
    ),
    "DSPi Console/Web/": (
        "DSPi-Link `js/apps/web`, built",
        "MIT (DSPi-Link `README.md`, `LICENSE`)",
        "none: MIT, notice only. Only `LinkServer.swift` loads it and iOS excludes that, so it is removable",
    ),
    "DSPi Console/Firmware/": (
        "DSPi firmware",
        "GPL-3.0 (`LICENSE`), with third-party parts",
        "cannot stay under a GPL grant alone: SW18 removes these from the iOS target",
    ),
}

# Apple SDK modules; any other import in an iOS member is flagged.
APPLE_MODULES = {
    "AppKit", "AVFoundation", "Accelerate", "CoreFoundation", "CoreGraphics",
    "CoreMIDI", "CoreText", "CryptoKit", "Combine", "Darwin", "Dispatch",
    "Foundation", "IOKit", "MetalKit", "Metal", "Network", "OSLog", "Observation",
    "QuartzCore", "Security", "ServiceManagement", "Swift", "SwiftUI", "Synchronization",
    "UIKit", "UniformTypeIdentifiers", "WebKit", "XCTest", "os", "simd", "Charts",
    "CoreAudio", "AudioToolbox", "CoreServices", "SystemConfiguration", "Testing",
    "_Concurrency", "ObjectiveC", "CoreImage", "CoreBluetooth", "CoreLocation",
    "MetalPerformanceShaders", "dnssd",
}

PERMISSIVE = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unlicense",
    "Unicode-3.0", "Unicode-DFS-2016", "0BSD", "BSL-1.0", "CC0-1.0", "LLVM-exception",
}
TEXT_LIMIT = 1_000_000
AI_TRAILER = re.compile(r"\b(claude|anthropic|copilot|chatgpt|openai)\b", re.I)
COPYRIGHT = re.compile(r"[^\n]*(?:\bcopyright\b|\bSPDX-License|licensed under|\(c\) \d{4})[^\n]*", re.I)
# On a line COPYRIGHT matches, this comment says the line is no notice (a test's list
# of forbidden words, say). Such a line leaves the notices and is listed in the
# inventory under its own heading, so a waiver is always visible.
NOT_A_NOTICE = "licence-inventory: not a notice"
PKG = re.compile(r"^(\S+) v(\S+)(?: \(([^)]*)\))?")
SWIFT_IMPORT = re.compile(
    r"^\s*(?:@\w+\s+)*import\s+(?:(?:class|struct|enum|func|protocol|typealias|var|let)\s+)?(\w+)",
    re.M,
)
GATED_MOD = re.compile(
    r"#\[cfg\([^\]]*(?:target_os|target_vendor|unix|windows)[^\]]*\)\]\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;"
)


def die(msg: str) -> None:
    print(f"licence-inventory: {msg}", file=sys.stderr)
    sys.exit(2)


def git(repo: Path, *args: str, binary: bool = False, check: bool = True):
    proc = subprocess.run(["git", "-C", str(repo), *args], capture_output=True)
    if check and proc.returncode != 0:
        die(f"git {' '.join(args)} in {repo}: {proc.stderr.decode(errors='replace').strip()}")
    return proc.stdout if binary else proc.stdout.decode(errors="replace")


def holder_of(author: str) -> str:
    return AUTHOR_NAMES.get(author, author)


def in_container(command: str) -> str:
    proc = subprocess.run(
        [str(ROOT / "ci/run.sh"), command],
        stdin=subprocess.DEVNULL,
        capture_output=True,
    )
    if proc.returncode != 0:
        die(f"ci/run.sh failed: {command}\n{proc.stderr.decode(errors='replace')[-2000:]}")
    return proc.stdout.decode(errors="replace")


# ---------------------------------------------------------------------------
# Rust: the graph and the files
# ---------------------------------------------------------------------------


def parse_tree(text: str) -> dict[str, dict]:
    crates = {}
    for line in text.splitlines():
        line = line.removesuffix(" (*)")
        if not line.strip():
            continue
        spec, licence, repo = (line.split("|") + ["", ""])[:3]
        match = PKG.match(spec)
        if not match:
            die(f"cannot read cargo tree line: {line}")
        name, version, where = match.groups()
        first_party = bool(where) and where.startswith("/src")
        crates[f"{name} {version}"] = {
            "name": name,
            "version": version,
            "license": licence.strip(),
            "repository": repo.strip(),
            "first_party": first_party,
            "manifest_dir": where[len("/src/") :] if first_party else "",
        }
    return crates


def collect_cargo() -> dict:
    """The linked crates, the compile-only ones, and each first-party crate's files."""
    fmt = "'{p}|{l}|{r}'"
    tree = f"cargo tree {CARGO_SELECT} --prefix none --format {fmt}"
    out = in_container(
        f"{tree} -e normal,no-proc-macro; echo @@@; {tree} -e normal; echo @@@; "
        f"cargo check {CARGO_SELECT} --lib --message-format=json"
    )
    linked_text, all_text, check_text = out.split("@@@\n")
    linked = parse_tree(linked_text)
    every = parse_tree(all_text)
    compile_only = {k: v for k, v in every.items() if k not in linked}

    dep_files: dict[str, str] = {}
    build_outputs: dict[str, str] = {}
    for line in check_text.splitlines():
        if not line.startswith("{"):
            continue
        msg = json.loads(line)
        pid = msg.get("package_id", "")
        if not pid.startswith("path+file"):
            continue
        name = re.search(r"/([^/#]+)#", pid + "#").group(1)
        if msg.get("reason") == "compiler-artifact" and "lib" in msg["target"]["kind"]:
            rmeta = next(f for f in msg["filenames"] if f.endswith(".rmeta"))
            stem = Path(rmeta).stem.removeprefix("lib")
            dep_files[name] = f"{Path(rmeta).parent}/{stem}.d"
        elif msg.get("reason") == "build-script-executed":
            build_outputs[name] = str(Path(msg["out_dir"]).parent / "output")
    if not dep_files:
        die("cargo check reported no first-party library")

    # Second pass: read those files inside the container, where they live.
    cats = "".join(f"echo '@@@dep {n}'; cat '{p}'; " for n, p in dep_files.items())
    cats += "".join(f"echo '@@@build {n}'; cat '{p}'; " for n, p in build_outputs.items())
    files: dict[str, set[str]] = defaultdict(set)
    inputs: dict[str, set[str]] = defaultdict(set)
    current, name = "", ""
    dirs = {c["name"]: c["manifest_dir"] for c in linked.values() if c["first_party"]}
    for line in in_container(cats).splitlines():
        if line.startswith("@@@"):
            current, name = line[3:].split(" ", 1)
        elif current == "dep":
            match = re.match(r"^([^\s:]+):$", line)
            if match and not match.group(1).startswith("/"):
                files[name].add(os.path.normpath(match.group(1)))
        elif current == "build" and line.startswith("cargo:rerun-if-changed="):
            rel = line.split("=", 1)[1]
            if rel.endswith((".h", ".json", ".txt", ".bin")) and name in dirs:
                inputs[name].add(f"{dirs[name]}/{rel}")
    return {
        "linked": linked,
        "compile_only": compile_only,
        "files": {k: sorted(v) for k, v in files.items()},
        "build_inputs": {k: sorted(v) for k, v in inputs.items()},
    }


def platform_gated_modules(files: dict[str, list[str]]) -> list[str]:
    """The file list is one target's, so a platform-gated `mod` would skew it."""
    hits = []
    for paths in files.values():
        for rel in paths:
            path = ROOT / rel
            if path.suffix == ".rs" and path.exists() and GATED_MOD.search(path.read_text(errors="replace")):
                hits.append(rel)
    return sorted(hits)


def classify_licence(expr: str) -> str:
    """permissive, mpl, choice (a copyleft term with a permissive alternative),
    copyleft, or unknown. A reading of the SPDX words, not a legal conclusion."""
    if not expr:
        return "unknown"
    words = set(re.findall(r"[A-Za-z0-9.+-]+", expr)) - {"OR", "AND", "WITH"}
    known = PERMISSIVE | {"MPL-2.0"}
    copyleft = {w for w in words if re.match(r"(A|L)?GPL|EPL|CDDL|SSPL|CC-BY-SA", w)}
    if copyleft:
        rest = words - copyleft
        return "choice" if " OR " in expr and rest <= PERMISSIVE and rest else "copyleft"
    if not words <= known:
        return "unknown"
    return "mpl" if "MPL-2.0" in words else "permissive"


# ---------------------------------------------------------------------------
# Xcode: a pbxproj is an old-style plist
# ---------------------------------------------------------------------------

PBX_TOKEN = re.compile(
    r'\s+|/\*.*?\*/|//[^\n]*|"((?:[^"\\]|\\.)*)"|([^\s"{}();=,]+)|([{}();=,])', re.S
)


def parse_pbxproj(text: str):
    tokens: list[tuple[str, str]] = []
    pos = 0
    while pos < len(text):
        match = PBX_TOKEN.match(text, pos)
        if not match:
            die(f"cannot read project.pbxproj at offset {pos}")
        pos = match.end()
        if match.group(1) is not None:
            tokens.append(("s", bytes(match.group(1), "utf8").decode("unicode_escape", "replace")))
        elif match.group(2) is not None:
            tokens.append(("s", match.group(2)))
        elif match.group(3) is not None:
            tokens.append(("p", match.group(3)))

    index = 0

    def value():
        nonlocal index
        kind, tok = tokens[index]
        index += 1
        if kind == "s":
            return tok
        if tok == "{":
            obj = {}
            while tokens[index] != ("p", "}"):
                key = value()
                index += 1  # =
                obj[key] = value()
                index += 1  # ;
            index += 1
            return obj
        if tok == "(":
            items = []
            while tokens[index] != ("p", ")"):
                items.append(value())
                if tokens[index] == ("p", ","):
                    index += 1
            index += 1
            return items
        die(f"unexpected {tok!r} in project.pbxproj")
        return None

    return value()


def swift_kind(rel: str) -> str:
    if rel.endswith(".swift"):
        return "swift"
    if rel.endswith(".uf2"):
        return "firmware-image"
    if rel.endswith((".plist", ".entitlements")):
        return "config"
    return "asset"


def collect_swift(repo: Path, prefix: str, ref: str) -> dict:
    """The iOS target's members: its synchronized folders minus their exceptions,
    plus the files named by build settings and explicit build-phase entries."""
    project = parse_pbxproj(git(repo, "show", f"{ref}:{prefix}{XCODEPROJ}"))
    objects = project["objects"]
    natives = {o["name"]: o for o in objects.values() if o.get("isa") == "PBXNativeTarget"}
    if IOS_TARGET not in natives:
        die(f"no target {IOS_TARGET!r} in the Xcode project: {sorted(natives)}")
    target_id = next(k for k, v in objects.items() if v is natives[IOS_TARGET])
    target = natives[IOS_TARGET]

    tree = git(repo, "ls-tree", "-r", "-z", "--name-only", ref, *([prefix] if prefix else [])).split("\0")
    tree = [p[len(prefix) :] for p in tree if p]

    members: set[str] = set()
    excluded: set[str] = set()
    for group_id in target.get("fileSystemSynchronizedGroups", []):
        group = objects[group_id]
        root = group["path"].rstrip("/") + "/"
        skips: set[str] = set()
        for exc_id in group.get("exceptions", []):
            exc = objects[exc_id]
            if exc.get("target") == target_id:
                skips.update(root + p for p in exc.get("membershipExceptions", []))
        for rel in tree:
            if rel.startswith(root):
                skip = rel in skips or any(rel.startswith(s + "/") for s in skips)
                (excluded if skip else members).add(rel)

    dev_only: set[str] = set()
    for cfg_id in objects[target["buildConfigurationList"]]["buildConfigurations"]:
        settings = objects[cfg_id]["buildSettings"]
        for key in ("INFOPLIST_FILE", "CODE_SIGN_ENTITLEMENTS"):
            if settings.get(key):
                members.add(settings[key])
        for path in re.findall(r'"([^"]+)"', settings.get("DEVELOPMENT_ASSET_PATHS", "")):
            dev_only.update(p for p in members if p.startswith(path.rstrip("/") + "/"))

    explicit, frameworks = [], []
    for phase_id in target.get("buildPhases", []):
        phase = objects[phase_id]
        for file_id in phase.get("files", []):
            ref_id = objects[file_id].get("fileRef") or objects[file_id].get("productRef")
            item = objects.get(ref_id, {})
            label = item.get("path") or item.get("productName") or ref_id
            (frameworks if phase.get("isa") == "PBXFrameworksBuildPhase" else explicit).append(label)
    members.update(p for p in explicit if p in tree)
    products = sorted(objects[p]["productName"] for p in target.get("packageProductDependencies", []))
    other_targets = sorted(n for n in natives if n != IOS_TARGET)
    project_packages = sorted(
        o["repositoryURL"] for o in objects.values() if o.get("isa") == "XCRemoteSwiftPackageReference"
    )
    return {
        "members": sorted(members),
        "excluded": sorted(excluded),
        "dev_only": sorted(dev_only),
        "products": products,
        "frameworks": sorted(frameworks),
        "other_targets": other_targets,
        "project_packages": project_packages,
    }


def find_console(args) -> tuple[Path, str, str] | None:
    """(repo, path prefix inside it, ref), or None when no Swift tree is at hand."""
    tree = args.console or os.environ.get("DSPI_CONSOLE_TREE")
    if not tree and (ROOT / IMPORTED_CONSOLE / XCODEPROJ).exists():
        return ROOT, IMPORTED_CONSOLE + "/", "HEAD"
    if tree:
        repo = Path(tree).expanduser()
    else:
        repo = FETCHED_CONSOLE
        if not repo.exists() and args.fetch_console:
            repo.parent.mkdir(parents=True, exist_ok=True)
            proc = subprocess.run(
                ["git", "clone", "-q", "--branch", CONSOLE_BRANCH, CONSOLE_URL, str(repo)],
                capture_output=True,
            )
            if proc.returncode != 0:
                print(f"licence-inventory: could not clone the fork: {proc.stderr.decode()}", file=sys.stderr)
                return None
        elif repo.exists() and args.fetch_console and args.write:
            git(repo, "fetch", "-q", "origin", CONSOLE_BRANCH)
            return repo, "", "FETCH_HEAD"
    if not (repo / ".git").exists():
        return None
    return repo, "", args.console_ref or CONSOLE_BRANCH


# ---------------------------------------------------------------------------
# Authorship
# ---------------------------------------------------------------------------


def blame(repo: Path, ref: str, path: str) -> dict[str, int]:
    out = git(repo, "blame", "--line-porcelain", "-w", "-C", ref, "--", path, binary=True)
    counts: Counter = Counter()
    for line in out.split(b"\n"):
        if line.startswith(b"author "):
            counts[holder_of(line[7:].decode(errors="replace"))] += 1
    return dict(counts)


def log_authors(repo: Path, ref: str, path: str) -> dict[str, int]:
    counts = Counter(holder_of(a) for a in git(repo, "log", "--format=%aN", ref, "--", path).splitlines() if a)
    return dict(counts)


def describe_file(repo: Path, ref: str, path: str) -> dict:
    """One file: bytes, blamed lines for text, committing authors for data."""
    committed = subprocess.run(["git", "-C", str(repo), "cat-file", "-e", f"{ref}:{path}"], capture_output=True)
    if committed.returncode != 0:
        disk = ROOT / path
        return {"path": path, "bytes": disk.stat().st_size if disk.exists() else 0, "lines": {UNCOMMITTED: 0},
                "by": "uncommitted", "copyright": [], "waived": []}
    data = git(repo, "show", f"{ref}:{path}", binary=True)
    text = b"\0" not in data[:8192] and len(data) <= TEXT_LIMIT
    entry = {"path": path, "bytes": len(data)}
    if text:
        entry["lines"] = blame(repo, ref, path)
        entry["by"] = "blame"
        decoded = data.decode(errors="replace")
        hits = {m.group(0).strip() for m in COPYRIGHT.finditer(decoded)}
        waived = {h for h in hits if NOT_A_NOTICE in h}
        entry["copyright"] = sorted(h[:90] for h in hits - waived)[:5]
        entry["waived"] = sorted(waived)
    else:
        entry["lines"] = log_authors(repo, ref, path)
        entry["by"] = "commits"
        entry["copyright"] = []
        entry["waived"] = []
    return entry


def describe_files(repo: Path, ref: str, paths: list[str]) -> list[dict]:
    with futures.ThreadPoolExecutor(max_workers=8) as pool:
        return list(pool.map(lambda p: describe_file(repo, ref, p), paths))


def outside_contributors(repo: Path, ref: str) -> dict[str, dict]:
    """Every author beyond Weeb Labs and the owner, whole history, with the paths touched."""
    people: dict[str, dict] = {}
    out = git(repo, "log", ref, "--format=%x01%aN", "--name-only", "-z", binary=False)
    for chunk in out.split("\x01")[1:]:
        head, _, rest = chunk.partition("\n")
        holder = holder_of(head.strip("\0"))
        if holder in KNOWN_HOLDERS:
            continue
        rec = people.setdefault(holder, {"commits": 0, "paths": set()})
        rec["commits"] += 1
        rec["paths"].update(p.strip() for p in rest.split("\0") if p.strip())
    return {k: {"commits": v["commits"], "paths": sorted(v["paths"])} for k, v in people.items()}


def trailer_people(repo: Path, ref: str) -> dict[str, Counter]:
    out = git(
        repo, "log", ref,
        "--format=%aN%x02%(trailers:key=Co-authored-by,key=Signed-off-by,valueonly,separator=%x03)",
    )
    humans: Counter = Counter()
    tools: Counter = Counter()
    for line in out.splitlines():
        author, _, trailers = line.partition("\x02")
        seen_tool = False
        for trailer in filter(None, trailers.split("\x03")):
            name = re.sub(r"\s*<.*", "", trailer).strip()
            if AI_TRAILER.search(trailer):
                seen_tool = True
            elif holder_of(name) not in KNOWN_HOLDERS:
                humans[name] += 1
        if seen_tool:
            tools[holder_of(author)] += 1
    return {"humans": humans, "tools": tools}


def old_core_overlap(rust_files: list[str]) -> dict[str, int]:
    """Percent of a file's substantial lines found verbatim in the old core."""

    def lines(text: str) -> set[str]:
        out = set()
        for raw in text.splitlines():
            line = " ".join(raw.split())
            if len(line) >= 24 and not line.startswith(("//", "use ", "#[", "pub use ")):
                out.add(line)
        return out

    commit, base = OLD_CORE
    names = git(ROOT, "ls-tree", "-r", "--name-only", commit, base, check=False).split()
    old: set[str] = set()
    for name in names:
        old |= lines(git(ROOT, "show", f"{commit}:{name}", check=False))
    result = {}
    for rel in rust_files:
        path = ROOT / rel
        if path.suffix == ".rs" and path.exists():
            mine = lines(path.read_text(errors="replace"))
            result[rel] = round(100 * len(mine & old) / len(mine)) if mine else 0
    return result


def firmware_report(tree: Path | None, ref: str, sub: str, paths: list[str] | None = None) -> dict | None:
    """Authorship of firmware files; surviving lines per outside author, and the third-party parts."""
    if tree is None:
        return None
    if paths is None:
        listed = git(tree, "ls-tree", "-r", "-z", "--name-only", ref, sub).split("\0")
        paths = [
            p for p in listed
            if p and re.search(r"\.(c|h|cpp|hpp|S|pio|cmake|txt)$", p) and "/lufa/" not in p
        ]
    described = describe_files(tree, ref, paths)
    totals: Counter = Counter()
    outside: dict[str, dict] = defaultdict(dict)
    for entry in described:
        for who, count in entry["lines"].items():
            totals[who] += count
            if who not in KNOWN_HOLDERS:
                outside[who][entry["path"]] = count
    return {
        "ref": ref,
        "commit": git(tree, "rev-parse", "--short", ref).strip(),
        "files": len(described),
        "lines": dict(totals),
        "outside": dict(outside),
        "commit_authors": {k: v["commits"] for k, v in outside_contributors(tree, ref).items()},
        "licence_files": [
            p for p in git(tree, "ls-tree", "-r", "--name-only", ref).splitlines()
            if re.search(r"(^|/)(licen[cs]e|copying)", p, re.I)
        ],
        "copyright": sorted(
            {c for e in described for c in e["copyright"] if not re.search("weeb", c, re.I)}
        )[:20],
    }


# ---------------------------------------------------------------------------
# Snapshot and comparison
# ---------------------------------------------------------------------------


# -- WP WB7 --
def collect_web() -> dict:
    """The web console's runtime packages (what its dist bundles), from the lockfile."""
    lock = json.loads(WEB_LOCK.read_text())
    packages = {}
    for path, pkg in lock.get("packages", {}).items():
        if not path or pkg.get("dev") or pkg.get("devOptional"):
            continue
        name = path.rsplit("node_modules/", 1)[-1]
        packages[name] = {"version": pkg.get("version", ""), "license": pkg.get("license", "")}
    hub = json.loads(HUB_PAGE.read_text())
    return {
        "packages": packages,
        # The hub's /hub/ page: upstream's, built unmodified (its own lockfile's packages).
        "hub_page": {
            "repository": hub["repository"],
            "commit": hub["commit"],
            "version": hub["version"],
            "license": hub["license"],
            "packages": hub["bundled_packages"],
        },
    }


def light_snapshot(args) -> dict:
    """What --check compares: cheap, no blame."""
    cargo = collect_cargo()
    gated = platform_gated_modules(cargo["files"])
    if gated:
        print(f"licence-inventory: platform-gated modules, file list may differ on iOS: {gated}", file=sys.stderr)
    snap = {
        "rust": {
            "crates": {
                c["name"]: {"version": c["version"], "license": c["license"], "first_party": c["first_party"]}
                for c in cargo["linked"].values()
            },
            "files": sorted(p for v in cargo["files"].values() for p in v)
            + sorted(p for v in cargo["build_inputs"].values() for p in v),
        },
        "swift": None,
        # -- WP WB7 --
        "web": collect_web(),
    }
    rust_paths = snap["rust"]["files"]
    snap["rust"]["authors"] = sorted(
        {holder_of(a) for a in git(ROOT, "log", "--format=%aN", "HEAD", "--", *rust_paths).splitlines() if a}
    )
    snap["_cargo"] = cargo
    console = find_console(args)
    if console:
        repo, prefix, ref = console
        swift = collect_swift(repo, prefix, ref)
        pathspecs = [prefix + p for p in swift["members"]]
        authors = sorted({holder_of(a) for a in git(repo, "log", "--format=%aN", ref, "--", *pathspecs).splitlines() if a})
        snap["swift"] = {
            "commit": git(repo, "rev-parse", ref).strip(),
            "files": swift["members"],
            "products": swift["products"],
            "frameworks": swift["frameworks"],
            "authors": authors,
        }
        snap["_swift"] = swift
        snap["_console"] = console
    return snap


def compare(old: dict, new: dict) -> tuple[list[str], list[str]]:
    """(hard differences, soft differences) between the record and the build."""
    hard: list[str] = []
    soft: list[str] = []
    old_c, new_c = old["rust"]["crates"], new["rust"]["crates"]
    for name in sorted(set(new_c) - set(old_c)):
        hard.append(f"new crate in the iOS graph: {name} {new_c[name]['version']} ({new_c[name]['license'] or 'no licence field'})")
    for name in sorted(set(old_c) - set(new_c)):
        soft.append(f"crate left the iOS graph: {name}")
    for name in sorted(set(old_c) & set(new_c)):
        if old_c[name]["license"] != new_c[name]["license"]:
            hard.append(f"licence changed: {name}: {old_c[name]['license']!r} -> {new_c[name]['license']!r}")
        if old_c[name]["version"] != new_c[name]["version"]:
            soft.append(f"version changed: {name} {old_c[name]['version']} -> {new_c[name]['version']}")
    old_r, new_r = set(old["rust"]["files"]), set(new["rust"]["files"])
    for rel in sorted(new_r - old_r):
        (soft if rel.endswith(".rs") else hard).append(f"new Rust input: {rel}")
    for rel in sorted(old_r - new_r):
        soft.append(f"Rust input gone: {rel}")
    for who in sorted(set(new["rust"]["authors"]) - set(old["rust"]["authors"]) - set(KNOWN_HOLDERS)):
        hard.append(f"new outside contributor to the Rust files: {who}")

    # -- WP WB7 -- the web console's JavaScript
    old_w, new_w = old.get("web"), new.get("web")
    if old_w is None:
        hard.append("the record has no web half: run --write")
    elif new_w is not None:
        old_p, new_p = old_w["packages"], new_w["packages"]
        for name in sorted(set(new_p) - set(old_p)):
            hard.append(f"new package in the web console: {name} {new_p[name]['version']} ({new_p[name]['license'] or 'no licence field'})")
        for name in sorted(set(old_p) - set(new_p)):
            soft.append(f"package left the web console: {name}")
        for name in sorted(set(old_p) & set(new_p)):
            if old_p[name]["license"] != new_p[name]["license"]:
                hard.append(f"web licence changed: {name}: {old_p[name]['license']!r} -> {new_p[name]['license']!r}")
            if old_p[name]["version"] != new_p[name]["version"]:
                soft.append(f"web version changed: {name} {old_p[name]['version']} -> {new_p[name]['version']}")
        old_h, new_h = old_w.get("hub_page"), new_w.get("hub_page")
        if old_h is None:
            hard.append("the record has no hub page (upstream's DSPi-Link web page): run --write")
        elif new_h != old_h:
            for key in ("license", "packages"):
                if old_h.get(key) != new_h.get(key):
                    hard.append(f"hub page {key} changed: {old_h.get(key)!r} -> {new_h.get(key)!r}")
            if old_h.get("commit") != new_h.get("commit"):
                soft.append(f"hub page commit changed: {old_h['commit'][:8]} -> {new_h['commit'][:8]}")

    old_s, new_s = old.get("swift"), new.get("swift")
    if new_s is None:
        # The record has a Swift half, so a missing tree must not pass as a match.
        message = "Swift half not checked: no Console tree (--console, $DSPI_CONSOLE_TREE or --fetch-console)"
        (hard if old_s is not None else soft).append(message)
        return hard, soft
    if old_s is None:
        hard.append("the record has no Swift half: run --write")
        return hard, soft
    for rel in sorted(set(new_s["files"]) - set(old_s["files"])):
        (soft if swift_kind(rel) == "swift" else hard).append(f"new iOS target member: {rel}")
    for rel in sorted(set(old_s["files"]) - set(new_s["files"])):
        soft.append(f"iOS target member gone: {rel}")
    for key in ("products", "frameworks"):
        for item in sorted(set(new_s[key]) - set(old_s[key])):
            hard.append(f"new Swift {key[:-1]} in the iOS target: {item}")
    for who in sorted(set(new_s["authors"]) - set(old_s["authors"]) - set(KNOWN_HOLDERS)):
        hard.append(f"new outside contributor to iOS target files: {who}")
    if old_s["commit"] != new_s["commit"]:
        soft.append(f"Console commit changed: {old_s['commit'][:7]} -> {new_s['commit'][:7]}")
    return hard, soft


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


def holders_cell(lines: dict[str, int]) -> str:
    items = sorted(lines.items(), key=lambda kv: (-kv[1], kv[0]))
    return ", ".join(f"{who} {count}" for who, count in items) or "none"


def needs_cell(holders: list[str]) -> str:
    return " + ".join(sorted(holders, key=lambda h: (h not in KNOWN_HOLDERS, h != WEEB_LABS, h))) or "none"


def asset_origin(rel: str):
    best = None
    for prefix, value in ASSET_ORIGINS.items():
        if rel.startswith(prefix) and (best is None or len(prefix) > len(best[0])):
            best = (prefix, value)
    return best[1] if best else None


def render(data: dict) -> str:
    rust, swift, fw = data["rust"], data["swift"], data["firmware"]
    third = rust["third_party"]
    groups = Counter(c["class"] for c in third)
    out: list[str] = []
    emit = out.append
    emit("# iOS licence inventory\n")
    emit("*Generated by `ci/licence-inventory.py --write`; do not edit by hand. Hand-written notes come from "
         "`ios-inventory-notes.md`. Facts for the owner and Weeb Labs (`PLAN-CLIENTS.md` 0.4). "
         "**Not legal advice, and no licence is changed by this file.***\n")
    emit(f"- **Repository:** `{data['repo_commit']}`. **Rust graph:** `cargo tree {CARGO_SELECT} -e normal,no-proc-macro`.")
    if swift:
        emit(f"- **Swift:** `{swift['source']}` at `{swift['commit'][:7]}`, Xcode target `{IOS_TARGET}`.")
    emit("- **Holders** come from `git blame -w -C` where a file is text, and from its commits where it is data. "
         "Every person outside Weeb Labs and the owner is named.\n")

    emit("## 1. Headline numbers\n")
    emit("| Part | Count | Authorship |")
    emit("|---|---|---|")
    fp_lines: Counter = Counter()
    for crate in rust["first_party"]:
        for entry in crate["files"]:
            if entry["by"] == "blame":
                fp_lines.update(entry["lines"])
    emit(f"| First-party Rust crates linked | {len(rust['first_party'])} crates, "
         f"{sum(len(c['files']) for c in rust['first_party'])} files | {holders_cell(fp_lines)} lines by blame; "
         "`dspi-core` was written from Weeb Labs' old core (section 3) |")
    emit(f"| Third-party Rust crates linked | {len(third)} | "
         + ", ".join(f"{k}: {v}" for k, v in sorted(groups.items())) + " |")
    emit(f"| Crates used only at compile time | {len(rust['compile_only'])} | not in the binary |")
    if swift:
        sw_lines: Counter = Counter()
        kinds = Counter(e["kind"] for e in swift["files"])
        for entry in swift["files"]:
            if entry["kind"] == "swift":
                sw_lines.update(entry["lines"])
        emit(f"| Swift files in the iOS target | {kinds['swift']} | {holders_cell(sw_lines)} lines by blame |")
        emit(f"| Other files in the iOS target | {sum(v for k, v in kinds.items() if k != 'swift')} "
             f"({kinds['firmware-image']} firmware images) | section 5 |")
        emit(f"| Swift packages linked by the iOS target | {len(swift['products'])} | "
             f"project declares {', '.join(swift['project_packages']) or 'none'}, used by other targets |")
    emit("")

    emit("## 2. Who must agree to what\n")
    emit("What each party would be asked to grant if **today's** iOS binary were the build. SW removes Swift and "
         "adds the core, so re-run after each wave. Terms are for the owner and Weeb Labs.\n")
    emit("| Party | Their part of the iOS build |")
    emit("|---|---|")
    needed: dict[str, list[str]] = defaultdict(list)
    for crate in rust["first_party"]:
        for who in crate["needs"]:
            needed[who].append(f"`{crate['name']}`")
    swift_needs: dict[str, list[dict]] = defaultdict(list)
    if swift:
        for entry in swift["files"]:
            if entry["kind"] == "swift":
                for who in entry["needs"]:
                    swift_needs[who].append(entry)
    for who in sorted(set(needed) | set(swift_needs), key=lambda h: (h not in KNOWN_HOLDERS, h != WEEB_LABS, h)):
        parts = []
        if needed[who]:
            parts.append("Rust " + ", ".join(needed[who]))
        if swift_needs[who]:
            names = swift_needs[who]
            total = sum(e["lines"].get(who, 0) for e in names)
            if who in KNOWN_HOLDERS:
                parts.append(f"{len(names)} Swift files, {total} lines")
            else:
                parts.append("Swift " + ", ".join(f"`{Path(e['path']).name}` ({e['lines'].get(who, 0)})" for e in names))
        emit(f"| {who} | {'; '.join(parts)} |")
    if fw.get("headers"):
        outside = fw["headers"]["outside"]
        people = ", ".join(sorted(outside)) or "nobody outside Weeb Labs and the owner"
        emit(f"| Firmware header authors (generated constants in `dspi-proto`) | {people} |")
    if fw.get("images"):
        people = ", ".join(f"{k} ({sum(v.values())} lines)" for k, v in sorted(fw["images"]["outside"].items()))
        emit(f"| Firmware authors, **only while the `.uf2` images stay in the target** | {people} |")
    emit("| Third-party crates | No grant; their licences' notice terms (section 4). MPL-2.0 is flagged there |")
    emit("")

    emit("## 3. First-party Rust crates in the iOS binary\n")
    emit("From the crates' dep-info (a `--lib` check), so test modules are out and `include_str!` data is in. "
         f"Platform-gated `mod` declarations that would skew it: {', '.join(rust['gated_modules']) or 'none'}. "
         "Code gated inside a file is not separated.\n")
    emit("| Crate | Licence field | Files | Lines by blame | Needs | Origin |")
    emit("|---|---|---|---|---|---|")
    for crate in rust["first_party"]:
        totals: Counter = Counter()
        for entry in crate["files"]:
            if entry["by"] == "blame":
                totals.update(entry["lines"])
        emit(f"| `{crate['name']}` | {crate['license'] or '**none**'} | {len(crate['files'])} | {holders_cell(totals)} "
             f"| {needs_cell(crate['needs'])} | {crate['origin']} |")
    emit("")
    emit(f"`dspi-core`: about {rust['old_core_overlap_pct']}% of its substantial lines are verbatim in the old core, "
         f"across {rust['old_core_overlap_files']} files; the file table gives each. A low figure does not rule out "
         "a port of behaviour, which is what the plan asked of those WPs.\n")
    if rust["copyright_hits"]:
        emit("Copyright or licence wording inside these files:\n")
        for path, hits in sorted(rust["copyright_hits"].items()):
            emit(f"- `{path}`: {'; '.join(f'`{h}`' for h in hits)}")
        emit("")
    if rust.get("waived_hits"):
        emit(f"Lines the scanner matched but a comment marks as no notice (`{NOT_A_NOTICE}`):\n")
        for path, hits in sorted(rust["waived_hits"].items()):
            emit(f"- `{path}`: {'; '.join(f'`{h}`' for h in hits)}")
        emit("")
    emit("<details><summary>Every first-party Rust file</summary>\n")
    emit("| File | Lines by blame | Verbatim in old core |")
    emit("|---|---|---|")
    for crate in rust["first_party"]:
        for entry in crate["files"]:
            pct = rust["overlap"].get(entry["path"])
            emit(f"| `{entry['path']}` | {holders_cell(entry['lines'])} | {'' if pct is None else f'{pct}%'} |")
    emit("\n</details>\n")
    for crate in rust["first_party"]:
        for entry in crate["files"]:
            origin = asset_origin(entry["path"])
            if origin:
                emit(f"- `{entry['path']}`: origin {origin[0]}. Licence as read: {origin[1]}. Needs: {origin[2]}.")
    emit("")
    if fw.get("headers"):
        header = fw["headers"]
        emit("### Firmware headers behind `dspi-proto`'s generated constants\n")
        emit(f"`crates/dspi-proto/build.rs` generates Rust constants from {header['files']} vendored firmware headers, so they "
             f"reach the binary as numbers and names, though the headers themselves are not compiled in. They come from "
             f"`{header['branch']}` `{header['commit']}` (GPL-3.0), which is Weeb Labs' release plus the fork's extension "
             f"channel, hence the owner's lines. Blame by holder: {holders_cell(header['lines'])}.\n")
        if fw.get("images"):
            emit(f"The `.uf2` images in section 6 are a different firmware: they are named for tag `{FIRMWARE_TAG}` "
                 f"(`{fw['images']['commit']}`), without those lines.\n")
        for who, files in sorted(header["outside"].items()):
            emit(f"- **{who}:** " + ", ".join(f"`{Path(p).name}` ({n})" for p, n in sorted(files.items())))
        emit("")

    emit("## 4. Third-party crates in the iOS binary\n")
    emit("Licence as declared in each crate's manifest; the crates' own licence files were not read one by one. "
         "`cargo-deny` covers the whole workspace (`deny.toml`).\n")
    emit("| Crate | Version | Licence | Class |")
    emit("|---|---|---|---|")
    for crate in third:
        emit(f"| {crate['name']} | {crate['version']} | {crate['license'] or '**none**'} | {crate['class']} |")
    emit("")
    emit("<details><summary>Compile-time only (proc-macros and their dependencies), not in the binary</summary>\n")
    emit(", ".join(f"{c['name']} ({c['license']})" for c in rust["compile_only"]))
    emit("\n</details>\n")

    if swift:
        emit("## 5. The iOS target's files\n")
        emit(f"The target uses synchronized folders: everything under `DSPi Console/` is a member unless it is in the "
             f"exception list. **`.uf2` images in the target: "
             f"{len([e for e in swift['files'] if e['kind'] == 'firmware-image'])}.** Modules imported beyond Apple's SDK: "
             f"{', '.join(swift['foreign_imports']) or 'none'}.\n")
        emit("### Non-Swift files\n")
        emit("| File | Bytes | Authors in this tree | Origin | Licence as read | Needs |")
        emit("|---|---|---|---|---|---|")
        for entry in swift["files"]:
            if entry["kind"] == "swift":
                continue
            origin = asset_origin(entry["path"])
            flag = " (dev only)" if entry["path"] in swift["dev_only"] else ""
            if origin:
                emit(f"| `{entry['path']}`{flag} | {entry['bytes']} | {holders_cell(entry['lines'])} | {origin[0]} | {origin[1]} | {origin[2]} |")
            else:
                emit(f"| `{entry['path']}`{flag} | {entry['bytes']} | {holders_cell(entry['lines'])} | this tree | GPL-3.0 (`LICENSE`), or uncertain | {needs_cell(entry['needs'])} |")
        emit("")
        if swift["copyright_hits"]:
            emit("Copyright or licence wording inside member files:\n")
            for path, hits in sorted(swift["copyright_hits"].items()):
                emit(f"- `{path}`: {'; '.join(f'`{h}`' for h in hits)}")
            emit("")
        emit("### Swift files\n")
        emit("Licence as read for all of them: `LICENSE` is GPL-3.0, the README says MIT, and no file has a header. "
             "Treat as uncertain.\n")
        emit("<details><summary>Every Swift file</summary>\n")
        emit("| File | Lines by blame | Needs |")
        emit("|---|---|---|")
        for entry in swift["files"]:
            if entry["kind"] == "swift":
                emit(f"| `{entry['path']}` | {holders_cell(entry['lines'])} | {needs_cell(entry['needs'])} |")
        emit("\n</details>\n")
        emit("### Excluded from the iOS target\n")
        emit(", ".join(f"`{p}`" for p in swift["excluded"]) or "none")
        emit(f"\n\nOther targets, not in the iOS binary: {', '.join(swift['other_targets'])}.\n")

    emit("## 6. Contributors beyond Weeb Labs and the owner\n")
    emit("Whole history of each tree. \"Surviving\" counts lines blame still gives them in the files listed.\n")
    emit("| Tree | Person | Commits | In the iOS build? | Surviving lines by file |")
    emit("|---|---|---|---|---|")
    for row in data["contributors"]:
        emit(f"| {row['tree']} | {row['person']} | {row['commits']} | {row['in_ios']} | {row['detail']} |")
    emit("")
    trailers = data["trailers"]
    humans = {k: v for k, v in trailers["humans"].items()}
    emit("**Other human names in `Co-authored-by` or `Signed-off-by` trailers:** "
         + (", ".join(f"{k} ({v})" for k, v in sorted(humans.items())) or "none") + ".\n")
    emit("**Tool trailers (a Claude `Co-Authored-By`), commits per committer's holder:** "
         + ("; ".join(f"{tree}: " + ", ".join(f"{k} {v}" for k, v in sorted(c.items())) for tree, c in trailers["tools"].items())) + ". "
         "Listed as a fact; these are not counted as contributors here.\n")
    if fw.get("images"):
        image = fw["images"]
        emit("### Firmware behind the `.uf2` images\n")
        emit(f"Tag `{FIRMWARE_TAG}` (`{image['commit']}`), `firmware/DSPi` without LUFA: {image['files']} files, "
             f"blame by holder {holders_cell(image['lines'])}. The images were not rebuilt and compared with this tag. "
             "Third-party licence files in the tree: "
             + ", ".join(f"`{p}`" for p in image["licence_files"]) + "; `firmware/pico-sdk` is a submodule with its own.\n")
        for who, files in sorted(image["outside"].items()):
            emit(f"- **{who}:** " + ", ".join(f"`{Path(p).name}` ({n})" for p, n in sorted(files.items())))
        emit("")
        if image["copyright"]:
            emit("Copyright wording not naming Weeb Labs in those files (first 20): "
                 + "; ".join(f"`{c}`" for c in image["copyright"]) + "\n")

    # -- WP WB7 --
    web = data.get("web")
    if web:
        emit("## 6b. The web console's JavaScript\n")
        emit("What `apps/web`'s dist bundles besides its Rust core (the lockfile's packages that are not dev-only). "
             "`dist/THIRD-PARTY-NOTICES.txt` carries their licence texts and the core's crates "
             "(`apps/web/notices/rust-crates.txt`, `apps/web/tools/gen_rust_notices.py`).\n")
        emit("| Package | Version | Licence | Class |")
        emit("|---|---|---|---|")
        for name, pkg in sorted(web["packages"].items()):
            emit(f"| `{name}` | {pkg['version']} | {pkg['license'] or 'none given'} | {classify_licence(pkg['license'])} |")
        emit("")
        hub = web.get("hub_page")
        if hub:
            emit(f"**`uconsole hub`'s page at `/hub/`** is upstream's DSPi Link web page, built unmodified and compiled into the hub "
                 f"(`crates/dspi-hub/assets/web/hub/`, `PROVENANCE.json`): {hub['repository']} at `{hub['commit'][:8]}`, "
                 f"`@dspi/web` {hub['version']}, **{hub['license']}** (its `LICENSE`, served in the hub's `/THIRD-PARTY-NOTICES.txt`). "
                 f"Bundled: {', '.join(f'`{p}`' for p in hub['packages'])}; `@dspi/web` declares no licence field, the repository's "
                 "`LICENSE` covers it. Not in the iOS build.\n")

    emit("## 7. Flags\n")
    for flag in data["flags"]:
        emit(f"- {flag}")
    emit("")
    if NOTES_MD.exists():
        emit(NOTES_MD.read_text().rstrip() + "\n")
    emit("## How to re-run\n")
    emit("```sh\n"
         "ci/licence-inventory.py --write --fetch-console   # after each SW wave; or --console <tree>\n"
         "ci/licence-inventory.py --check --fetch-console   # any difference exits 1\n"
         "ci/check.sh --step licences                       # the gate: --gate --fetch-console\n"
         "ci/check.sh --step deny                           # cargo-deny, whole workspace\n"
         "```\n")
    emit("After MR1's Swift half the script finds `apps/apple` by itself.")
    return "\n".join(out) + "\n"


def build_full(args, snap: dict) -> dict:
    cargo = snap["_cargo"]
    first, third = [], []
    overlap: dict[str, int] = {}
    copyright_hits: dict[str, list[str]] = {}
    waived_hits: dict[str, list[str]] = {}
    dirs_files = cargo["files"]
    for crate in sorted(cargo["linked"].values(), key=lambda c: c["name"]):
        if not crate["first_party"]:
            third.append({k: crate[k] for k in ("name", "version", "license", "repository")}
                         | {"class": classify_licence(crate["license"])})
            continue
        paths = sorted(set(dirs_files.get(crate["name"].replace("-", "_"), []) + dirs_files.get(crate["name"], [])))
        files = describe_files(ROOT, "HEAD", paths)
        for entry in files:
            if entry.get("copyright"):
                copyright_hits[entry["path"]] = entry["copyright"]
            if entry.get("waived"):
                waived_hits[entry["path"]] = entry["waived"]
        totals: Counter = Counter()
        for entry in files:
            totals.update(entry["lines"])
        extra, origin = CRATE_ORIGINS.get(crate["name"], ([], "No origin recorded: read its history."))
        needs = sorted((set(totals) | set(extra)) - {UNCOMMITTED})
        if crate["name"] == "dspi-core":
            overlap.update(old_core_overlap(paths))
        first.append({"name": crate["name"], "version": crate["version"], "license": crate["license"],
                      "files": files, "needs": needs, "origin": origin})

    fw_tree_env = os.environ.get("DSPI_FIRMWARE_TREE", str(Path.home() / "prog/DSPi"))
    fw_tree = Path(fw_tree_env).expanduser()
    fw_tree = fw_tree if (fw_tree / ".git").exists() else None
    firmware: dict = {}
    build_inputs = [p for v in cargo["build_inputs"].values() for p in v]
    headers = [p for p in build_inputs if "/firmware/" in p and p.endswith(".h")]
    if fw_tree and headers:
        prov = (ROOT / "crates/dspi-proto/firmware/PROVENANCE.toml").read_text()
        commit = re.search(r'^commit\s*=\s*"([0-9a-f]+)"', prov, re.M).group(1)
        paths = [f"firmware/DSPi/{Path(p).name}" for p in headers]
        firmware["headers"] = firmware_report(fw_tree, commit, "firmware/DSPi", paths)
        firmware["headers"]["branch"] = re.search(r'^branch\s*=\s*"([^"]+)"', prov, re.M).group(1)

    swift_out = None
    contributors: list[dict] = []
    trailers = {"humans": Counter(), "tools": {}}
    flags: list[str] = []

    def add_trailers(tree: str, repo: Path, ref: str):
        found = trailer_people(repo, ref)
        trailers["humans"].update(found["humans"])
        if found["tools"]:
            trailers["tools"][tree] = dict(found["tools"])

    repo_outside = outside_contributors(ROOT, "HEAD")
    if not repo_outside:
        contributors.append({"tree": "`dspi-unified-console` (the Rust core)", "person": "none", "commits": 0,
                             "in_ios": "n/a", "detail": "only Weeb Labs and the owner"})
    for who, rec in sorted(repo_outside.items()):
        contributors.append({"tree": "`dspi-unified-console`", "person": who, "commits": rec["commits"],
                             "in_ios": "see section 3", "detail": ", ".join(f"`{p}`" for p in rec["paths"][:20])})
    add_trailers("dspi-unified-console", ROOT, "HEAD")

    if snap.get("_swift"):
        swift = snap["_swift"]
        repo, prefix, ref = snap["_console"]
        described = describe_files(repo, ref, [prefix + p for p in swift["members"]])
        sw_copyright: dict[str, list[str]] = {}
        foreign: set[str] = set()
        files = []
        for entry, rel in zip(described, swift["members"]):
            entry["path"] = rel
            entry["kind"] = swift_kind(rel)
            entry["needs"] = sorted(set(entry["lines"]) - {UNCOMMITTED})
            if entry.get("copyright") and entry["kind"] != "asset":
                sw_copyright[rel] = entry["copyright"]
            if entry["kind"] == "swift":
                text = git(repo, "show", f"{ref}:{prefix}{rel}")
                foreign.update(set(SWIFT_IMPORT.findall(text)) - APPLE_MODULES)
            files.append(entry)
        members = set(swift["members"])
        for who, rec in sorted(outside_contributors(repo, ref).items()):
            surviving = {e["path"]: e["lines"][who] for e in files if who in e["lines"] and e["by"] == "blame"}
            all_swift = [p for p in rec["paths"] if p[len(prefix):] in members]
            detail = ", ".join(f"`{Path(p).name}` ({n})" for p, n in sorted(surviving.items())) or "none in iOS members"
            contributors.append({"tree": "Console fork (Swift)", "person": who, "commits": rec["commits"],
                                 "in_ios": "**yes**" if surviving else ("touched, no surviving line" if all_swift else "no"),
                                 "detail": detail + (f"; touched {len(rec['paths'])} paths in total" if rec["paths"] else "")})
        add_trailers("Console fork", repo, ref)
        swift_out = {
            "source": f"{git(repo, 'remote', 'get-url', 'origin', check=False).strip() or repo.name} {CONSOLE_BRANCH if ref != 'HEAD' else 'HEAD'}",
            "commit": snap["swift"]["commit"],
            "files": files,
            "excluded": swift["excluded"],
            "dev_only": swift["dev_only"],
            "products": swift["products"],
            "frameworks": swift["frameworks"],
            "other_targets": swift["other_targets"],
            "project_packages": swift["project_packages"],
            "foreign_imports": sorted(foreign),
            "copyright_hits": sw_copyright,
        }
        if any(e["kind"] == "firmware-image" for e in files):
            firmware["images"] = firmware_report(fw_tree, FIRMWARE_TAG, "firmware/DSPi")

    header_rep = firmware.get("headers")
    if header_rep:
        for who, count in sorted(header_rep["commit_authors"].items()):
            lines = header_rep["outside"].get(who, {})
            contributors.append({"tree": "Firmware, headers behind `dspi-proto`", "person": who, "commits": count,
                                 "in_ios": "**as generated constants**" if lines else "no surviving line in these headers",
                                 "detail": ", ".join(f"`{Path(p).name}` ({n})" for p, n in sorted(lines.items())) or "none"})
    image_rep = firmware.get("images")
    if image_rep:
        for who, count in sorted(image_rep["commit_authors"].items()):
            lines = image_rep["outside"].get(who, {})
            contributors.append({"tree": "Firmware, `.uf2` images", "person": who, "commits": count,
                                 "in_ios": "**yes, until SW18**" if lines else "no surviving line in `firmware/DSPi`",
                                 "detail": ", ".join(f"`{Path(p).name}` ({n})" for p, n in sorted(lines.items())) or "none"})

    for crate in third:
        if crate["class"] in ("copyleft", "unknown"):
            flags.append(f"**{crate['name']} {crate['version']}**: licence `{crate['license'] or 'none'}`, class {crate['class']}.")
    mpl = [c["name"] for c in third if c["class"] == "mpl"]
    if mpl:
        flags.append(f"**MPL-2.0 in the binary:** {', '.join(mpl)}. File-level copyleft, not GPL; the plan's table says nothing here blocks a route. "
                     "The owner and Weeb Labs should still read its source-availability and notice terms against the store build. "
                     "UniFFI's generated Swift (SW2) will also come from its templates.")
    if any(c["name"] == "nusb" for c in third):
        flags.append("**`nusb` is in the iOS graph and does not compile for iOS.** CG3's `usb` feature keeps it out; "
                     "something has turned that feature on again (`ci/check.sh --step ios-check` fails too).")
    names = {c["name"] for c in third}
    if {"cargo_metadata", "uniffi_pipeline"} & names:
        flags.append("**Build-tool crates in the graph through `uniffi`:** `cargo_metadata`, `camino`, `semver` and `cargo-platform` come with its "
                     "default `cargo-metadata` feature, which SW1 could turn off. `uniffi_pipeline` (MPL-2.0), `tempfile`, `rustix`, `fastrand`, `indexmap` and "
                     "`hashbrown` come with a fixed dependency. They are listed because the graph has them; the linker may drop what is unused.")
    if swift_out and any(e["kind"] == "firmware-image" for e in swift_out["files"]):
        names = ", ".join(f"`{Path(e['path']).name}`" for e in swift_out["files"] if e["kind"] == "firmware-image")
        flags.append(f"**GPL firmware images are in the iOS target:** {names}. Confirmed from `project.pbxproj`: the exception list does not name them. SW18 removes them.")
    if swift_out and swift_out["foreign_imports"]:
        flags.append(f"**Non-Apple imports in iOS members:** {', '.join(swift_out['foreign_imports'])}. They may sit behind `#if`; read them.")
    if not snap.get("_swift"):
        flags.append("**Swift half not inventoried:** no Console tree was available.")
    if fw_tree is None:
        flags.append("**Firmware not inspected:** no firmware tree at `$DSPI_FIRMWARE_TREE` or `~/prog/DSPi`.")

    return {
        "repo_commit": git(ROOT, "rev-parse", "--short", "HEAD").strip(),
        "rust": {
            "first_party": first,
            "third_party": third,
            "compile_only": sorted(({k: c[k] for k in ("name", "version", "license")} for c in cargo["compile_only"].values()),
                                   key=lambda c: c["name"]),
            "overlap": overlap,
            "old_core_overlap_files": sum(1 for v in overlap.values() if v > 0),
            "old_core_overlap_pct": old_core_weighted(first, overlap),
            "gated_modules": platform_gated_modules(cargo["files"]),
            "copyright_hits": copyright_hits,
            "waived_hits": waived_hits,
        },
        "swift": swift_out,
        # -- WP WB7 --
        "web": snap["web"],
        "firmware": firmware,
        "contributors": contributors,
        "trailers": {"humans": dict(trailers["humans"]), "tools": trailers["tools"]},
        "flags": flags,
    }


def old_core_weighted(first: list[dict], overlap: dict[str, int]) -> int:
    weighted = total = 0
    for crate in first:
        for entry in crate["files"]:
            if entry["path"] in overlap:
                count = sum(entry["lines"].values())
                weighted += count * overlap[entry["path"]]
                total += count
    return round(weighted / total) if total else 0


def public_snapshot(snap: dict) -> dict:
    return {k: v for k, v in snap.items() if not k.startswith("_")}


def print_about_state() -> None:
    """Whether the About wording is approved. Only printed, never recorded, so that
    approving it stays a change to assets/about.json alone."""
    about = json.loads((ROOT / "assets/about.json").read_text())
    state = "DRAFT, not approved by the owner" if about["draft"] else "approved by the owner"
    print(f"licence-inventory: assets/about.json wording: {state}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--gate", action="store_true")
    parser.add_argument("--console", help="a clone of the Console (default: see the module help)")
    parser.add_argument("--console-ref", help=f"its ref (default {CONSOLE_BRANCH})")
    parser.add_argument("--fetch-console", action="store_true", help="clone the fork if no tree is at hand")
    args = parser.parse_args()
    print_about_state()

    snap = light_snapshot(args)
    if args.write:
        if snap["swift"] is None:
            print("licence-inventory: writing without the Swift half; use --console or --fetch-console", file=sys.stderr)
        full = build_full(args, snap)
        OUT_DIR.mkdir(parents=True, exist_ok=True)
        OUT_MD.write_text(render(full))
        record = {"schema": 1, "repo_commit": full["repo_commit"], **public_snapshot(snap)}
        OUT_JSON.write_text(json.dumps(record, indent=1, sort_keys=True) + "\n")
        print(f"wrote {OUT_MD.relative_to(ROOT)} and {OUT_JSON.relative_to(ROOT)}")
        return 0

    if not OUT_JSON.exists():
        print("licence-inventory: no recorded inventory; run --write", file=sys.stderr)
        return 1
    hard, soft = compare(json.loads(OUT_JSON.read_text()), public_snapshot(snap))
    for line in hard:
        print(f"CHANGED  {line}")
    for line in soft:
        print(f"{'NOTE    ' if args.gate else 'CHANGED '} {line}")
    failing = hard if args.gate else hard + soft
    if failing:
        print(f"licence-inventory: {len(failing)} difference(s) from docs/licensing/. "
              "Read them, then run ci/licence-inventory.py --write and commit the result.", file=sys.stderr)
        return 1
    print(f"licence-inventory: matches the record ({len(soft)} note(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
