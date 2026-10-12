#!/usr/bin/env python3
"""Build the Redrob Canvas product lockup the way the design system builds the seven it ships.

The delivery has no Canvas lockup: Canvas is a feature of Design's band, not an eighth product
(DESIGN_SYSTEM_PIN.json). assets/Products/README.md gives the construction instead of leaving it open:

  "the Redrob Primary lockup ... with the product name beside it. Name: Pretendard SemiBold at 70% of
   the lockup's height, ink-primary or white, set 40% of the lockup's height after the wordmark, on the
   wordmark's baseline ... 400px tall, transparent."

So this composes exactly that from shipped files only -- the primary logo is placed untouched, never
redrawn -- and proves the construction first: it rebuilds the shipped Browser lockup the same way and
refuses to write anything unless that rebuild matches the shipped file.

    build_lockup.py            write the two Canvas lockups (needs the design-system delivery + ImageMagick)
    build_lockup.py --check    verify the committed lockups against their pins, offline

Outputs (pinned in DESIGN_SYSTEM_PIN.json):
  resources/brand/redrob-canvas-lockup.png         Primary, ink-primary name: light grounds (the disk image)
  resources/brand/redrob-canvas-lockup-white.png   Primary White, white name: dark grounds (the Windows setup)
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DS = pathlib.Path.home() / "workplace/redrob-design-system/assets"
FONT = DS / "Fonts/Pretendard-SemiBold.ttf"
PIN = ROOT / "DESIGN_SYSTEM_PIN.json"
BRAND = ROOT / "resources/brand"
HEIGHT = 400
INK_PRIMARY = "#16181D"  # ink-primary on light, third_party/redrob-ui tokens
# Measured from the shipped Browser lockup: the name's ink starts 160px (40% of 400) after the 2000px
# wordmark, and its cap/x-height ink spans rows 128..328 -- the wordmark's baseline.
GAP = int(HEIGHT * 0.40)
NAME_SIZE = int(HEIGHT * 0.70)
# The delivery's lockups set the name with slightly tight tracking; -4.5 reproduces the shipped
# Browser lockup's width to the pixel (see verify()).
KERNING = -4.5
BASELINE_SHIFT = 60  # shipped name ink rows 128..328; ImageMagick's label ink rows 68..268 at 280px

LOCKUPS = {
    "redrob-canvas-lockup.png": ("redrob-logo-primary.png", INK_PRIMARY),
    "redrob-canvas-lockup-white.png": ("redrob-logo-primary-white.png", "#FFFFFF"),
}


def magick(*args: str) -> str:
    return subprocess.run(["magick", *args], check=True, capture_output=True, text=True).stdout


def compose(logo: pathlib.Path, name: str, colour: str, out: pathlib.Path, tmp: pathlib.Path) -> None:
    mark = tmp / "mark.png"
    magick(str(logo), "-resize", f"x{HEIGHT}", "-strip", f"PNG32:{mark}")
    mark_w = int(magick(str(mark), "-format", "%w", "info:"))
    label = tmp / "label.png"
    magick("-background", "none", "-fill", colour, "-font", str(FONT), "-pointsize", str(NAME_SIZE),
           "-kerning", str(KERNING), f"label:{name}", f"PNG32:{label}")
    label_w = int(magick(str(label), "-format", "%w", "info:"))
    # The label's own origin (its left side bearing included) goes GAP after the wordmark, and its
    # baseline on the wordmark's: ImageMagick's label puts a 280px Pretendard baseline 60px lower than
    # the shipped lockups do, which verify() measures against the Browser lockup.
    x = mark_w + GAP
    y = BASELINE_SHIFT
    width = x + label_w
    magick("-size", f"{width}x{HEIGHT}", "xc:none", str(mark), "-geometry", "+0+0", "-composite",
           str(label), "-geometry", f"+{x}{y:+d}", "-composite",
           # Trim the label's empty right side only; the height stays 400 and the mark stays at x 0.
           "-define", "trim:edges=east", "-trim", "+repage",
           # The shipped lockups end 6px after the name's ink.
           "-gravity", "east", "-background", "none", "-splice", "6x0", "+gravity", "-strip",
           "-define", "png:exclude-chunks=date,time", f"PNG32:{out}")


def verify(tmp: pathlib.Path) -> None:
    """Rebuild the shipped Browser lockup this way; refuse if it does not match."""
    shipped = DS / "Products/redrob-browser-lockup-white.png"
    rebuilt = tmp / "browser.png"
    compose(DS / "Logos/redrob-logo-primary-white.png", "Browser", "#FFFFFF", rebuilt, tmp)
    sw, sh = (int(v) for v in magick(str(shipped), "-format", "%w %h", "info:").split())
    padded = tmp / "browser-padded.png"
    magick(str(rebuilt), "-background", "none", "-extent", f"{sw}x{sh}", f"PNG32:{padded}")
    # Alpha only: the mark is the same file either way, so the name is what is being compared.
    diff = magick("(", str(shipped), "-alpha", "extract", ")", "(", str(padded), "-alpha", "extract", ")",
                  "-compose", "difference", "-composite", "-threshold", "50%", "-format", "%[fx:mean]", "info:")
    share = float(diff)
    # Measured: the construction as written differs in 0.64% of pixels (antialiasing along the name's
    # edges); moving the name 1px right or down raises that to 0.86% / 0.78%. 0.75% passes only the former.
    if share > 0.0075:
        sys.exit(f"rebuilt Browser lockup differs from the shipped one in {share:.2%} of pixels; "
                 "the construction is not the delivery's, so nothing was written")
    print(f"construction check: rebuilt Browser lockup matches the shipped one ({share:.3%} pixels differ)")


def pin_entries() -> dict:
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    return {e["file"]: e for e in pin["generated"] if e["file"].startswith("resources/brand/")}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    if args.check:
        bad = []
        entries = pin_entries()
        for name in LOCKUPS:
            rel = f"resources/brand/{name}"
            path = ROOT / rel
            if rel not in entries:
                bad.append(f"{rel} has no pin")
            elif not path.exists():
                bad.append(f"{rel} is missing")
            elif hashlib.sha256(path.read_bytes()).hexdigest() != entries[rel].get("sha256"):
                bad.append(f"{rel} does not match its pin; regenerate with tools/build_lockup.py")
        for line in bad:
            print(line, file=sys.stderr)
        if not bad:
            print(f"{len(LOCKUPS)} lockups match their pins")
        return 1 if bad else 0

    if not FONT.exists():
        sys.exit(f"the design-system delivery is needed: {FONT}")
    BRAND.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as t:
        tmp = pathlib.Path(t)
        verify(tmp)
        for name, (logo, colour) in LOCKUPS.items():
            compose(DS / "Logos" / logo, "Canvas", colour, BRAND / name, tmp)
    text = PIN.read_text(encoding="utf-8")
    for name in LOCKUPS:
        rel = f"resources/brand/{name}"
        digest = hashlib.sha256((ROOT / rel).read_bytes()).hexdigest()
        pattern = re.compile(r'("file": "' + re.escape(rel) + r'",(?:[^{}]*?)"sha256": ")[0-9a-f]*(")')
        text, count = pattern.subn(lambda m: m.group(1) + digest + m.group(2), text)
        if count != 1:
            sys.exit(f"{rel} has no single sha256 entry in {PIN.name}; add one, then rerun")
    PIN.write_text(text, encoding="utf-8", newline="\n")
    print("wrote " + ", ".join(f"resources/brand/{n}" for n in LOCKUPS))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
