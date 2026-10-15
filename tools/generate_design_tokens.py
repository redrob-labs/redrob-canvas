#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Check the vendored design-system delivery against DESIGN_SYSTEM_PIN.json.

WHY A VENDORED COPY. Every other Redrob product installs `@redrob-labs/ui` and reads its values through
`var(--surface-base)`. This one cannot: there is no Node in this repository's build and the shell is
egui, which has no CSS to evaluate. The delivery anticipates that -- it ships `dist/styles/tokens.json`
with every `var()` chain already resolved per theme, and `dist/native/redrob_tokens.h` with the same
colours as 0xAARRGGBB constants, precisely so a consumer that cannot evaluate CSS still gets the same
numbers. Both are vendored under `third_party/redrob-ui/` and pinned by sha256 in DESIGN_SYSTEM_PIN.json,
so a drifted copy fails a check instead of silently repainting the app.

WHAT THIS CHECKS. Every pinned file's sha256 (tokens, the C header, the icon set), and that each of
our own glyphs under resources/icons/tools exists and shares the delivered geometry. It emits no
generated file any more: the QML token object the Qt shell read was removed with that shell, and the
egui shell reads tokens.json directly. `--check` and a bare run are the same; both exit 1 on drift.

Run by tools/check_design_system.py and by the `design system` CI job.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOKENS = ROOT / "third_party/redrob-ui/tokens.json"
PIN = ROOT / "DESIGN_SYSTEM_PIN.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="kept for the callers; a bare run checks too")
    parser.parse_args()

    pin = json.loads(PIN.read_text(encoding="utf-8"))
    # Tool icons are vendored and pinned the same way as tokens, so they share this gate.
    icons = pin.get("vendored_icons", {}).get("files", [])
    for asset in pin["vendored_tokens"] + icons:
        path = ROOT / asset["file"]
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != asset["sha256"]:
            print(f"{asset['file']} does not match its pin.\n"
                  f"  expected {asset['sha256']}\n  actual   {actual}\n"
                  f"  Re-copy it from @redrob-labs/ui {pin['version']} at {asset['delivery_path']},"
                  f" or update the pin deliberately.", file=sys.stderr)
            return 1

    # Our own glyphs carry no hash -- there is no upstream to drift from -- but a missing or
    # off-geometry one is worse than a drifted hash: an absent SVG renders as an EMPTY button,
    # which builds and runs without a word. So they get an existence and geometry gate instead.
    for rel in pin.get("local_icons", {}).get("files", []):
        path = ROOT / rel
        if not path.is_file():
            print(f"{rel} is listed in local_icons but is not on disk. A missing glyph renders as an"
                  " empty button with no error.", file=sys.stderr)
            return 1
        head = path.read_text(encoding="utf-8")[:400]
        for needed in ('viewBox="0 0 24 24"', 'stroke="currentColor"', 'stroke-width="2"'):
            if needed not in head:
                print(f"{rel} is missing {needed}. The rail reads as one set only while every glyph"
                      " shares the delivered geometry.", file=sys.stderr)
                return 1

    data = json.loads(TOKENS.read_text(encoding="utf-8"))
    colours = sum(1 for t in data["tokens"] if t["kind"] == "color")
    print(f"third_party/redrob-ui matches its pins ({len(pin['vendored_tokens'])} token files,"
          f" {len(icons)} vendored icons, {colours} colour tokens)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
