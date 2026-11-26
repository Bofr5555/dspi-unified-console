#!/usr/bin/env python3
# -- WP CIPAR -- `cargo test`, with its test binaries run side by side.
#
#   python3 ci/par-test.py [--jobs N] -- <cargo test arguments> [-- <test arguments>]
#
# cargo test runs one test binary at a time (each runs its own tests on every
# core). This builds the same binaries (cargo test ... --no-run), then runs
# them, up to N at once (--jobs, else DSPI_TEST_PAR, else 4), the longest
# first by the times of the last run, as cargo would: in the package's
# directory, with cargo's runtime environment (CARGO, CARGO_MANIFEST_DIR,
# CARGO_MANIFEST_PATH, CARGO_PKG_*, CARGO_CRATE_NAME, the deps directory on
# LD_LIBRARY_PATH) and the test arguments. The doc tests run as cargo
# test --doc with the same arguments, beside them. The same binaries, the same
# tests, the same arguments: only the order and the overlap differ.
#
# Every binary runs, failed or not (cargo's --no-fail-fast); each one's output
# is printed whole when it ends, under cargo's own "Running" line; then a
# table of times. Exits 101 if a binary or the doc tests failed, as cargo does.
# Runs in the build container (ci/run.sh): python3, stdlib only.
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor


def split_args(argv):
    jobs = int(os.environ.get("DSPI_TEST_PAR") or 4)
    if argv[:1] == ["--jobs"]:
        jobs, argv = int(argv[1]), argv[2:]
    if argv[:1] != ["--"]:
        sys.exit("usage: par-test.py [--jobs N] -- <cargo test arguments> [-- <test arguments>]")
    argv = argv[1:]
    if "--" in argv:
        i = argv.index("--")
        return max(1, jobs), argv[:i], argv[i + 1:]
    return max(1, jobs), argv, []


