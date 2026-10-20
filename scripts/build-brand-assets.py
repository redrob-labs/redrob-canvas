#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Build every Redrob Canvas brand asset from the design system's published files, and pin them.

    scripts/build-brand-assets.py --design-system ~/workplace/redrob-design-system \\
        --ui <unpacked `npm pack @redrob-labs/ui@1.8.0`>/package

Nothing here draws a mark. The app icon comes from the 2026 app-icon delivery that
@redrob-labs/ui ships (dist/brand/app-icons/redrob-canvas/); the lockups and splash come from the
design system. Every file read is pinned by sha256 in brand/SOURCES.json together with the sha256
of every output. scripts/check-brand.py, which tools/check_design_system.py and CI run, refuses an
output tree that differs from the pins, so a hand edit, a re-trace or a recolor fails the gate.
Ported from redrob-shape's scripts/build-brand-assets.py.

What is derived, and how:
  - Every desktop icon (hicolor PNGs, .ico, .icns, installer PNGs, the window icon and the launcher
    PNG) is the rounded `icon-web` variant, transparent corners, because the launcher, Windows and
    Linux show the PNG unmasked. It is rendered once at 1024 px by headless Chromium (ImageMagick's
    SVG reader drops the gradients) and every smaller size is a Lanczos downsize of that render.
  - The .ico, the .icns and the freedesktop hicolor set are containers around those pixels.
  - redrob-canvas.svg is the delivery's icon-web.svg, byte for byte. redrob-canvas-small.svg is
    its symbol.svg, byte for byte: the white symbol alone, for a dark ground that is not the tile.
  - The lockups follow assets/Products/README.md's product-lockup rule: the Redrob Primary lockup
    placed untouched, 400px tall, with the name in Pretendard SemiBold at 70% of the height, set 40%
    of the height after the wordmark, on the wordmark's baseline. The construction is proven before
    anything is written: the shipped Browser lockup is rebuilt the same way and must match.
  - The splash / About image is Redrob Black with the white lockup unit at 56px, the maths of
    redrob-shape's splash.
  - The installer PNGs are the 256 icon and the white lockup scaled, never redrawn.

