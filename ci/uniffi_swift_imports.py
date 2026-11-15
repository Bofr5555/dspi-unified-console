#!/usr/bin/env python3
"""WP SW-M3: give UniFFI's Swift the imports of the namespaces whose types it uses.

uniffi-bindgen 0.29's Swift calls another namespace's public converters when one
namespace's API takes another's type (`dspi_hub`'s `attach_app` takes `dspi_core`'s
`DspiCore`), and expects both to be compiled as one module. The app keeps each
namespace a module of its own (Core/Generated/Package.swift), so the file needs
`import <that namespace>`. The namespaces it uses are the ones whose
`uniffiEnsure<Name>Initialized()` it calls without defining: `DspiCore` is
`dspi_core`.

    ci/uniffi_swift_imports.py FILE.swift...

Rewrites each file in place, once: an import already there is not added again. Run
by every step that generates the Swift (ci/drift.sh, ci/compile-bindings.sh,
apps/apple/build-xcframework.sh), so the output stays a pure function of the sources.
"""
import re
import sys

ENSURE = re.compile(r"\buniffiEnsure([A-Za-z0-9]+)Initialized\(\)")
DEFINED = re.compile(r"\bfunc uniffiEnsure([A-Za-z0-9]+)Initialized\(\)")


def module_of(camel: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", camel).lower()


def fix(path: str) -> None:
    with open(path, encoding="utf-8") as f:
        text = f.read()
    own = set(DEFINED.findall(text))
    used = sorted({module_of(n) for n in ENSURE.findall(text) if n not in own})
    lines = [f"import {m}" for m in used if not re.search(rf"^import {m}$", text, re.M)]
    if not lines:
        return
    anchor = "import Foundation\n"
    if anchor not in text:
        sys.exit(f"{path}: no `import Foundation` to add the imports after")
    added = anchor + "// The namespaces whose types this one uses (ci/uniffi_swift_imports.py).\n"
    added += "".join(line + "\n" for line in lines)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text.replace(anchor, added, 1))


for p in sys.argv[1:]:
    fix(p)
