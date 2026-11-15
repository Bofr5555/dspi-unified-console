#!/usr/bin/env python3
"""WP SECFWV: the CI digest check fails on any bundle that differs.

Runs on a small fake tree (no real firmware) under ~/.cache/dspi-build/work, never
tmpfs: python3 tools/test_firmware_manifest.py
"""

import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import firmware_manifest as fm  # noqa: E402

WORK = Path(os.environ.get("DSPI_WORK", Path.home() / ".cache/dspi-build/work"))


def fake_uf2(tag: bytes) -> bytes:
    """Two 512-byte UF2 blocks whose payload holds a version string."""
    out = b""
    for no, payload in enumerate([tag * 8 + b"v1.1.6-1-gabc\x00", tag * 30]):
        head = (0x0A324655).to_bytes(4, "little") + bytes(12) + (256).to_bytes(4, "little")
        head += no.to_bytes(4, "little") + (2).to_bytes(4, "little") + bytes(4)
        out += head + payload.ljust(476, b"\0") + (0x0AB16F30).to_bytes(4, "little")
    return out


IMAGES = {"DSPi-RP2040-v1.1.6.uf2": fake_uf2(b"rp2040"), "DSPi-RP2350-v1.1.6.uf2": fake_uf2(b"rp2350")}


def pin_source() -> str:
    lines = [
        f'        assertEquals(\n            "{fm.hashlib.sha256(b).hexdigest()}",\n'
        f'            sha256(Assets.read("{n}"))\n        )'
        for n, b in IMAGES.items()
    ]
    return "class Uf2ImageTest {\n    @Test fun pins() {\n" + "\n".join(lines) + "\n    }\n}\n"


class WpSecfwvDigestCheck(unittest.TestCase):
    def setUp(self):
        WORK.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix="wp-secfwv-", dir=WORK))
        (self.root / fm.PINS).parent.mkdir(parents=True)
        (self.root / fm.PINS).write_text(pin_source())
        for rel in fm.BUNDLES.values():
            d = self.root / rel
            d.mkdir(parents=True)
            for n, b in IMAGES.items():
                (d / n).write_bytes(b)
        self.assertEqual(fm.main(["--root", str(self.root)]), 0)

    def tearDown(self):
        shutil.rmtree(self.root)

    def run_check(self) -> int:
        return fm.main(["--root", str(self.root), "--check"])

    def test_wp_secfwv_a_consistent_tree_passes(self):
        self.assertEqual(fm.check(self.root), [])
        self.assertEqual(self.run_check(), 0)
        self.assertEqual({e["version"] for e in fm.build(self.root)["images"]}, {"1.1.6-1-gabc"})

    def test_wp_secfwv_a_flipped_byte_in_any_bundle_fails(self):
        for app, rel in fm.BUNDLES.items():
            with self.subTest(app=app):
                path = self.root / rel / "DSPi-RP2350-v1.1.6.uf2"
                good = path.read_bytes()
                bad = bytearray(good)
                bad[len(bad) // 2] ^= 0x01
                path.write_bytes(bytes(bad))
                problems = fm.check(self.root)
                self.assertEqual(len(problems), 1, problems)
                self.assertTrue(problems[0].startswith(f"{app}: "), problems)
                self.assertEqual(self.run_check(), 1)
                path.write_bytes(good)
                self.assertEqual(fm.check(self.root), [])

    def test_wp_secfwv_a_missing_or_extra_image_fails(self):
        for app, rel in fm.BUNDLES.items():
            if app == "android":
                continue
            with self.subTest(app=app):
                path = self.root / rel / "DSPi-RP2040-v1.1.6.uf2"
                good = path.read_bytes()
                path.unlink()
                self.assertTrue(any("is missing" in p for p in fm.check(self.root)))
                path.write_bytes(good)
                extra = self.root / rel / "DSPi-RP2350-v9.9.9.uf2"
                extra.write_bytes(b"x")
                self.assertTrue(any("not in the manifest" in p for p in fm.check(self.root)))
                extra.unlink()
                self.assertEqual(fm.check(self.root), [])

    def test_wp_secfwv_a_stale_manifest_fails(self):
        path = self.root / fm.MANIFEST
        path.write_text(path.read_text().replace('"size": 1024', '"size": 1025'))
        self.assertEqual(fm.check(self.root), [f"{fm.MANIFEST} is not current"])
        self.assertEqual(self.run_check(), 1)


if __name__ == "__main__":
    unittest.main()
