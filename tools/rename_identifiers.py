#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rename the upstream identifiers a user can meet: file extensions, URI schemes, MIME types and
environment-variable prefixes. Companion of tools/rename_crates.py (crate names) and
tools/rebrand_crafts.py (display strings); like them it is idempotent and re-run after every
`git subtree pull` (tools/merge_workspaces.py calls it).

    extension   .pcraft / .photocraft   -> .rphoto      .vectorcraft -> .rvector     .designcraft -> .rlayout
    URI scheme  photocraft://           -> redrob-photo://   vectorcraft:// -> redrob-vector://
                designcraft://          -> redrob-layout://
    MIME        application/x-photocraft -> application/x-redrob-photo  (vector, layout alike)
                application/vnd.designcraft+zip -> application/vnd.redrob-layout+zip
    env         PHOTOCRAFT_* -> REDROB_PHOTO_*   VECTORCRAFT_* -> REDROB_VECTOR_*
                DESIGNCRAFT_* -> REDROB_LAYOUT_*  CRAFT_* -> REDROB_CRAFT_*

Old files still open and old URIs, MIME types and environment variables are still honoured: the
READ paths that accept the upstream value are marked with a `// legacy:` comment; a line that
carries that marker, or whose neighbour does, is never rewritten here and is whitelisted by
scripts/check_upstream_ids.py.
Writers emit only the Redrob value.

Not renamed (internal, not something a user types): the format ids the engines use in their
command parameters and file headers (`"format": "vectorcraft"`), app ids, settings directories.

    python3 tools/rename_identifiers.py            # rewrite
    python3 tools/rename_identifiers.py --check    # exit 1 and list files still carrying one
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LEGACY_MARKER = "// legacy:"

RULES: list[tuple[re.Pattern[str], str]] = [
    # URI schemes and MIME types first: they contain the product names.
    (re.compile(r"\bphotocraft://"), "redrob-photo://"),
    (re.compile(r"\bvectorcraft://"), "redrob-vector://"),
    (re.compile(r"\bdesigncraft://"), "redrob-layout://"),
    (re.compile(r"application/x-photocraft\b"), "application/x-redrob-photo"),
    (re.compile(r"application/x-vectorcraft\b"), "application/x-redrob-vector"),
    (re.compile(r"application/x-designcraft\b"), "application/x-redrob-layout"),
    (re.compile(r"application/vnd\.designcraft\+zip"), "application/vnd.redrob-layout+zip"),
    (re.compile(r"x-scheme-handler/photocraft\b"), "x-scheme-handler/redrob-photo"),
    (re.compile(r"x-scheme-handler/vectorcraft\b"), "x-scheme-handler/redrob-vector"),
    (re.compile(r"x-scheme-handler/designcraft\b"), "x-scheme-handler/redrob-layout"),
    # File extensions. `pcraft` only ever names the Photo document (`.pcraft`, `"pcraft"`,
    # `pcraft|psd`); `is_pcraft` and the like are identifiers and are not word-bounded here.
    (re.compile(r"(?<!\w)pcraft(?!\w)"), "rphoto"),
    (re.compile(r"\.photocraft\b"), ".rphoto"),
    (re.compile(r"\.vectorcraft\b"), ".rvector"),
    (re.compile(r"\.designcraft\b"), ".rlayout"),
    # Environment variables (and the constants named after them).
    (re.compile(r"\bPHOTOCRAFT_(?=[A-Z0-9])"), "REDROB_PHOTO_"),
    (re.compile(r"\bVECTORCRAFT_(?=[A-Z0-9])"), "REDROB_VECTOR_"),
    (re.compile(r"\bDESIGNCRAFT_(?=[A-Z0-9])"), "REDROB_LAYOUT_"),
    (re.compile(r"\bCRAFT_(?=[A-Z0-9])"), "REDROB_CRAFT_"),
]

# Where identifiers live: the editors' code and text assets, our shell and crates, packaging. Prose
# (README, docs/, UPSTREAM.md) is not scanned: it is allowed to name the old extensions it says still open.
SCAN = ["crafts", "apps", "crates", "installer", "resources", "tools/release"]
TEXT_SUFFIXES = {".rs", ".toml", ".md", ".txt", ".json", ".yml", ".yaml", ".sh", ".py", ".ftl", ".tsv",
                 ".html", ".js", ".wgsl", ".in", ".xml", ".plist", ".desktop", ".wxs", ".ps1", ".ron"}
SKIP_DIRS = {".git", "target"}
SKIP_FILES = {"NOTICE", "COPYRIGHT", "LICENSE-APACHE", "LICENSE-MIT", "LICENSE", "ATTRIBUTION.md",
              "UPSTREAM_NOTICES.md", "CHANGELOG.md", "THIRD_PARTY_NOTICES.md", "Cargo.lock"}


def is_legacy(lines: list[str], i: int) -> bool:
    """A line is a legacy read path when it, the line before it or the line after it carries the
    marker: rustfmt moves a trailing comment off a line it breaks, so the marker may sit on the
    neighbour."""
    return any(LEGACY_MARKER in lines[j] for j in (i - 1, i, i + 1) if 0 <= j < len(lines))


def rewrite(text: str) -> str:
    lines = text.splitlines(keepends=True)
    out = []
    for i, line in enumerate(lines):
        if not is_legacy(lines, i):
            for pattern, repl in RULES:
                line = pattern.sub(repl, line)
        out.append(line)
    return "".join(out)


def files() -> list[Path]:
    found: list[Path] = []
    for entry in SCAN:
        base = ROOT / entry
        if base.is_file():
            found.append(base)
            continue
        for path in base.rglob("*"):
            if any(part in SKIP_DIRS for part in path.relative_to(ROOT).parts):
                continue
            if path.is_file() and path.suffix in TEXT_SUFFIXES and path.name not in SKIP_FILES:
                found.append(path)
    return found


def main(argv: list[str]) -> int:
    check = "--check" in argv
    changed: list[Path] = []
    for path in files():
        try:
            before = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        after = rewrite(before)
        if after != before:
            changed.append(path)
            if not check:
                path.write_text(after, encoding="utf-8")
    if check:
        for path in changed:
            print(f"error: upstream identifier in {path.relative_to(ROOT)}", file=sys.stderr)
        if changed:
            print(f"{len(changed)} files carry an upstream identifier; run tools/rename_identifiers.py", file=sys.stderr)
            return 1
        print("no upstream file extension, URI scheme, MIME type or env prefix outside `// legacy:` lines")
        return 0
    print(f"renamed identifiers in {len(changed)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
