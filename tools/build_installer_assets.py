#!/usr/bin/env python3
"""Rasterise the committed product icon for the platforms that cannot read SVG.

Windows wants an .ico (the exe's own icon, Start-menu and desktop shortcuts, Settings > Apps) and the
setup window wants a PNG; neither toolchain on the Windows machine renders SVG. So the rasters are
made here, from the same SVGs tools/build_app_icon.py writes, and committed:

  resources/icons/redrob-canvas.ico   16..256. 32px and below come from the small master, as the
                                      design system's product README asks for at that size.
  resources/icons/redrob-canvas.icns  the macOS bundle icon (Finder, Dock, the disk image), 32..1024,
                                      from the macOS-grid master.
  installer/windows/res/icon.png      256px, drawn at 2x in the 128-dip setup window.
  installer/macos/res/icon.png        same raster, used on the DMG background.

Needs ImageMagick (`magick`). Run after tools/build_app_icon.py whenever the SVGs change.
"""
from __future__ import annotations

import hashlib
import pathlib
import re
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
ICONS = ROOT / "resources/icons"
MASTER = ICONS / "redrob-canvas.svg"
SMALL = ICONS / "redrob-canvas-small.svg"
ICO = ICONS / "redrob-canvas.ico"
MACOS_SVG = ICONS / "redrob-canvas-macos.svg"
ICNS = ICONS / "redrob-canvas.icns"
# ICNS entry types holding PNG data, by pixel size (Apple's "Icon Composer" set; 'ic11'-'ic14' are @2x).
ICNS_TYPES = [(b"ic11", 32), (b"ic12", 64), (b"ic07", 128), (b"ic13", 256), (b"ic08", 256),
              (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)]
PNGS = [ROOT / "installer/windows/res/icon.png", ROOT / "installer/macos/res/icon.png"]
# The setup window draws the product lockup 34 dip tall (the browser setup's size, above the 18px minimum
# in 10-logo.md); 68px covers 200% displays. White, for the setup's deep ground.
LOCKUP_WHITE = ROOT / "resources/brand/redrob-canvas-lockup-white.png"
SETUP_LOCKUP = ROOT / "installer/windows/res/lockup.png"
SETUP_LOCKUP_HEIGHT = 68
SIZES = [16, 24, 32, 48, 64, 128, 256]


def render(svg: pathlib.Path, size: int, out: pathlib.Path) -> None:
    # Render large, then downsample: the internal SVG renderer antialiases poorly at small sizes.
    # -strip and no date chunks, so the same SVG always gives the same bytes and the pins stay put.
    subprocess.run(["magick", "-background", "none", "-density", "768", str(svg),
                    "-resize", f"{size}x{size}", "-depth", "8", "-strip",
                    "-define", "png:exclude-chunks=date,time", f"PNG32:{out}"], check=True)


def main() -> int:
    if not shutil.which("magick"):
        sys.exit("ImageMagick (magick) is required")
    with tempfile.TemporaryDirectory() as tmp:
        frames = []
        for s in SIZES:
            p = pathlib.Path(tmp) / f"{s}.png"
            render(SMALL if s <= 32 else MASTER, s, p)
            frames.append(str(p))
        subprocess.run(["magick", *frames, str(ICO)], check=True)
        big = pathlib.Path(tmp) / "256.png"
        for png in PNGS:
            png.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(big, png)
        # The bundle icon. Drawn from the macOS-grid master, so the Dock and Finder show it at the same
        # size as every other app; ICNS entries carry PNG data, so no macOS tool is needed to write it.
        entries = b""
        for kind, size in ICNS_TYPES:
            p = pathlib.Path(tmp) / f"mac{size}.png"
            if not p.exists():
                render(MACOS_SVG, size, p)
            data = p.read_bytes()
            entries += kind + struct.pack(">I", 8 + len(data)) + data
        ICNS.write_bytes(b"icns" + struct.pack(">I", 8 + len(entries)) + entries)
    # Scaled, not redrawn: the pinned lockup (tools/build_lockup.py) at the window's size.
    subprocess.run(["magick", str(LOCKUP_WHITE), "-filter", "Lanczos", "-resize", f"x{SETUP_LOCKUP_HEIGHT}",
                    "-strip", "-define", "png:exclude-chunks=date,time", f"PNG32:{SETUP_LOCKUP}"], check=True)
    # Regeneration owns the pins in DESIGN_SYSTEM_PIN.json, as tools/build_app_icon.py does for the SVGs;
    # tools/build_app_icon.py --check (and so the design-system guard) holds these rasters to them.
    pin = ROOT / "DESIGN_SYSTEM_PIN.json"
    text = pin.read_text(encoding="utf-8")
    for path in (ICO, ICNS):
        relative = str(path.relative_to(ROOT))
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        pattern = re.compile(r'("file": "' + re.escape(relative) + r'",(?:[^{}]*?)"sha256": ")[0-9a-f]*(")')
        text, count = pattern.subn(lambda m: m.group(1) + digest + m.group(2), text)
        if count != 1:
            sys.exit(f"{relative} has no single sha256 entry in {pin.name}; add one, then rerun")
    pin.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {ICO.relative_to(ROOT)}, {ICNS.relative_to(ROOT)} and "
          f"{', '.join(str(p.relative_to(ROOT)) for p in PNGS)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
