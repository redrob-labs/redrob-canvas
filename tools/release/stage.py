#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Stage one platform's release tree from the built `redrob-canvas` binary.

    stage.py --target <rust-triple> --version X.Y.Z --out <dir> [--binary <path>]

The tree every archive, package and installer is made from (release.yml, build-linux.sh):

  Linux / Windows                        macOS
    bin/redrob-canvas[.exe]                Redrob Canvas.app/
    share/redrob-canvas/licenses/            Contents/Info.plist
      LICENSE-MIT  LICENSE-APACHE            Contents/MacOS/redrob-canvas
      COPYRIGHT  NOTICE                      Contents/Resources/AppIcon.icns
      THIRD_PARTY_NOTICES.md                 Contents/Resources/licenses/...
      SOURCE_ORIGINS.md                    share/redrob-canvas/licenses/   (the same set, for the dmg)

The licences travel with every copy: MIT asks the notice to be retained in every copy and Apache 4(d)
asks the NOTICE to travel with every distribution, so they are inside the bundle as well as beside it.
The binary defaults to target/<target>/release/redrob-canvas[.exe]; --binary overrides it.

The Info.plist is written here rather than committed so the version in it is the workspace version
the build used; release.yml reads it back and refuses a mismatch. It also declares the document types
(`document_types`), read from apps/redrob-canvas/src/routing.rs, so Finder opens them in Canvas.
"""
from __future__ import annotations

import argparse
import plistlib
import re
import shutil
import stat
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LICENSES = (
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "COPYRIGHT",
    "NOTICE",
    "THIRD_PARTY_NOTICES.md",
    "SOURCE_ORIGINS.md",
)
ICNS = ROOT / "resources" / "icons" / "redrob-canvas.icns"
BUNDLE_ID = "ai.redrob.canvas"
APP_NAME = "Redrob Canvas"
BINARY = "redrob-canvas"
ROUTING = ROOT / "apps" / "redrob-canvas" / "src" / "routing.rs"

# Finder's double-click and "Open With". The three native documents are ours: exported types,
# Canvas the Owner. Every other kind Canvas opens (routing.rs) is Alternate, so Canvas shows under
# "Open With" without taking another app's default.
OWN_TYPES = (
    ("rlayout", "Redrob Canvas Layout"),
    ("rvector", "Redrob Canvas Vector"),
    ("rphoto", "Redrob Canvas Photo"),
)
# The system's own type identifiers for the extensions macOS declares itself. An extension not here
# is matched by its extension (CFBundleTypeExtensions), except idml, imported below.
SYSTEM_UTIS = {
    "png": "public.png",
    "jpg": "public.jpeg",
    "jpeg": "public.jpeg",
    "tif": "public.tiff",
    "tiff": "public.tiff",
    "gif": "com.compuserve.gif",
    "bmp": "com.microsoft.bmp",
    "ico": "com.microsoft.ico",
    "heic": "public.heic",
    "heif": "public.heif",
    "avif": "public.avif",
    "webp": "org.webmproject.webp",
    "psd": "com.adobe.photoshop-image",
    "tga": "com.truevision.tga-image",
    "exr": "com.ilm.openexr-image",
    "dng": "com.adobe.raw-image",
    "cr2": "com.canon.cr2-raw-image",
    "cr3": "com.canon.cr3-raw-image",
    "nef": "com.nikon.raw-image",
    "nrw": "com.nikon.nrw-raw-image",
    "arw": "com.sony.arw-raw-image",
    "raf": "com.fuji.raw-image",
    "orf": "com.olympus.raw-image",
    "rw2": "com.panasonic.rw2-raw-image",
    "pef": "com.pentax.raw-image",
    "svg": "public.svg-image",
    "pdf": "com.adobe.pdf",
    "ai": "com.adobe.illustrator.ai-image",
    "eps": "com.adobe.encapsulated-postscript",
    "docx": "org.openxmlformats.wordprocessingml.document",
    "xlsx": "org.openxmlformats.spreadsheetml.sheet",
    "rtf": "public.rtf",
}
# Types macOS does not declare but other apps name the same way.
IMPORTED_TYPES = {
    "idml": ("com.adobe.indesign-idml", "InDesign Markup Document", ["public.zip-archive", "public.data"]),
}
ROUTING_LISTS = (("PHOTO_EXTENSIONS", "Photo"), ("VECTOR_EXTENSIONS", "Vector"), ("LAYOUT_EXTENSIONS", "Layout"))


def copy_licenses(into: Path) -> None:
    into.mkdir(parents=True, exist_ok=True)
    for name in LICENSES:
        source = ROOT / name
        if not source.is_file():
            sys.exit(f"missing {source}; the licence set must be complete before staging")
        shutil.copyfile(source, into / name)


def executable(path: Path) -> None:
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def stage_unix_or_windows(binary: Path, out: Path, windows: bool) -> None:
    bin_dir = out / "bin"
    bin_dir.mkdir(parents=True)
    dest = bin_dir / (f"{BINARY}.exe" if windows else BINARY)
    shutil.copyfile(binary, dest)
    if not windows:
        executable(dest)
    copy_licenses(out / "share" / "redrob-canvas" / "licenses")


def routed_extensions() -> list[tuple[str, list[str]]]:
    """Each mode's extensions as routing.rs lists them (the one list the app routes by)."""
    text = ROUTING.read_text(encoding="utf-8").split("#[cfg(test)]")[0]
    lists = []
    for const, mode in ROUTING_LISTS:
        match = re.search(rf"pub const {const}: &\[&str\] = &\[(.*?)\];", text, re.S)
        if not match:
            sys.exit(f"{ROUTING}: no {const}; the document types are read from it")
        body = re.sub(r"//[^\n]*", "", match.group(1))
        lists.append((mode, re.findall(r'"([a-z0-9]+)"', body)))
    return lists


