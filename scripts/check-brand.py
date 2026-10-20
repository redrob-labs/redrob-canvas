#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Refuse brand assets that are not exactly what build-brand-assets.py made from their sources.

    scripts/check-brand.py

No dependencies: the gate and CI run it without ImageMagick, Chromium, the design system or the
@redrob-labs/ui package. A build reads no pixels, so a hand-drawn or recolored mark passes every
other check for as long as nobody looks; this one compares bytes against the pins
brand/SOURCES.json recorded when the files were built, and refuses any file under resources/icons
or resources/brand that the pins do not account for (resources/icons/tools, the shell's own tool
glyphs, is listed in DESIGN_SYSTEM_PIN.json instead and is left alone).
"""

import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCES_JSON = ROOT / "brand" / "SOURCES.json"
# Hand-drawn glyphs the design system does not ship, accounted for in DESIGN_SYSTEM_PIN.json.
NOT_BRAND = {"resources/icons/tools"}


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    errors = []
    pins = json.loads(SOURCES_JSON.read_text(encoding="utf-8"))
    outputs = pins["outputs"]

    present = set()
    for owned in pins["owned_dirs"]:
        for path in (ROOT / owned).rglob("*"):
            rel = path.relative_to(ROOT).as_posix()
            if path.is_file() and not any(rel.startswith(f"{skip}/") for skip in NOT_BRAND):
                present.add(rel)
    for rel in sorted(present - set(outputs)):
        errors.append(f"{rel} is not built from the design system (not in brand/SOURCES.json)")
    for rel, digest in sorted(outputs.items()):
        path = ROOT / rel
        if not path.is_file():
            errors.append(f"{rel} is missing")
        elif sha256(path) != digest:
            errors.append(f"{rel} was changed by hand: rebuild it with scripts/build-brand-assets.py")

    # Files carried unchanged from a delivery must agree with that delivery's pin.
    app_icon_sources = pins.get("app_icons", {}).get("sources", {})
    delivered = {}
    for rel, source in pins["copied_unchanged"].items():
        expected = pins["sources"].get(source) or app_icon_sources.get(source)
        if outputs.get(rel) != expected:
            errors.append(f"{rel} is not byte-identical to {source}")
        elif source in app_icon_sources:
            delivered[rel] = source

    # An SVG built here (not a byte-for-byte delivered app icon, whose own mask is the drawing) may
    # apply no effect to the mark.
    for rel in outputs:
        if not rel.endswith(".svg") or not (ROOT / rel).is_file():
            continue
        text = (ROOT / rel).read_text(encoding="utf-8")
        if rel not in delivered and re.search(r"\b(filter|mask|opacity|transform)=", text):
            errors.append(f"{rel} applies an effect to the mark (filter, mask, opacity or transform)")
        if re.search(r"GPL|GNU General Public", text):
            errors.append(f"{rel} carries GPL text; this tree is MIT OR Apache-2.0")

    if errors:
        print("\n".join(f"error: {error}" for error in errors), file=sys.stderr)
        sys.exit(1)
    print(f"brand assets match the design system pins: {len(outputs)} files")


if __name__ == "__main__":
    main()
