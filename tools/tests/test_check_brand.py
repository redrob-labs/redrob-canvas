# SPDX-License-Identifier: MIT OR Apache-2.0
"""Prove scripts/check-brand.py fails on each defect it exists to catch.

The byte pins catch every edit, so each rule below is exercised on a scratch copy of the pinned tree
whose pins are rewritten to match the defective file: that isolates the rule from the pin.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts/check-brand.py"
SMALL = "resources/icons/redrob-canvas-small.svg"


def copy_tree(dest: Path) -> None:
    (dest / "scripts").mkdir(parents=True)
    shutil.copyfile(CHECK, dest / "scripts/check-brand.py")
    shutil.copytree(ROOT / "brand", dest / "brand")
    pins = json.loads((ROOT / "brand/SOURCES.json").read_text(encoding="utf-8"))
    for rel in pins["outputs"]:
        (dest / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, dest / rel)


def run_check(tree: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(tree / "scripts/check-brand.py")], cwd=tree,
                          capture_output=True, text=True, encoding="utf-8")


def repin(tree: Path, rel: str) -> None:
    pins_path = tree / "brand/SOURCES.json"
    pins = json.loads(pins_path.read_text(encoding="utf-8"))
    pins["outputs"][rel] = hashlib.sha256((tree / rel).read_bytes()).hexdigest()
    pins_path.write_text(json.dumps(pins, indent=2) + "\n", encoding="utf-8")


class CheckBrand(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(dir=os.environ.get("KIROCREW_SCRATCH"))
        self.tree = Path(self._tmp.name) / "tree"
        copy_tree(self.tree)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_pinned_tree_passes(self) -> None:
        result = run_check(self.tree)
        self.assertEqual(result.returncode, 0, result.stderr)

    def _with_svg(self, mutate, expected: str) -> None:
        path = self.tree / SMALL
        path.write_text(mutate(path.read_text(encoding="utf-8")), encoding="utf-8")
        repin(self.tree, SMALL)
        result = run_check(self.tree)
        self.assertNotEqual(result.returncode, 0, "check-brand passed with a defective SVG")
        self.assertIn(expected, result.stderr)

    def test_rejects_gpl_text_in_a_generated_svg(self) -> None:
        self._with_svg(lambda s: "<!-- SPDX-License-Identifier: GPL-3.0-or-later -->\n" + s, "carries GPL text")

    def test_rejects_a_retraced_symbol(self) -> None:
        self._with_svg(lambda s: s.replace('d="M54.4', 'd="M54.5', 1), "not the design system's")

    def test_rejects_an_effect_on_the_mark(self) -> None:
        self._with_svg(lambda s: s.replace("<path ", '<path opacity="0.5" ', 1), "applies an effect")

    def test_rejects_an_unpinned_file(self) -> None:
        (self.tree / "resources/icons/redrob.svg").write_text("<svg/>\n", encoding="utf-8")
        result = run_check(self.tree)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not built from the design system", result.stderr)

    def test_rejects_a_copied_file_that_differs_from_its_source(self) -> None:
        rel = "resources/icons/redrob-canvas.svg"
        path = self.tree / rel
        path.write_bytes(path.read_bytes() + b"\n")
        repin(self.tree, rel)
        result = run_check(self.tree)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not byte-identical", result.stderr)


if __name__ == "__main__":
    unittest.main()