def document_types() -> tuple[list[dict], list[dict], list[dict]]:
    """CFBundleDocumentTypes, UTExportedTypeDeclarations and UTImportedTypeDeclarations."""
    own = {ext for ext, _ in OWN_TYPES}
    documents: list[dict] = []
    exported: list[dict] = []
    for ext, name in OWN_TYPES:
        uti = f"{BUNDLE_ID}.{ext}"
        exported.append(
            {
                "UTTypeIdentifier": uti,
                "UTTypeDescription": name,
                "UTTypeConformsTo": ["public.data", "public.content"],
                "UTTypeIconFile": "AppIcon",
                "UTTypeTagSpecification": {"public.filename-extension": [ext]},
            }
        )
        documents.append(
            {
                "CFBundleTypeName": name,
                "CFBundleTypeRole": "Editor",
                "LSHandlerRank": "Owner",
                "CFBundleTypeIconFile": "AppIcon",
                "LSItemContentTypes": [uti],
            }
        )
    imported = [
        {
            "UTTypeIdentifier": uti,
            "UTTypeDescription": name,
            "UTTypeConformsTo": conforms,
            "UTTypeTagSpecification": {"public.filename-extension": [ext]},
        }
        for ext, (uti, name, conforms) in IMPORTED_TYPES.items()
    ]
    for mode, extensions in routed_extensions():
        others = [e for e in extensions if e not in own]
        utis: list[str] = []
        for e in others:
            uti = SYSTEM_UTIS.get(e) or IMPORTED_TYPES.get(e, (None,))[0]
            if uti and uti not in utis:
                utis.append(uti)
        by_extension = [e for e in others if e not in SYSTEM_UTIS and e not in IMPORTED_TYPES]
        # A document type with LSItemContentTypes ignores CFBundleTypeExtensions, so the two are
        # separate entries.
        if utis:
            documents.append(
                {
                    "CFBundleTypeName": f"{mode} document",
                    "CFBundleTypeRole": "Editor",
                    "LSHandlerRank": "Alternate",
                    "LSItemContentTypes": utis,
                }
            )
        if by_extension:
            documents.append(
                {
                    "CFBundleTypeName": f"Other {mode.lower()} document",
                    "CFBundleTypeRole": "Editor",
                    "LSHandlerRank": "Alternate",
                    "CFBundleTypeExtensions": by_extension,
                }
            )
    return documents, exported, imported


def stage_macos(binary: Path, out: Path, version: str) -> None:
    app = out / f"{APP_NAME}.app"
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir(parents=True)
    dest = contents / "MacOS" / BINARY
    shutil.copyfile(binary, dest)
    executable(dest)
    if not ICNS.is_file():
        sys.exit(f"missing {ICNS}; run scripts/build-brand-assets.py")
    shutil.copyfile(ICNS, contents / "Resources" / "AppIcon.icns")
    copy_licenses(contents / "Resources" / "licenses")
    copy_licenses(out / "share" / "redrob-canvas" / "licenses")
    info = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": APP_NAME,
        "CFBundleExecutable": BINARY,
        "CFBundleIconFile": "AppIcon",
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": APP_NAME,
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSMinimumSystemVersion": "12.0",
        "LSApplicationCategoryType": "public.app-category.graphics-design",
        "NSHighResolutionCapable": True,
        "NSHumanReadableCopyright": "Copyright 2026 Janghoon Lee (Redrob) and contributors. MIT OR Apache-2.0.",
        "NSSupportsAutomaticGraphicsSwitching": True,
    }
    documents, exported, imported = document_types()
    info["CFBundleDocumentTypes"] = documents
    info["UTExportedTypeDeclarations"] = exported
    info["UTImportedTypeDeclarations"] = imported
    with open(contents / "Info.plist", "wb") as handle:
        plistlib.dump(info, handle, sort_keys=True)
    (contents / "PkgInfo").write_text("APPL????", encoding="ascii")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", required=True, help="the Rust target triple that was built")
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--binary", type=Path, help="the built binary (default target/<target>/release/)")
    args = parser.parse_args()

    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", args.version):
        sys.exit(f"--version must be X.Y.Z, got {args.version!r}")
    windows = "windows" in args.target
    macos = "apple-darwin" in args.target
    binary = args.binary or (
        ROOT / "target" / args.target / "release" / (f"{BINARY}.exe" if windows else BINARY)
    )
    if not binary.is_file():
        sys.exit(f"binary not found: {binary}; run cargo build --release -p redrob-canvas --target {args.target}")

    out = args.out.resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    if macos:
        stage_macos(binary, out, args.version)
    else:
        stage_unix_or_windows(binary, out, windows)
    print(f"staged {args.target} {args.version} into {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
