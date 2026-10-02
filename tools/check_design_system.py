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

for number, line in enumerate(MAIN.read_text(encoding="utf-8").split("\n"), start=1):
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
pin = json.loads((ROOT / "DESIGN_SYSTEM_PIN.json").read_text(encoding="utf-8"))
expected = {pathlib.Path(entry["file"]).name for entry in pin["generated"]
            if entry["file"].startswith("resources/icons/")}
present = {p.name for p in (ROOT / "resources/icons").iterdir() if p.is_file()}
for extra in sorted(present - expected):
    failures.append(
        f"resources/icons/{extra} is not generated from the design system.\n"
        f"    10-logo.md: the artwork is never redrawn, restretched, recoloured or otherwise modified."
    )

# 5. A tool or command button without a design-system glyph. The rail was a column of letters and the
# header a row of words until #48-#51; a new button added the old way would look like neither. Each
# `ToolRailButton {` / `CommandButton {` block must set `iconName:`, unless it is one of the few
# text-only actions named here.
TEXT_ONLY_COMMANDS = {
    'text: "Save As"',  # Sits beside Save, which carries the glyph; a second disk icon reads as a duplicate.
    'text: "1:1"',  # A ratio read as text, like the "100%" beside it; no glyph says "actual pixels" better.
}
BUTTON = re.compile(r"^\s*(ToolRailButton|CommandButton)\s*\{")


def block_text(lines: list[str], start: int) -> str:
    """The text from `start` to the brace closing the block opened there, skipping strings and // comments."""
    depth = 0
    out: list[str] = []
    for line in lines[start:]:
        out.append(line)
        quote = None
        index = 0
        while index < len(line):
            character = line[index]
            if quote:
                if character == "\\":
                    index += 2
                    continue
                if character == quote:
                    quote = None
            elif character in "\"'":
                quote = character
            elif line.startswith("//", index):
                break
            elif character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    return "\n".join(out)
            index += 1
    return "\n".join(out)


main_lines = MAIN.read_text(encoding="utf-8").split("\n")
for number, line in enumerate(main_lines, start=1):
    match = BUTTON.match(line)
    if not match or line.lstrip().startswith("component "):
        continue
    block = block_text(main_lines, number - 1)
    if "iconName:" in block or any(marker in block for marker in TEXT_ONLY_COMMANDS):
        continue
    label = re.search(r'\btext:\s*(.+)', block)
    failures.append(
        f"qml/Main.qml:{number} {match.group(1)} without iconName ({label.group(1).strip() if label else 'no text'}).\n"
        f"    Give it a design-system glyph (third_party/redrob-ui/icons, pinned), or add it to\n"
        f"    TEXT_ONLY_COMMANDS in {pathlib.Path(__file__).name} and say why."
    )

if failures:
    print(f"design system guard: {len(failures)} problem(s)\n", file=sys.stderr)
    for failure in failures:
        print("  " + failure, file=sys.stderr)
    sys.exit(1)
print("design system guard: clean")
