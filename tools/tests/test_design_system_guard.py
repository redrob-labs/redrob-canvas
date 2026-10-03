# SPDX-License-Identifier: GPL-3.0-or-later
"""Prove the design system guard fails when the thing it guards is broken.

A guard that has never failed is a guard nobody has tested, and a green suite is exactly what a broken
guard produces. Each defect is put back, the guard is run, and the file is restored from bytes held in
memory. Not from git: these files may be uncommitted while the change is being made, and a checkout
would delete the work rather than restore it.
"""
from __future__ import annotations

from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
GUARD = [sys.executable, str(ROOT / "tools/check_design_system.py")]


def run_guard() -> subprocess.CompletedProcess[str]:
    return subprocess.run(GUARD, cwd=ROOT, capture_output=True, text=True, encoding="utf-8")


class DesignSystemGuard(unittest.TestCase):
    def test_clean_tree_passes(self) -> None:
        self.assertEqual(run_guard().returncode, 0, run_guard().stderr)

    def _with_defect(self, relative: str, mutate, expected: str) -> None:
        target = ROOT / relative
        original = target.read_bytes()
        try:
            target.write_bytes(mutate(original))
            result = run_guard()
            self.assertNotEqual(result.returncode, 0,
                                f"guard passed with a defect in {relative}")
            self.assertIn(expected, result.stderr)
        finally:
            target.write_bytes(original)
        self.assertEqual(run_guard().returncode, 0, "tree did not restore cleanly")

    def test_rejects_a_colour_literal_in_qml(self) -> None:
        self._with_defect(
            "qml/Main.qml",
            lambda b: b.replace(b"color: window.tokens.surfaceBase",
                                b'color: "#17191d"', 1),
            "colour literal",
        )

    def test_rejects_a_hand_edited_token_file(self) -> None:
        self._with_defect(
            "qml/RedrobTokens.qml",
            lambda b: b.replace(b'readonly property color surfaceBase',
                                b'readonly property color surfaceBaseX', 1),
            "generate_design_tokens.py --check failed",
        )

    def test_rejects_a_drifted_vendored_token_source(self) -> None:
        self._with_defect(
            "third_party/redrob-ui/tokens.json",
            lambda b: b.replace(b'"#0a0b0c"', b'"#0a0b0d"', 1),
            "does not match its pin",
        )

    def test_rejects_a_tool_button_without_a_glyph(self) -> None:
        # The rail went back to a letter once before; this is that defect.
        self._with_defect(
            "qml/Main.qml",
            lambda b: b.replace(b'iconName: "brush"', b'text: "B"', 1),
            "ToolRailButton without iconName",
        )

    def test_rejects_a_command_button_without_a_glyph(self) -> None:
        self._with_defect(
            "qml/Main.qml",
            lambda b: b.replace(b'iconName: "zoomIn"', b'// icon removed', 1),
            "CommandButton without iconName",
        )

    def test_rejects_a_hand_edited_icon(self) -> None:
        self._with_defect(
            "resources/icons/redrob-canvas.svg",
            lambda b: b.replace(b'fill="#C162F4"', b'fill="#FF0000"', 1),
            "build_app_icon.py --check failed",
        )

    def test_rejects_an_ungenerated_icon_file(self) -> None:
        stray = ROOT / "resources/icons/redrob.svg"
        self.assertFalse(stray.exists(), "the hand-drawn mark is back in the tree")
        stray.write_text('<svg xmlns="http://www.w3.org/2000/svg"><text>R</text></svg>\n', encoding="utf-8", newline="\n")
        try:
            result = run_guard()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("is not generated from the design system", result.stderr)
        finally:
            stray.unlink()
        self.assertEqual(run_guard().returncode, 0)


if __name__ == "__main__":
    unittest.main()
