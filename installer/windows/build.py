#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Build the Redrob Canvas Windows setup (and its uninstaller).

Standard library only. It:
  1. locates Visual Studio (vswhere) and runs rc.exe / cl.exe under its x64 developer environment;
  2. compiles redrob_canvas_setup.rc + redrob_canvas_setup.cpp twice:
       uninstall/Uninstall Redrob Canvas.exe   no payload; this IS the uninstaller (--uninstall)
       <name>                                  embeds the release zip and the uninstaller above

Usage:
  build.py --out <dir> --payload <redrob-canvas-X-windows-x64-portable.zip> --version 0.5.6 [--name <exe>]
  build.py --out <dir> --version 0.5.6          # demo-only build (REDROB_NO_PAYLOAD): --demo flags only

The payload is the release's own -portable.zip (tools/release/stage.py: bin/redrob-canvas.exe plus the licence
set), already Authenticode-signed inside, so the setup must be built after signing and then signed itself
(release workflow, Windows job). No Qt runtime is staged any more: the egui shell is one static executable.

The art is committed, not rendered here: res/background.png is res/background.html rasterised at
1320x800, res/lockup.png (the product lockup, tools/build_lockup.py, scaled) comes from tools/build_installer_assets.py. Pretendard is embedded when the design
system delivery is present (--design-system, default ../redrob-design-system) and fontTools is installed;
otherwise the window uses Segoe UI Variable.

Adapted from redrob-labs/redrob-browser installer/windows/build.py (our own code).
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
RES = HERE / "res"
REPO = HERE.parent.parent
DEFAULT_DS = REPO.parent / "redrob-design-system"
ICON = REPO / "resources" / "icons" / "redrob-canvas.ico"
BACKGROUND = RES / "background.png"
LOCKUP_PNG = RES / "lockup.png"


def find_vswhere() -> Path:
    pf = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
    vswhere = Path(pf) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
    if vswhere.exists():
        return vswhere
    found = shutil.which("vswhere")
    if found:
        return Path(found)
    sys.exit("vswhere.exe not found; install Visual Studio with the C++ x64 toolset.")


def vs_dev_env() -> dict:
    """The environment of an x64 developer command prompt."""
    vsinstall = subprocess.run(
        [str(find_vswhere()), "-latest", "-products", "*",
         "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
         "-property", "installationPath"],
        capture_output=True, text=True, check=True).stdout.strip()
    if not vsinstall:
        sys.exit("No Visual Studio install with the C++ x64 toolset found.")
    vcvars = Path(vsinstall) / "VC" / "Auxiliary" / "Build" / "vcvars64.bat"
    if not vcvars.exists():
        sys.exit(f"vcvars64.bat missing under {vsinstall}")
    # vcvars can leave a non-zero errorlevel on success, so key off a marker and a sane env instead.
    # A real batch file carries vcvars' variables into `set`; an inline `cmd /c "call ... & set"` does not.
    marker = "__REDROB_ENV__"
    bat = Path(tempfile.gettempdir()) / "_redrob_canvas_vcvars.bat"
    bat.write_text(f'@echo off\r\ncall "{vcvars}"\r\necho {marker}\r\nset\r\n', encoding="ascii")
    try:
        out = subprocess.run(["cmd", "/c", str(bat)], capture_output=True, text=True, errors="replace").stdout
    finally:
        bat.unlink(missing_ok=True)
    env, seen = {}, False
    for line in out.splitlines():
        if line.strip() == marker:
            seen = True
        elif seen and "=" in line:
            k, v = line.split("=", 1)
            env[k] = v
    keys = {k.upper() for k in env}
    if "PATH" not in keys or "INCLUDE" not in keys:
        sys.exit("vcvars64 did not produce a developer environment (no INCLUDE/PATH).")
    env.setdefault("SystemRoot", os.environ.get("SystemRoot", r"C:\Windows"))
    return env


def prepare_font(design_system: Path, out_ttf: Path) -> bool:
    """Pretendard SemiBold. AddFontMemResourceEx needs TTF/OTF; the design system ships woff2."""
    woff2 = design_system / "fonts" / "Pretendard-SemiBold.woff2"
    if not woff2.exists():
        return False
    try:
        from fontTools.ttLib import TTFont  # type: ignore
        font = TTFont(str(woff2))
        font.flavor = None
        font.save(str(out_ttf))
        return out_ttf.exists()
    except Exception:
        print("note: fontTools not available; the window uses Segoe UI Variable.")
        return False


def run_in_env(cmd: list, env: dict, cwd: str) -> None:
    """CreateProcess resolves the program on the PARENT's PATH, so resolve it on the dev env's first."""
    path = next((v for k, v in env.items() if k.upper() == "PATH"), None)
    resolved = shutil.which(cmd[0], path=path)
    if resolved:
        cmd = [resolved, *cmd[1:]]
    print("$", " ".join(cmd), flush=True)
    subprocess.run(cmd, env=env, check=True, cwd=cwd)