Needs ImageMagick 7 (`magick`) for resizing and the lockups, and a Chromium headless shell
(`--chromium`, default: Playwright's) for the one SVG render; the check needs neither.
"""

import argparse
import hashlib
import json
import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCES_JSON = ROOT / "brand" / "SOURCES.json"

UI_PACKAGE = "@redrob-labs/ui@1.8.0"
APP_ICON_KEY = "redrob-canvas"
APP_ICONS = "dist/brand/app-icons"
APP_ICON_SOURCES = {
    "icon-web": f"{APP_ICONS}/{APP_ICON_KEY}/{APP_ICON_KEY}-icon-web.svg",
    "symbol": f"{APP_ICONS}/{APP_ICON_KEY}/{APP_ICON_KEY}-symbol.svg",
}
CHROMIUM = Path.home() / (
    ".cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell"
)

SOURCES = {
    "lockup": "assets/Logos/redrob-logo-primary.png",
    "lockup-white": "assets/Logos/redrob-logo-primary-white.png",
    "browser-lockup-white": "assets/Products/redrob-browser-lockup-white.png",
    "pretendard-semibold": "fonts/Pretendard-SemiBold.woff2",
    "tokens": "tokens.css",
}

# Outputs, relative to the repository root.
ICONS = "resources/icons"
BRAND = "resources/brand"
OUT_ICO = f"{ICONS}/redrob-canvas.ico"
OUT_ICNS = f"{ICONS}/redrob-canvas.icns"
OUT_SVG_APP = f"{ICONS}/redrob-canvas.svg"
OUT_SVG_SMALL = f"{ICONS}/redrob-canvas-small.svg"
HICOLOR_SIZES = (16, 32, 48, 64, 128, 256, 512)
OUT_LOCKUP = f"{BRAND}/redrob-canvas-lockup.png"
OUT_LOCKUP_WHITE = f"{BRAND}/redrob-canvas-lockup-white.png"
OUT_SPLASH = f"{BRAND}/redrob-canvas-splash.png"
OUT_WIN_ICON = "installer/windows/res/icon.png"
OUT_WIN_LOCKUP = "installer/windows/res/lockup.png"
OUT_MAC_ICON = "installer/macos/res/icon.png"

PRODUCT = "Canvas"
LOCKUP_HEIGHT = 400  # assets/Products/README.md: 400px tall, transparent
SPLASH_SIZE = (1000, 500)
SPLASH_LOCKUP_HEIGHT = 56  # px; the lockup's minimum on screen is 18
# The setup window draws the lockup 34 dip tall (above the 18px minimum); 68px covers 200% displays.
SETUP_LOCKUP_HEIGHT = 68
# Measured from redrob-browser-lockup-white.png (3218 x 400): the name's ink spans rows 128..328 and
# "Browser" has no descender, so the baseline is row 327 (annotate puts the cap tops on row 128). Pretendard SemiBold at
# 280pt (70% of 400) reproduces the shipped name's width to the pixel with this tracking.
NAME_BASELINE = 327 / 400
NAME_KERNING = -4.5 / 280  # per point
NAME_TAIL = 6  # the shipped lockups end 6px after the name's ink
# The rebuilt Browser lockup differs from the shipped one in 0.64% of alpha pixels (antialiasing
# along the name's edges); moving the name 1px right or down raises that past 0.75%.
BROWSER_TOLERANCE = 0.0075


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def magick(*args, capture=False):
    result = subprocess.run(["magick", *map(str, args)], check=True, capture_output=True)
    return result.stdout if capture else None


def png_size(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    return struct.unpack(">II", data[16:24])


def token(tokens_css, name):
    """A token's hex value as the light theme resolves it (its block comes first in tokens.css)."""
    match = re.search(rf"--{re.escape(name)}:\s*(#[0-9a-fA-F]{{6}}|var\(--[\w-]+\))", tokens_css)
    if not match:
        sys.exit(f"tokens.css has no --{name}")
    value = match.group(1)
    if value.startswith("var("):
        return token(tokens_css, value[6:-1])
    return value.lower()


def write_ico(path, pngs):
    """ICO with PNG entries (Windows Vista and later)."""
    header = struct.pack("<HHH", 0, 1, len(pngs))
    entries, blobs = b"", b""
    offset = 6 + 16 * len(pngs)
    for data in pngs:
        width, height = png_size(data)
        entries += struct.pack(
            "<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(data), offset + len(blobs)
        )
        blobs += data
    path.write_bytes(header + entries + blobs)


def write_icns(path, entries):
    """ICNS of PNG elements: (type, png bytes) pairs."""
    body = b"".join(kind.encode("ascii") + struct.pack(">I", 8 + len(data)) + data for kind, data in entries)
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def render_svg(chromium, svg, size, out, scratch):
    """Rasterize the SVG at size x size on a transparent ground with headless Chromium."""
    page = scratch / "render.html"
    page.write_text(
        "<!doctype html><html><head><style>html,body{margin:0;background:transparent}"
        f"img{{display:block;width:{size}px;height:{size}px}}</style></head>"
        f'<body><img src="{svg.as_uri()}"></body></html>\n',
        encoding="utf-8",
    )
    subprocess.run(
        [str(chromium), "--headless", "--no-sandbox", "--hide-scrollbars", "--disable-gpu",
         "--allow-file-access-from-files", "--default-background-color=00000000",
         f"--window-size={size},{size}", f"--screenshot={out}", page.as_uri()],
        check=True, capture_output=True,
    )
    raw = scratch / "render-raw.png"
    out.rename(raw)
    magick(raw, "-define", "png:exclude-chunks=date,time", "-strip", f"PNG32:{out}")
    if png_size(out.read_bytes()) != (size, size):
        sys.exit(f"Chromium rendered {svg.name} at the wrong size")
    return out


def resize(source, size, out):
    magick(source, "-filter", "Lanczos", "-resize", f"{size}x{size}",
           "-define", "png:exclude-chunks=date,time", "-strip", f"PNG32:{out}")
    return out.read_bytes()


def compose_lockup(logo, name, colour, height, font, out, scratch):
    """The product-lockup rule: the Primary lockup untouched, the name after it on its baseline."""
    mark = scratch / f"mark-{height}-{out.name}"
    magick(logo, "-filter", "Lanczos", "-resize", f"x{height}", "-strip",
           "-define", "png:exclude-chunks=date,time", f"PNG32:{mark}")
    mark_width = int(magick(mark, "-format", "%w", "info:", capture=True))
    size = 0.7 * height
    gap = round(0.4 * height)
    kerning = NAME_KERNING * size
    text = ["-font", font, "-pointsize", f"{size:g}", "-kerning", f"{kerning:g}", "-fill", colour]
    # Generous, then trimmed on the east only: the height stays `height` and the mark stays at x 0.
    canvas_width = mark_width + gap + int(size * len(name) * 1.2)
    magick(
        "-size", f"{canvas_width}x{height}", "xc:none",
        mark, "-geometry", "+0+0", "-composite",
        *text, "-annotate", f"+{mark_width + gap}+{round(NAME_BASELINE * height)}", name,
        "-define", "trim:edges=east", "-trim", "+repage",
        "-gravity", "east", "-background", "none", "-splice", f"{NAME_TAIL}x0", "+gravity",
        "-strip", "-define", "png:exclude-chunks=date,time", f"PNG32:{out}",
    )


def verify_construction(design, font, scratch):
    """Rebuild the shipped Browser lockup this way; refuse to write anything if it differs."""
    shipped = design / SOURCES["browser-lockup-white"]
    rebuilt = scratch / "browser.png"
    compose_lockup(design / SOURCES["lockup-white"], "Browser", "white", LOCKUP_HEIGHT, font, rebuilt, scratch)
    width, height = magick(shipped, "-format", "%w %h", "info:", capture=True).decode().split()
    padded = scratch / "browser-padded.png"
    magick(rebuilt, "-background", "none", "-extent", f"{width}x{height}", f"PNG32:{padded}")
    # Alpha only: the mark is the same file either way, so the name is what is compared.
    share = float(magick(
        "(", shipped, "-alpha", "extract", ")", "(", padded, "-alpha", "extract", ")",
        "-compose", "difference", "-composite", "-threshold", "50%", "-format", "%[fx:mean]", "info:",
        capture=True,
    ))
    if share > BROWSER_TOLERANCE:
        sys.exit(f"rebuilt Browser lockup differs from the shipped one in {share:.2%} of pixels; "
                 "the construction is not the delivery's, so nothing was written")
    print(f"construction check: rebuilt Browser lockup matches the shipped one ({share:.3%} pixels differ)")


def build_splash(path, design, black, font, scratch):
    width, height = SPLASH_SIZE
    unit = scratch / "splash-unit.png"
    compose_lockup(design / SOURCES["lockup-white"], PRODUCT, "white", SPLASH_LOCKUP_HEIGHT, font, unit, scratch)
    unit_width = int(magick(unit, "-format", "%w", "info:", capture=True))
    left = (width - unit_width) // 2
    top = (height - SPLASH_LOCKUP_HEIGHT) // 2
    magick(
        "-size", f"{width}x{height}", f"xc:{black}",
        unit, "-geometry", f"+{left}+{top}", "-composite",
        "-alpha", "off", "-define", "png:exclude-chunks=date,time", "-strip", f"PNG24:{path}",
    )


def app_icon_files(ui):
    """The delivery's SVGs, checked against the package's own manifest before anything uses them."""
    manifest = json.loads((ui / APP_ICONS / "manifest.json").read_text(encoding="utf-8"))
    listed = {}
    for entry in manifest["icons"]:
        for variant, meta in entry["files"].items():
            listed[f"{APP_ICONS}/{meta['path']}"] = meta["sha256"]
    for rel in APP_ICON_SOURCES.values():
        if rel not in listed:
            sys.exit(f"{UI_PACKAGE} manifest.json does not list {rel}")
        if sha256(ui / rel) != listed[rel]:
            sys.exit(f"{rel} does not match {UI_PACKAGE}'s manifest.json")
    return {rel: listed[rel] for rel in APP_ICON_SOURCES.values()}


def build(design, ui, chromium):
    missing = [rel for rel in SOURCES.values() if not (design / rel).is_file()]
    missing += [rel for rel in APP_ICON_SOURCES.values() if not (ui / rel).is_file()]
    if missing:
        sys.exit(f"source files missing: {missing}")
    app_icon_pins = app_icon_files(ui)
    read = {key: (design / rel).read_bytes() for key, rel in SOURCES.items()}
    tokens_css = read["tokens"].decode("utf-8")
    black, ink = token(tokens_css, "redrob-black"), token(tokens_css, "ink-primary")
    font = design / SOURCES["pretendard-semibold"]
    icon_web = ui / APP_ICON_SOURCES["icon-web"]
    symbol = ui / APP_ICON_SOURCES["symbol"]

    with tempfile.TemporaryDirectory() as temporary:
        scratch = Path(temporary)
        verify_construction(design, font, scratch)
        master = render_svg(chromium, icon_web, 1024, scratch / "icon-1024.png", scratch)
        pngs = {size: resize(master, size, scratch / f"icon-{size}.png") for size in HICOLOR_SIZES}
        pngs[1024] = master.read_bytes()

        def write_lockup(logo_key, colour):
            return lambda p: compose_lockup(design / SOURCES[logo_key], PRODUCT, colour, LOCKUP_HEIGHT, font, p, scratch)

        def scale_lockup_white(p):
            magick(ROOT / OUT_LOCKUP_WHITE, "-filter", "Lanczos", "-resize", f"x{SETUP_LOCKUP_HEIGHT}",
                   "-strip", "-define", "png:exclude-chunks=date,time", f"PNG32:{p}")

        outputs = {
            OUT_SVG_APP: lambda p: p.write_bytes(icon_web.read_bytes()),
            OUT_SVG_SMALL: lambda p: p.write_bytes(symbol.read_bytes()),
            OUT_ICO: lambda p: write_ico(p, [pngs[16], pngs[32], pngs[48], pngs[64], pngs[256]]),
            OUT_ICNS: lambda p: write_icns(p, [
                ("icp4", pngs[16]), ("icp5", pngs[32]), ("icp6", pngs[64]),
                ("ic07", pngs[128]), ("ic08", pngs[256]), ("ic09", pngs[512]), ("ic10", pngs[1024]),
                ("ic11", pngs[32]), ("ic12", pngs[64]), ("ic13", pngs[256]), ("ic14", pngs[512]),
            ]),
            **{f"{ICONS}/hicolor/{s}x{s}/apps/redrob-canvas.png": (lambda data: lambda p: p.write_bytes(data))(pngs[s])
               for s in HICOLOR_SIZES},
            OUT_LOCKUP: write_lockup("lockup", ink),
            OUT_LOCKUP_WHITE: write_lockup("lockup-white", "white"),
            OUT_SPLASH: lambda p: build_splash(p, design, black, font, scratch),
            OUT_WIN_ICON: lambda p: p.write_bytes(pngs[256]),
            OUT_MAC_ICON: lambda p: p.write_bytes(pngs[256]),
            OUT_WIN_LOCKUP: scale_lockup_white,
        }
        for rel, make in outputs.items():
            target = ROOT / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            make(target)

    pins = {
        "//": "Written by scripts/build-brand-assets.py; checked by scripts/check-brand.py. Do not edit.",
        "design_system": "redrob-design-system (the published distribution)",
        "sources": {rel: sha256(design / rel) for rel in SOURCES.values()},
        "app_icons": {"package": UI_PACKAGE, "variant": "icon-web", "sources": app_icon_pins},
        "tokens": {"redrob-black": black, "ink-primary": ink},
        "copied_unchanged": {
            OUT_SVG_APP: APP_ICON_SOURCES["icon-web"],
            OUT_SVG_SMALL: APP_ICON_SOURCES["symbol"],
        },
        "owned_dirs": [ICONS, BRAND],
        "outputs": {rel: sha256(ROOT / rel) for rel in sorted(outputs)},
    }
    SOURCES_JSON.parent.mkdir(parents=True, exist_ok=True)
    SOURCES_JSON.write_text(json.dumps(pins, indent=2) + "\n", encoding="utf-8")
    for rel in sorted(outputs):
        print(f"  {rel}  {pins['outputs'][rel][:12]}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--design-system", required=True, type=Path)
    parser.add_argument("--ui", required=True, type=Path, help=f"the unpacked {UI_PACKAGE} (its package/ dir)")
    parser.add_argument("--chromium", type=Path, default=CHROMIUM)
    args = parser.parse_args()
    build(args.design_system.expanduser().resolve(), args.ui.expanduser().resolve(), args.chromium.expanduser())


if __name__ == "__main__":
    main()
