#!/usr/bin/env python3
"""Copy the Qt runtime into an installed Linux tree, so the tarball and the .deb run without Qt.

    bundle_linux_qt.py <stage> <qt-prefix>

<stage> is what `cmake --install` produced (bin/redrob-canvas, share/...). Until 0.5.5 that tree was
shipped as is: the binary's RUNPATH is $ORIGIN/../lib, but nothing was put there, so the Linux download
started only on a machine that already had Qt 6.11 installed system-wide -- which no distribution ships.

What goes in, mirroring the Qt prefix layout so Qt's own RUNPATHs ($ORIGIN/../../lib and so on) hold:

  lib/       every library under <qt-prefix>/lib that the binary, the plugins and the QML modules need,
             found by following ldd until nothing new appears
  plugins/   the platform plugins (xcb and Wayland), their GL integrations, image formats, icon engines,
             input contexts, TLS backends (the console sign-in is HTTPS) and network information
  qml/       QtCore, QtQml and QtQuick (without tooling): everything the app's `import` lines reach
  bin/qt.conf  tells Qt where those are, relative to the binary

System libraries (libc, libGL, X11, fontconfig, ...) are NOT copied: they must match the user's system.
The .deb names them as dependencies (tools/release/make-deb.sh).
"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

PLUGIN_DIRS = ["platforms", "xcbglintegrations", "egldeviceintegrations", "imageformats", "iconengines",
               "platforminputcontexts", "platformthemes", "tls", "networkinformation",
               "wayland-decoration-client", "wayland-graphics-integration-client", "wayland-shell-integration",
               "generic"]
QML_DIRS = ["QtCore", "QtQml", "QtQuick"]
QML_SKIP = {"tooling"}  # the QML debugger, not for a shipped app
# The GTK 3 theme plugin would make GTK a dependency of the package; Qt's own Fusion style is what the
# app draws with, and the xdg-desktop-portal theme plugin still follows the desktop's dark mode.
PLUGIN_SKIP = {"libqgtk3.so"}

QT_CONF = """[Paths]
Prefix = ..
Libraries = lib
Plugins = plugins
QmlImports = qml
"""


def is_elf(path: Path) -> bool:
    try:
        with open(path, "rb") as f:
            return f.read(4) == b"\x7fELF"
    except OSError:
        return False


def ldd_qt_deps(path: Path, qt_lib: Path, env: dict) -> set[Path]:
    """Libraries `path` needs that resolve into the Qt prefix's lib directory."""
    out = subprocess.run(["ldd", str(path)], capture_output=True, text=True, env=env).stdout
    deps = set()
    for line in out.splitlines():
        m = re.match(r"\s*(\S+) => (\S+)", line)
        if m and m.group(2) != "not":
            target = Path(m.group(2)).resolve()
            if qt_lib in target.parents:
                deps.add(Path(m.group(2)))
        elif "not found" in line:
            name = line.split()[0]
            if (qt_lib / name).exists():
                deps.add(qt_lib / name)
    return deps


def copy_lib(src: Path, lib_dir: Path) -> Path:
    """Copy a library under its SONAME-style name (the name ldd reported), following the symlink chain."""
    dest = lib_dir / src.name
    if not dest.exists():
        shutil.copy2(src.resolve(), dest)
    return dest


def main() -> int:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    stage = Path(sys.argv[1]).resolve()
    qt = Path(sys.argv[2]).resolve()
    qt_lib = qt / "lib"
    binary = stage / "bin" / "redrob-canvas"
    if not binary.is_file():
        sys.exit(f"no {binary}")
    for d in ("lib", "plugins", "qml"):
        (stage / d).mkdir(exist_ok=True)

    for name in PLUGIN_DIRS:
        src = qt / "plugins" / name
        if src.is_dir():
            shutil.copytree(src, stage / "plugins" / name, dirs_exist_ok=True,
                            ignore=lambda d, names: [n for n in names if n in PLUGIN_SKIP])
    for name in QML_DIRS:
        shutil.copytree(qt / "qml" / name, stage / "qml" / name, dirs_exist_ok=True,
                        ignore=lambda d, names: [n for n in names if n in QML_SKIP])
    (stage / "bin" / "qt.conf").write_text(QT_CONF)

    # Resolve against the Qt prefix only: a system Qt must never satisfy a lookup here.
    env = dict(os.environ, LD_LIBRARY_PATH=str(qt_lib))
    pending = [binary] + [p for p in (stage / "plugins").rglob("*.so") if is_elf(p)] \
                       + [p for p in (stage / "qml").rglob("*.so") if is_elf(p)]
    seen: set[str] = set()
    copied = 0
    while pending:
        item = pending.pop()
        for dep in ldd_qt_deps(item, qt_lib, env):
            if dep.name in seen:
                continue
            seen.add(dep.name)
            pending.append(copy_lib(dep, stage / "lib"))
            copied += 1

    # Proof, not hope: with the Qt prefix out of reach, nothing may resolve outside the stage.
    clean_env = {k: v for k, v in os.environ.items() if k != "LD_LIBRARY_PATH"}
    leaks = []
    for elf in [binary, *(stage / "lib").glob("*.so*"), *(stage / "plugins").rglob("*.so")]:
        out = subprocess.run(["ldd", str(elf)], capture_output=True, text=True, env=clean_env).stdout
        for line in out.splitlines():
            if str(qt) in line:
                leaks.append(f"{elf.relative_to(stage)}: {line.strip()}")
            m = re.match(r"\s*(libQt6\S+) => not found", line)
            if m:
                leaks.append(f"{elf.relative_to(stage)}: {m.group(1)} not found")
    if leaks:
        print("Qt libraries still resolve outside the bundle:\n  " + "\n  ".join(leaks[:20]), file=sys.stderr)
        return 1
    size = sum(f.stat().st_size for f in stage.rglob("*") if f.is_file()) // (1024 * 1024)
    print(f"bundled {copied} Qt libraries, {len(PLUGIN_DIRS)} plugin groups and {len(QML_DIRS)} QML modules "
          f"into {stage} ({size} MB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
