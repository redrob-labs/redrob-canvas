#!/usr/bin/env python3
"""Rasterise the committed product icon for the platforms that cannot read SVG.

Windows wants an .ico (the exe's own icon, Start-menu and desktop shortcuts, Settings > Apps) and the
setup window wants a PNG; neither toolchain on the Windows machine renders SVG. So the rasters are
made here, from the same SVGs tools/build_app_icon.py writes, and committed:

  resources/icons/redrob-canvas.ico   16..256. 32px and below come from the small master, as the
                                      design system's product README asks for at that size.
  installer/windows/res/icon.png      256px, drawn at 2x in the 128-dip setup window.
  installer/macos/res/icon.png        same raster, used on the DMG background.

Needs ImageMagick (`magick`). Run after tools/build_app_icon.py whenever the SVGs change.
"""
from __future__ import annotations

import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
ICONS = ROOT / "resources/icons"
MASTER = ICONS / "redrob-canvas.svg"
SMALL = ICONS / "redrob-canvas-small.svg"
ICO = ICONS / "redrob-canvas.ico"
PNGS = [ROOT / "installer/windows/res/icon.png", ROOT / "installer/macos/res/icon.png"]
SIZES = [16, 24, 32, 48, 64, 128, 256]


def render(svg: pathlib.Path, size: int, out: pathlib.Path) -> None:
    # Render large, then downsample: the internal SVG renderer antialiases poorly at small sizes.
    subprocess.run(["magick", "-background", "none", "-density", "768", str(svg),
                    "-resize", f"{size}x{size}", "-depth", "8", f"PNG32:{out}"], check=True)


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
    print(f"wrote {ICO.relative_to(ROOT)} and {', '.join(str(p.relative_to(ROOT)) for p in PNGS)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
