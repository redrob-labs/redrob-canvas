#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Guard the design system against the ways it drifts back, each of which a build happily passes.

  1. A drifted vendored token or icon file. The delivery is pinned by sha256 in DESIGN_SYSTEM_PIN.json;
     tools/generate_design_tokens.py --check verifies every pin, and this calls it.
  2. A brand file edited by hand, or a hand-drawn brand mark coming back. Every file under
     resources/icons/ and resources/brand/ is built by scripts/build-brand-assets.py from the design
     system's shipped files and pinned by sha256 in brand/SOURCES.json; scripts/check-brand.py
     refuses a drifted or unpinned one (an earlier resources/icons/redrob.svg was a typed letter R in
     an invented gradient, which 10-logo.md forbids, and nothing in a build would have rejected it).
  3. GPL text in a generated SVG: this tree is MIT OR Apache-2.0 (check-brand.py).
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

for check in (["tools/generate_design_tokens.py", "--check"], ["scripts/check-brand.py"]):
    result = subprocess.run([sys.executable, str(ROOT / check[0]), *check[1:]], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        failures.append(f"{' '.join(check)} failed:\n    " + result.stderr.strip().replace("\n", "\n    "))

# DESIGN_SYSTEM_PIN.json names the brand pin file; the two must not drift apart.
pin = json.loads((ROOT / "DESIGN_SYSTEM_PIN.json").read_text(encoding="utf-8"))
if pin.get("brand", {}).get("pins") != "brand/SOURCES.json":
    failures.append("DESIGN_SYSTEM_PIN.json no longer points brand.pins at brand/SOURCES.json")

if failures:
    print(f"design system guard: {len(failures)} problem(s)\n", file=sys.stderr)
    for failure in failures:
        print("  " + failure, file=sys.stderr)
    sys.exit(1)
print("design system guard: clean")
