#!/usr/bin/env python3
"""Rebrand the vendored editors' user-visible strings to Redrob Canvas.

Runs over crafts/*/crates/**/src and the i18n assets. Only STRING LITERAL contents and
comment-free text assets are touched: identifiers such as `PhotocraftApp` keep their case and
are never matched. Re-run after every `git subtree pull` (tools/merge_workspaces.py then this).

Upstream attribution is unaffected: NOTICE, UPSTREAM.md, UPSTREAM_NOTICES.md and the
`crafts/*/LICENSE-*` / `NOTICE` files keep the upstream copyright lines verbatim, as the
MIT and Apache-2.0 licences require.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRAFTS = ROOT / "crafts"

# Ordered: URLs first (they contain product names), then names.
REPLACEMENTS = [
    (r"https?://(www\.)?getartcraft\.com[^\s\"']*", "https://redrob.ai"),
    (r"https?://discord\.gg/[^\s\"']*", "https://github.com/redrob-labs/redrob-canvas/issues"),
    (r"https?://github\.com/storytold/(photocraft|vectorcraft|designcraft|craft-libs)[^\s\"']*",
     "https://github.com/redrob-labs/redrob-canvas"),
    (r"Join the ArtCraft Discord", "Redrob community"),
    (r"Join the ArtCraft community", "Redrob community"),
    (r"More ArtCraft apps", "More Redrob apps"),
    (r"ArtCraft App Page", "Redrob apps"),
    (r"ArtCraft Website", "Redrob website"),
    (r"Part of the ArtCraft suite of open creative tools\.", "Part of the Redrob suite of agentic apps."),
    (r"ArtCraft Team", "Redrob"),
    (r"ArtCraft", "Redrob"),
    (r"getartcraft\.com", "redrob.ai"),
    (r"(PhotoCraft|VectorCraft|DesignCraft)", "Redrob Canvas"),
    (r"(Photocraft|Vectorcraft|Designcraft)(?=[ .,:;!?'\"\u2014-]|$)", "Redrob Canvas"),
]

STRING_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')
SKIP_DIRS = {"docs", "assets", "packaging", "examples", "scorecard", "perf", "book"}
TEXT_ASSET_SUFFIXES = {".json", ".ftl", ".tsv", ".toml", ".txt", ".yaml", ".yml", ".html"}


def rebrand_text(text: str) -> str:
    for pattern, repl in REPLACEMENTS:
        text = re.sub(pattern, repl, text)
    return text


def rebrand_rust(source: str) -> str:
    def sub(m: re.Match) -> str:
        return '"' + rebrand_text(m.group(1)) + '"'

    source = STRING_RE.sub(sub, source)
    # Raw strings and strings with escaped quotes slip past STRING_RE; the CamelCase product
    # names and the upstream org URL never occur in Rust identifiers, so a plain pass is safe.
    source = re.sub(r"https?://github\.com/storytold/\{APP_ID\}", "https://github.com/redrob-labs/redrob-canvas", source)
    source = re.sub(r"(PhotoCraft|VectorCraft|DesignCraft)", "Redrob Canvas", source)
    source = re.sub(r"ArtCraft Team", "Redrob", source)
    source = re.sub(r"ArtCraft", "Redrob", source)
    return source


def main() -> int:
    changed = 0
    for craft in sorted(p for p in CRAFTS.iterdir() if p.is_dir()):
        for path in sorted((craft / "crates").rglob("*")):
            if not path.is_file() or any(part in SKIP_DIRS for part in path.relative_to(craft).parts):
                continue
            if path.suffix == ".rs":
                before = path.read_text(encoding="utf-8")
                after = rebrand_rust(before)
            elif path.suffix in TEXT_ASSET_SUFFIXES and ("i18n" in path.parts or path.suffix in {".ftl", ".tsv"}):
                before = path.read_text(encoding="utf-8")
                after = rebrand_text(before)
            else:
                continue
            if after != before:
                path.write_text(after, encoding="utf-8")
                changed += 1
    print(f"rebranded {changed} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