def main():
    jobs, cargo_args, test_args = split_args(sys.argv[1:])
    cargo = shutil.which("cargo") or "cargo"
    started = time.monotonic()

    build = subprocess.run([cargo, "test", *cargo_args, "--no-run", "--message-format=json-render-diagnostics"],
                           stdout=subprocess.PIPE, text=True)
    binaries = []
    for line in build.stdout.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") == "compiler-artifact" and m.get("profile", {}).get("test") and m.get("executable"):
            binaries.append(m)
    if build.returncode != 0:
        print(f"par-test: the build failed (exit {build.returncode})", flush=True)
        return 101
    built_s = time.monotonic() - started
    print(f"par-test: {len(binaries)} test binaries built in {built_s:.0f} s; running {jobs} at once", flush=True)

    meta = json.loads(subprocess.run([cargo, "metadata", "--no-deps", "--format-version", "1"],
                                     stdout=subprocess.PIPE, text=True, check=True).stdout)
    packages = {p["manifest_path"]: p for p in meta["packages"]}

    target = os.environ.get("CARGO_TARGET_DIR") or meta["target_directory"]
    # One file per profile (debug, proptest): the same binary takes another time in each
    profile = os.path.basename(os.path.dirname(os.path.dirname(binaries[0]["executable"]))) if binaries else "none"
    times_file = os.path.join(target, f".par-test-times-{profile}.json")
    try:
        with open(times_file) as f:
            last = json.load(f)
    except (OSError, ValueError):
        last = {}

    def key(m):  # the binary's name and cargo's label: "dspi_core unittests src/lib.rs"
        return os.path.basename(m["executable"]).rsplit("-", 1)[0] + " " + label(m)

    def label(m):
        pkg_dir = os.path.dirname(m["manifest_path"])
        src = os.path.relpath(m["target"]["src_path"], pkg_dir)
        kinds = m["target"]["kind"]
        return f"unittests {src}" if ("lib" in kinds or "bin" in kinds or "proc-macro" in kinds) else src

    def env_for(m):
        p = packages[m["manifest_path"]]
        major, minor, rest = (p["version"].split(".", 2) + ["0", "0"])[:3]
        patch, _, pre = rest.partition("-")
        deps = os.path.dirname(m["executable"])
        e = dict(os.environ)
        e.update({
            "CARGO": cargo,
            "CARGO_MANIFEST_DIR": os.path.dirname(m["manifest_path"]),
            "CARGO_MANIFEST_PATH": m["manifest_path"],
            "CARGO_PKG_NAME": p["name"],
            "CARGO_PKG_VERSION": p["version"],
            "CARGO_PKG_VERSION_MAJOR": major,
            "CARGO_PKG_VERSION_MINOR": minor,
            "CARGO_PKG_VERSION_PATCH": patch,
            "CARGO_PKG_VERSION_PRE": pre,
            "CARGO_PKG_AUTHORS": ":".join(p.get("authors") or []),
            "CARGO_PKG_DESCRIPTION": p.get("description") or "",
            "CARGO_PKG_HOMEPAGE": p.get("homepage") or "",
            "CARGO_PKG_REPOSITORY": p.get("repository") or "",
            "CARGO_PKG_LICENSE": p.get("license") or "",
            "CARGO_PKG_LICENSE_FILE": p.get("license_file") or "",
            "CARGO_PKG_RUST_VERSION": p.get("rust_version") or "",
            "CARGO_PKG_README": p.get("readme") or "",
            "CARGO_CRATE_NAME": m["target"]["name"].replace("-", "_"),
            "LD_LIBRARY_PATH": ":".join(x for x in (deps, os.path.dirname(deps), os.environ.get("LD_LIBRARY_PATH")) if x),
        })
        return e

    lock = threading.Lock()
    results = []

    def report(name, title, out, code, secs):
        with lock:
            sys.stdout.write(f"     Running {title}\n{out}")
            if code != 0:
                sys.stdout.write(f"par-test: {name} FAILED (exit {code})\n")
            sys.stdout.flush()
            results.append((name, code, secs))

    def run_binary(m):
        t0 = time.monotonic()
        r = subprocess.run([m["executable"], *test_args], cwd=os.path.dirname(m["manifest_path"]), env=env_for(m),
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        report(key(m), f"{label(m)} ({m['executable']})", r.stdout, r.returncode, time.monotonic() - t0)

    def run_doc():
        t0 = time.monotonic()
        r = subprocess.run([cargo, "test", *cargo_args, "--doc", *(["--", *test_args] if test_args else [])],
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        report("doc-tests", "cargo test --doc", r.stdout, r.returncode, time.monotonic() - t0)

    # The longest first (the last run's times; a new binary, by size, before them)
    binaries.sort(key=lambda m: (key(m) in last, -last.get(key(m), os.path.getsize(m["executable"]))))
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        tasks = [pool.submit(run_doc)] + [pool.submit(run_binary, m) for m in binaries]
        for t in tasks:
            t.result()

    times = dict(last)
    times.update({n: round(s, 1) for n, _, s in results if n != "doc-tests"})
    try:
        with open(times_file + ".new", "w") as f:
            json.dump(times, f, indent=0, sort_keys=True)
        os.replace(times_file + ".new", times_file)
    except OSError as e:
        print(f"par-test: could not save the times: {e}", flush=True)

    failed = [n for n, c, _ in results if c != 0]
    print("\npar-test: the slowest binaries")
    for n, c, s in sorted(results, key=lambda r: -r[2])[:15]:
        print(f"  {s:8.1f} s  {'FAILED ' if c else ''}{n}")
    total = time.monotonic() - started
    print(f"par-test: {len(results)} runs ({len(binaries)} binaries and the doc tests), {len(failed)} failed; "
          f"build {built_s:.0f} s, all {total:.0f} s; serial sum {sum(s for _, _, s in results):.0f} s", flush=True)
    for n in failed:
        print(f"par-test: FAILED {n}")
    return 101 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
