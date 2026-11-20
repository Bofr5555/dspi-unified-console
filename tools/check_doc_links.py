#!/usr/bin/env python3
# -- WP DOCS14 -- Checks the relative links of the repository's Markdown pages: every
# [text](path) and ![alt](path) that is not a URL must name a file or folder that exists, and
# a #fragment must name a heading of the page it points to (GitHub's anchor rule). Standard
# library only, no network.
#
#   python3 tools/check_doc_links.py [files or folders...]   (default: the whole tree)
#
# Skipped: node_modules, build output, and apps/apple: the Swift app is deferred by the
# owner (2026-11-19) and its README's Images/ were never imported (WP DOCS14's report).
# Exit 1 on any broken link, with one line each.
import os
import re
import sys
import unicodedata

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKIP_DIRS = {".git", "node_modules", "target", "build", "dist", ".gradle", "_ref", "Generated"}
SKIP_PATHS = {os.path.join(ROOT, "apps", "apple")}
LINK = re.compile(r"!?\[(?:[^\]\[]|\[[^\]]*\])*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
FENCE = re.compile(r"^\s*(```|~~~)")


def md_files(paths):
    for p in paths:
        if os.path.isfile(p):
            yield p
            continue
        for d, dirs, files in os.walk(p):
            dirs[:] = [x for x in dirs if x not in SKIP_DIRS and os.path.join(d, x) not in SKIP_PATHS]
            for f in files:
                if f.endswith(".md"):
                    yield os.path.join(d, f)


def strip_code(text, inline=True):
    """The text without fenced blocks (and inline code), line count kept."""
    out, fenced = [], False
    for line in text.split("\n"):
        if FENCE.match(line):
            fenced = not fenced
            out.append("")
            continue
        out.append("" if fenced else re.sub(r"`[^`]*`", "", line) if inline else line)
    return out


def anchor(heading):
    """GitHub's anchor of a heading: lower case, punctuation dropped, spaces to dashes."""
    h = re.sub(r"<[^>]+>", "", heading).strip().lower()
    h = re.sub(r"[`*_~]|\[|\]|\(([^)]*)\)", lambda m: m.group(1) or "", h)
    h = "".join(c for c in h if c in " -" or unicodedata.category(c)[0] in "LN")
    return h.replace(" ", "-")


_anchors = {}


def anchors(path):
    if path not in _anchors:
        seen, found = {}, set()
        with open(path, encoding="utf-8") as f:
            for line in strip_code(f.read(), inline=False):
                m = re.match(r"^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$", line)
                if not m:
                    continue
                a = anchor(m.group(1))
                n = seen.get(a, 0)
                seen[a] = n + 1
                found.add(a if n == 0 else f"{a}-{n}")
        with open(path, encoding="utf-8") as f:
            found.update(re.findall(r'<a\s+(?:name|id)="([^"]+)"', f.read()))
        _anchors[path] = found
    return _anchors[path]


def check(path):
    bad = []
    with open(path, encoding="utf-8") as f:
        lines = strip_code(f.read())
    for no, line in enumerate(lines, 1):
        for target in LINK.findall(line):
            if re.match(r"^[a-z][a-z0-9+.-]*:", target, re.I):
                continue  # a URL or mailto:
            file_part, _, frag = target.partition("#")
            dest = os.path.normpath(os.path.join(os.path.dirname(path), file_part)) if file_part else path
            if not os.path.exists(dest):
                bad.append(f"{os.path.relpath(path, ROOT)}:{no}: {target}: no such file")
            elif frag and dest.endswith(".md") and frag.lower() not in anchors(dest):
                bad.append(f"{os.path.relpath(path, ROOT)}:{no}: {target}: no heading #{frag}")
    return bad


def main(argv):
    paths = argv or [ROOT]
    bad, n = [], 0
    for p in md_files(paths):
        n += 1
        bad += check(p)
    for b in bad:
        print(b)
    print(f"check_doc_links: {n} pages, {len(bad)} broken links")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