def compile_app(env, out: Path, exe_name: str, font, ver: str, payload, uninstaller) -> Path:
    """rc + cl + link into out/exe_name. Paths reach RC through a generated header with forward slashes:
    RC's preprocessor reads backslashes in string macros as escapes."""
    def fwd(p) -> str:
        return str(p).replace("\\", "/")

    out.mkdir(parents=True, exist_ok=True)
    parts = (ver.split(".") + ["0", "0", "0", "0"])[:4]
    lines = [
        "// generated by build.py -- do not edit",
        f'#define REDROB_BACKGROUND_PATH "{fwd(BACKGROUND)}"',
        f'#define REDROB_LOCKUP_PATH "{fwd(LOCKUP_PNG)}"',
        f'#define REDROB_ICON_PATH "{fwd(ICON)}"',
        f"#define REDROB_VER_FILE {','.join(parts)}",
        f'#define REDROB_VER_STR "{ver}"',
        f'#define REDROB_VER_WSTR L"{ver}"',
        f'#define REDROB_SETUP_KIND "{"Uninstaller" if payload is None else "Setup"}"',
    ]
    if font:
        lines.append(f'#define REDROB_FONT_PATH "{fwd(font)}"')
    if payload:
        lines.append(f'#define REDROB_PAYLOAD_PATH "{fwd(payload)}"')
    else:
        lines.append("#define REDROB_NO_PAYLOAD 1")
    if uninstaller:
        lines.append(f'#define REDROB_UNINSTALLER_PATH "{fwd(uninstaller)}"')
    (out / "build_defs.h").write_text("\n".join(lines) + "\n", encoding="ascii")

    res = out / "redrob_canvas_setup.res"
    run_in_env(["rc.exe", "/nologo", f"/I{out}", f"/I{HERE}", f"/fo{res}", str(HERE / "redrob_canvas_setup.rc")],
               env, str(HERE))
    exe = out / exe_name
    obj = out / "redrob_canvas_setup.obj"
    run_in_env(["cl.exe", "/nologo", "/EHsc", "/O2", "/W3", "/WX", "/MT", "/std:c++17", "/utf-8",
                "/DUNICODE", "/D_UNICODE", f"/I{out}", f"/Fo{obj}", str(HERE / "redrob_canvas_setup.cpp"),
                "/link", "/SUBSYSTEM:WINDOWS", "/MANIFEST:NO", str(res), f"/OUT:{exe}"],
               env, str(out))
    if not exe.exists():
        sys.exit(f"link produced no {exe_name}")
    return exe


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--payload", help="the release's windows zip (omit for a demo-only build)")
    ap.add_argument("--version", required=True, help="e.g. 0.5.6")
    ap.add_argument("--name", default="Redrob Canvas Setup.exe", help="output file name of the setup")
    ap.add_argument("--design-system", default=str(DEFAULT_DS))
    ap.add_argument("--uninstaller-only", action="store_true",
                    help="build only <out>/uninstall/Uninstall Redrob Canvas.exe, so it can be signed first")
    ap.add_argument("--uninstaller", help="an already built (and signed) uninstaller to embed, instead of "
                                          "building one")
    args = ap.parse_args()

    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        sys.exit(f"--version must be X.Y.Z, got {args.version!r}")
    for f in (BACKGROUND, LOCKUP_PNG, ICON):
        if not f.exists():
            sys.exit(f"missing committed asset {f}; run tools/build_installer_assets.py")

    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = vs_dev_env()
    font = out / "Pretendard-SemiBold.ttf"
    font = font if prepare_font(Path(args.design_system), font) else None

    if args.uninstaller_only:
        exe = compile_app(env, out / "uninstall", "Uninstall Redrob Canvas.exe", font, args.version,
                          payload=None, uninstaller=None)
    elif args.payload:
        payload = Path(args.payload).resolve()
        if not payload.is_file():
            sys.exit(f"payload not found: {payload}")
        if args.uninstaller:
            uninstaller = Path(args.uninstaller).resolve()
            if not uninstaller.is_file():
                sys.exit(f"uninstaller not found: {uninstaller}")
        else:
            uninstaller = compile_app(env, out / "uninstall", "Uninstall Redrob Canvas.exe", font, args.version,
                                      payload=None, uninstaller=None)
        exe = compile_app(env, out, args.name, font, args.version, payload=payload, uninstaller=uninstaller)
    else:
        exe = compile_app(env, out, args.name, font, args.version, payload=None, uninstaller=None)
    print(f"\nbuilt {exe}  {exe.stat().st_size // 1024} KB  (payload: {'yes' if args.payload else 'none'})")


if __name__ == "__main__":
    main()
