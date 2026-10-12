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

import pathlib
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
    print(f"wrote {ICO.relative_to(ROOT)}, {ICNS.relative_to(ROOT)} and "
          f"{', '.join(str(p.relative_to(ROOT)) for p in PNGS)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
