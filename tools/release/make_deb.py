#!/usr/bin/env python3
"""Package a bundled Linux tree (tools/release/bundle_linux_qt.py) as a .deb.

    make_deb.py <stage> <version> <out.deb>

  /opt/redrob-canvas/...                     the tree as is (bin, lib, plugins, qml, share)
  /usr/bin/redrob-canvas                     a symlink, so a terminal finds it
  /usr/share/applications/redrob-canvas.desktop   the menu entry
  /usr/share/icons/hicolor/{scalable,256x256}/apps/redrob-canvas.{svg,png}

Uninstalling is the system's own: `sudo apt remove redrob-canvas`, or the software centre. Nothing is
written outside the package's own files, so removal leaves nothing behind; a person's settings in
~/.config/Redrob stay, as for every other app.

Depends is worked out the way dpkg-shlibdeps would: every library an ELF file in the tree needs that the
tree does not carry is looked up with `dpkg -S` on this build machine. The few the Qt xcb plugin needs
that a minimal machine may not have installed (so dpkg -S cannot name them) come from a fixed table.
Package names are this build machine's (e.g. the t64 names), which matches the archive's own promise:
it runs on distributions at least as new as the one it was built on.
"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
ICON_SVG = ROOT / "resources" / "icons" / "redrob-canvas.svg"
ICON_PNG = ROOT / "installer" / "windows" / "res" / "icon.png"  # 256px, same raster as the setup's
# Libraries Qt's xcb platform plugin needs that dpkg -S cannot name when they are not installed here.
KNOWN = {
    "libxcb-cursor.so.0": "libxcb-cursor0",
    "libxcb-icccm.so.4": "libxcb-icccm4",
    "libxcb-image.so.0": "libxcb-image0",
    "libxcb-keysyms.so.1": "libxcb-keysyms1",
    "libxcb-render-util.so.0": "libxcb-render-util0",
    "libxcb-util.so.1": "libxcb-util1",
    "libxkbcommon-x11.so.0": "libxkbcommon-x11-0",
    # GL is often diverted by a vendor driver package on the build machine (dpkg -S then names the
    # driver); a user's machine gets it from the distribution's GL packages.
    "libGL.so.1": "libgl1",
    "libEGL.so.1": "libegl1",
    "libGLX.so.0": "libglx0",
    "libOpenGL.so.0": "libopengl0",
    "libz.so.1": "zlib1g",
}
# Loaded at run time rather than linked, so no ELF names them, but the xcb plugin needs them.
ALWAYS = ["libxcb-cursor0", "libgl1", "libegl1", "libfontconfig1"]

DESKTOP = """[Desktop Entry]
Type=Application
Name=Redrob Canvas
GenericName=Image Editor
Comment=Edit images, with an agent that works on the canvas beside you
Exec=redrob-canvas %F
Icon=redrob-canvas
Terminal=false
Categories=Graphics;2DGraphics;RasterGraphics;
MimeType=image/png;image/jpeg;image/webp;image/tiff;image/bmp;image/gif;image/vnd.adobe.photoshop;
StartupWMClass=redrob-canvas
"""


def elf_files(stage: Path):
    for p in stage.rglob("*"):
        if p.is_file() and not p.is_symlink():
            with open(p, "rb") as f:
                if f.read(4) == b"\x7fELF":
                    yield p


def depends(stage: Path) -> list[str]:
    carried = {p.name for p in (stage / "lib").iterdir()}
    needed: set[str] = set()
    for elf in elf_files(stage):
        out = subprocess.run(["readelf", "-d", str(elf)], capture_output=True, text=True).stdout
        needed.update(re.findall(r"\(NEEDED\)\s+Shared library: \[(.+?)\]", out))
    system = sorted(n for n in needed if n not in carried)
    pkgs: set[str] = set(ALWAYS)
    unresolved = []
    for name in system:
        if name in KNOWN:
            pkgs.add(KNOWN[name])
            continue
        r = subprocess.run(["dpkg", "-S", f"*/{name}"], capture_output=True, text=True)
        owners = set()
        for line in r.stdout.splitlines():
            # "pkg[:arch][, pkg...]: /path"; dpkg also prints "diversion by ... from/to: ..." lines.
            if line.startswith("diversion ") or ": /" not in line:
                continue
            for owner in line.split(": /", 1)[0].split(","):
                owners.add(owner.strip().split(":")[0])
        # Only distribution library packages: anything else is this build machine's own software.
        owners = {o for o in owners if o.startswith("lib") and not o.endswith("-dev")}
        if not owners:
            unresolved.append(name)
            continue
        pkgs.add(sorted(owners)[0])
    if unresolved:
        sys.exit("no package found for: " + ", ".join(unresolved) + " (add it to KNOWN)")
    return sorted(pkgs)


def main() -> int:
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    stage, version, out = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        sys.exit("version must be X.Y.Z")
    if not (stage / "lib" / "libQt6Core.so.6").exists():
        sys.exit("the tree carries no Qt; run tools/release/bundle_linux_qt.py first")
    deps = depends(stage)
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "pkg"
        shutil.copytree(stage, root / "opt" / "redrob-canvas", symlinks=True)
        (root / "usr" / "bin").mkdir(parents=True)
        os.symlink("/opt/redrob-canvas/bin/redrob-canvas", root / "usr" / "bin" / "redrob-canvas")
        apps = root / "usr" / "share" / "applications"
        apps.mkdir(parents=True)
        (apps / "redrob-canvas.desktop").write_text(DESKTOP)
        icons = root / "usr" / "share" / "icons" / "hicolor"
        (icons / "scalable" / "apps").mkdir(parents=True)
        (icons / "256x256" / "apps").mkdir(parents=True)
        shutil.copyfile(ICON_SVG, icons / "scalable" / "apps" / "redrob-canvas.svg")
        shutil.copyfile(ICON_PNG, icons / "256x256" / "apps" / "redrob-canvas.png")
        docs = root / "usr" / "share" / "doc" / "redrob-canvas"
        docs.mkdir(parents=True)
        shutil.copyfile(ROOT / "COPYRIGHT", docs / "copyright")
        size_kb = sum(f.stat().st_size for f in root.rglob("*") if f.is_file() and not f.is_symlink()) // 1024
        (root / "DEBIAN").mkdir()
        (root / "DEBIAN" / "control").write_text(
            "Package: redrob-canvas\n"
            f"Version: {version}\n"
            "Section: graphics\n"
            "Priority: optional\n"
            "Architecture: amd64\n"
            "Maintainer: Janghoon Lee (Redrob) <44862514+savagemanage@users.noreply.github.com>\n"
            f"Installed-Size: {size_kb}\n"
            f"Depends: {', '.join(deps)}\n"
            "Homepage: https://github.com/redrob-labs/redrob-canvas\n"
            "Description: Redrob Canvas image editor\n"
            " Redrob Canvas is an agentic image editor: layers, selections, filters and\n"
            " Photoshop or Illustrator shortcuts, with an agent that edits the canvas\n"
            " beside you. Qt is bundled under /opt/redrob-canvas.\n")
        for p in root.rglob("*"):
            if p.is_symlink():
                continue
            # Libraries are not run directly (Debian policy 10.9): 0644 for them, 0755 for programs.
            is_lib = ".so" in p.name
            p.chmod(0o755 if p.is_dir() or (os.access(p, os.X_OK) and not is_lib) else 0o644)
        out.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "--build", str(root), str(out)],
                       check=True, stdout=subprocess.DEVNULL)
    print(f"built {out} ({out.stat().st_size // (1024 * 1024)} MB); Depends: {', '.join(deps)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
