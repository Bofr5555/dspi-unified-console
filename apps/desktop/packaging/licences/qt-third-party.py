#!/usr/bin/env python3
# -- WP QD42 -- Qt's licence files and third-party notices, copied from Qt's
# source archives (release-sources.sh's qt/<module>-everywhere-src-<v>.tar.xz):
#
#   qt-third-party.py <out dir> <module archive>...
#
# Writes into <out dir> the files of each module's LICENSES/ folder, verbatim
# (when two modules have different texts under one name, the later module's is
# <name>.<module>.txt), and THIRD-PARTY.txt: every
# component a qt_attribution.json under the modules' src/ names (Qt's own
# record of the third-party code in it), with the licence files it names,
# verbatim. Test-only code (src/testlib, src/testinternal) and the Android
# build files (src/3rdparty/gradle) are left out; the rest is listed whether or
# not a given platform's build compiles it in. Reads the archives directly;
# nothing is unpacked. Standard library only.
import json
import os
import posixpath
import sys
import tarfile

SKIP = ("src/testlib/", "src/testinternal/", "src/3rdparty/gradle/")


def main():
    out, archives = sys.argv[1], sys.argv[2:]
    licences = {}
    renamed = {}
    entries = []
    names = []
    for path in archives:
        names.append(os.path.basename(path))
        with tarfile.open(path) as tar:
            members = {m.name: m for m in tar.getmembers() if m.isfile()}
            root = next(iter(members)).split("/", 1)[0]
            module = root.split("-everywhere-src-", 1)[0]

            def read(name):
                return tar.extractfile(members[name]).read()

            for name in sorted(members):
                rel = name[len(root) + 1:]
                if rel.startswith("LICENSES/") and rel.count("/") == 1:
                    text = read(name)
                    base = rel.split("/", 1)[1]
                    if base in licences and licences[base] != text:
                        base = f"{base[:-4]}.{module}.txt"
                    licences[base] = text
                    renamed[(module, rel.split("/", 1)[1])] = base
                elif (rel.startswith("src/") and rel.endswith("/qt_attribution.json")
                      and not rel.startswith(SKIP)):
                    data = json.loads(read(name), strict=False)
                    for e in data if isinstance(data, list) else [data]:
                        files = e.get("LicenseFiles") or ([e["LicenseFile"]] if e.get("LicenseFile") else [])
                        texts = []
                        for f in files:
                            full = posixpath.normpath(posixpath.join(posixpath.dirname(name), f))
                            if full.startswith(root + "/LICENSES/"):
                                b = posixpath.basename(full)
                                texts.append((renamed.get((module, b), b), None))
                            elif full in members:
                                texts.append((f"{module}/{full[len(root) + 1:]}", read(full)))
                            else:
                                sys.exit(f"qt-third-party: {name} names {f}, not in the archive")
                        entries.append((e.get("Name", e.get("Id", "?")), module, rel, e, texts))

    os.makedirs(out, exist_ok=True)
    for base, text in sorted(licences.items()):
        with open(os.path.join(out, base), "wb") as f:
            f.write(text)
    entries.sort(key=lambda x: (x[0].lower(), x[1], x[2]))
    with open(os.path.join(out, "THIRD-PARTY.txt"), "wb") as f:
        w = lambda s="": f.write((s + "\n").encode())
        w("Third-party components in Qt, as Qt's source records them (qt_attribution.json),")
        w("with the licence texts they name. From: " + ", ".join(names) + ".")
        w("Qt's own licence files (from each module's LICENSES/), and the licence texts named below by")
        w("file name alone, are in the same folder as this file.")
        for name, module, rel, e, texts in entries:
            w()
            w("=" * 78)
            w(f"{name} {e.get('Version', '')}".rstrip())
            w(f"In: {module}/{posixpath.dirname(rel)}")
            if e.get("License") or e.get("LicenseId"):
                w(f"Licence: {e.get('License', '')} ({e.get('LicenseId', '')})")
            cr = e.get("Copyright", [])
            for line in (cr if isinstance(cr, list) else cr.splitlines()):
                w(line)
            if e.get("Homepage"):
                w(f"Homepage: {e['Homepage']}")
            for label, text in texts:
                if text is None:
                    w(f"Licence text: {label}")
                else:
                    w(f"---- {label}")
                    f.write(text if text.endswith(b"\n") else text + b"\n")
    print(f"qt-third-party: {len(entries)} components, {len(licences)} licence files in {out}")


if __name__ == "__main__":
    main()
