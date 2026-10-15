#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Guard the design system against the ways it drifts back, each of which a build happily passes.

  1. A drifted vendored token or icon file. The delivery is pinned by sha256 in DESIGN_SYSTEM_PIN.json;
     tools/generate_design_tokens.py --check verifies every pin, and this calls it.
  2. A generated brand file edited by hand. tools/build_app_icon.py and tools/build_lockup.py have a
     --check mode that rebuilds the mark and the lockups and compares hashes; this runs both.
  3. A hand-drawn brand mark coming back. An earlier resources/icons/redrob.svg was a typed letter R in
     an invented gradient, which 10-logo.md forbids; nothing in a build would reject it, so every file
     under resources/icons/ must be one DESIGN_SYSTEM_PIN.json lists as generated.
  4. A colour literal in the shell's own chrome. The egui shell paints from the vendored tokens; a
     hex literal in apps/redrob-canvas/src/**/*.rs stops following the theme. Allowed exceptions are
     named in ALLOWED below and must be the user's document, never chrome.

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
SHELL = ROOT / "apps/redrob-canvas/src"
LITERAL = re.compile(r'"#[0-9a-fA-F]{6,8}"')

# The user's DOCUMENT, not this app's chrome. A line containing one of these markers may carry a
# colour literal; add a line here only with a reason beside it.
ALLOWED = {
    "// design-guard: document colour",
}

failures: list[str] = []

if SHELL.is_dir():
    for source in sorted(SHELL.rglob("*.rs")):
        for number, line in enumerate(source.read_text(encoding="utf-8").split("\n"), start=1):
            if any(marker in line for marker in ALLOWED):
                continue
            for hit in LITERAL.findall(line):
                failures.append(
                    f"{source.relative_to(ROOT)}:{number} colour literal {hit} -- take the value from the"
                    f" vendored tokens (third_party/redrob-ui/tokens.json) instead.\n"
                    f"    If it is the user's document rather than this app's chrome, end the line with"
                    f" `// design-guard: document colour` and say why in a comment above."
                )

for generator in ("tools/generate_design_tokens.py", "tools/build_app_icon.py", "tools/build_lockup.py"):
    result = subprocess.run(
        [sys.executable, str(ROOT / generator), "--check"], cwd=ROOT, capture_output=True, text=True
    )
    if result.returncode != 0:
        failures.append(f"{generator} --check failed:\n    " + result.stderr.strip().replace("\n", "\n    "))

# The mark is generated from the delivery's own master. A file that is not one of the generated set is
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

if failures:
    print(f"design system guard: {len(failures)} problem(s)\n", file=sys.stderr)
    for failure in failures:
        print("  " + failure, file=sys.stderr)
    sys.exit(1)
print("design system guard: clean")
