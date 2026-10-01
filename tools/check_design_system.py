#!/usr/bin/env python3
"""Guard the design system against the ways it drifts back, each of which a build happily passes.

  1. A colour literal in qml/Main.qml. Once one exists the next edit copies it, and that colour stops
     following the theme, because a literal cannot retint. Seven literals are ALLOWED and named below:
     they are the user's document, not this app's chrome.
  2. A drifted vendored token file. tools/generate_design_tokens.py checks the pin, and this calls it.
  3. A generated file edited by hand. Both generators have a --check mode; this runs both.
  4. A hand-drawn brand mark coming back. The old resources/icons/redrob.svg was a typed letter R in an
     invented gradient, which 10-logo.md forbids; nothing in a build would reject it.

Reverse-verified by tools/tests/test_design_system_guard.py, which puts each defect back and confirms
this script fails on it.
"""
from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MAIN = ROOT / "qml/Main.qml"
LITERAL = re.compile(r'"#[0-9a-fA-F]{3,8}"')

# The user's DOCUMENT, not this app's chrome: the default gradient a drawing is filled with, the matte
# an export is flattened onto, and the hex a colour field shows for editing. Tokenising these would
# change what the user's artwork looks like, and make a colour field display a name instead of a value.
ALLOWED = {
    "property color gradientStartColor",
    "property color gradientEndColor",
    "property color exportMatte",
    'text: "#ff000000"',
    'text: "#ff5378dc"',
    'text: "#ff20242a"',
    'vectorStrokeColor.text = "#ff20242a"',
}

failures: list[str] = []

for number, line in enumerate(MAIN.read_text().split("\n"), start=1):
    hits = LITERAL.findall(line)
    if not hits:
        continue
    if any(marker in line for marker in ALLOWED):
        continue
    for hit in hits:
        failures.append(
            f"qml/Main.qml:{number} colour literal {hit} -- bind it to window.tokens.* instead.\n"
            f"    If it is the user's document rather than this app's chrome, add the line to ALLOWED"
            f" in {pathlib.Path(__file__).name} and say why."
        )

for generator in ("tools/generate_design_tokens.py", "tools/build_app_icon.py"):
    result = subprocess.run(
        [sys.executable, str(ROOT / generator), "--check"], cwd=ROOT, capture_output=True, text=True
    )
    if result.returncode != 0:
        failures.append(f"{generator} --check failed:\n    " + result.stderr.strip().replace("\n", "\n    "))

# The mark is generated from the delivery's own master. A file that is not one of the generated pair is
# a hand-drawn mark finding its way back in.
pin = json.loads((ROOT / "DESIGN_SYSTEM_PIN.json").read_text())
expected = {pathlib.Path(entry["file"]).name for entry in pin["generated"]
            if entry["file"].startswith("resources/icons/")}
present = {p.name for p in (ROOT / "resources/icons").iterdir() if p.is_file()}
for extra in sorted(present - expected):
    failures.append(
        f"resources/icons/{extra} is not generated from the design system.\n"
        f"    10-logo.md: the artwork is never redrawn, restretched, recoloured or otherwise modified."
    )

if failures:
    print(f"design system guard: {len(failures)} problem(s)\n", file=sys.stderr)
    for failure in failures:
        print("  " + failure, file=sys.stderr)
    sys.exit(1)
print("design system guard: clean")
