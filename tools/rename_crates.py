#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Rename the vendored editors' crates to the Redrob names, everywhere a crate name appears.

    photocraft-<x>   -> redrob-photo-<x>      (crafts/photocraft/crates/*)
    vectorcraft-<x>  -> redrob-vector-<x>     (crafts/vectorcraft/crates/*)
    designcraft-<x>  -> redrob-layout-<x>     (crafts/designcraft/crates/*)
    craft-<x>        -> redrob-craft-<x>      (crafts/craft-libs/crates/*)

and the `_` forms Rust uses in paths (`use redrob_photo_engine::` -> `use redrob_photo_engine::`).
The crate list is read from the subtree manifests, so a crate upstream adds is renamed the first
time this runs after the pull. Rewrites `[package].name`, dependency keys, `use` paths, feature
strings, the parked Cargo.upstream.toml files, tools/, scripts/, .github/, docs and Cargo.lock.
Idempotent: a renamed tree is left alone (the upstream names do not occur in the new ones, and the
craft-libs pattern refuses a `redrob-` prefix), so it runs after every `git subtree pull`
(tools/merge_workspaces.py calls it, then `cargo update --workspace --offline` re-sorts the lock).

Crate DIRECTORIES are not named after their crate upstream (`crates/engine` holds
`redrob-photo-engine`), so nothing is `git mv`ed. The excluded upstream binaries under
`crafts/*/apps/` (`photocraft`, `photocraft-cli`, `photocraft-web`, ...) keep their names: they are
not workspace members and their directories carry the name.

Attribution files are never touched: NOTICE, COPYRIGHT, LICENSE-*, ATTRIBUTION.md, UPSTREAM_NOTICES.md
and CHANGELOG.md (history). THIRD_PARTY_NOTICES.md is generated; regenerate it instead.

    python3 tools/rename_crates.py            # rewrite
    python3 tools/rename_crates.py --check    # exit 1 and list files still carrying an upstream crate name
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# (subtree, upstream crate prefix, Redrob crate prefix)
FAMILIES = [
    ("photocraft", "photocraft", "redrob-photo"),
    ("vectorcraft", "vectorcraft", "redrob-vector"),
    ("designcraft", "designcraft", "redrob-layout"),
    ("craft-libs", "craft", "redrob-craft"),
]

TEXT_SUFFIXES = {
    ".rs", ".toml", ".lock", ".md", ".txt", ".json", ".yml", ".yaml", ".sh", ".py", ".ftl", ".tsv",
    ".html", ".js", ".wgsl", ".in", ".xml", ".plist", ".desktop", ".wxs", ".ps1", ".cfg", ".ini",
    ".svg", ".csv", ".ron",
}
SKIP_DIRS = {".git", "target", "node_modules", "dist"}
SKIP_FILES = {
    "NOTICE", "COPYRIGHT", "LICENSE-APACHE", "LICENSE-MIT", "LICENSE", "ATTRIBUTION.md",
    "UPSTREAM_NOTICES.md", "CHANGELOG.md", "THIRD_PARTY_NOTICES.md",
}


def crate_suffixes() -> dict[str, list[str]]:
    """Per family, the `<x>` of every `<prefix>-<x>` crate under crafts/<subtree>/crates/*."""
    out: dict[str, list[str]] = {}
    for subtree, old, new in FAMILIES:
        suffixes: set[str] = set()
        for manifest in (ROOT / "crafts" / subtree / "crates").glob("*/Cargo.toml"):
            try:
                name = tomllib.load(manifest.open("rb"))["package"]["name"]
            except (KeyError, tomllib.TOMLDecodeError):
                continue
            for prefix in (old, new):
                if name.startswith(prefix + "-"):
                    suffixes.add(name[len(prefix) + 1:])
        # Upstream's own workspace table may name a crate its tree no longer has (designcraft
        # declares `redrob-layout-testkit` with no crates/testkit); the key is still a crate name.
        for manifest_name in ("Cargo.upstream.toml", "Cargo.toml"):
            parked = ROOT / "crafts" / subtree / manifest_name
            if not parked.is_file():
                continue
            try:
                table = tomllib.load(parked.open("rb")).get("workspace", {}).get("dependencies", {})
            except tomllib.TOMLDecodeError:
                continue
            for name, spec in table.items():
                if isinstance(spec, dict) and str(spec.get("path", "")).startswith("crates/"):
                    for prefix in (old, new):
                        if name.startswith(prefix + "-"):
                            suffixes.add(name[len(prefix) + 1:])
        out[subtree] = sorted(suffixes, key=len, reverse=True)
    return out


def build_rules() -> list[tuple[re.Pattern[str], str]]:
    rules: list[tuple[re.Pattern[str], str]] = []
    suffixes = crate_suffixes()
    for subtree, old, new in FAMILIES:
        if not suffixes[subtree]:
            continue
        hy = "|".join(re.escape(s) for s in suffixes[subtree])
        us = "|".join(re.escape(s.replace("-", "_")) for s in suffixes[subtree])
        old_us, new_us = old.replace("-", "_"), new.replace("-", "_")
        # `(?<![\w-])`: not inside another crate's name (`photocraft-ui` is not `redrob-craft-ui`) and not
        # already prefixed (`redrob-craft-ui`). `(?![\w-])`: the whole crate name, not a prefix of
        # a longer one (`designcraft-text` vs `redrob-layout-textimport` is handled by the alternation
        # being longest-first as well).
        rules.append((re.compile(rf"(?<![\w-]){re.escape(old)}-({hy})(?![\w-])"), rf"{new}-\1"))
        rules.append((re.compile(rf"(?<!\w){re.escape(old_us)}_({us})(?!\w)"), rf"{new_us}_\1"))
    return rules


def candidate_files() -> list[Path]:
    files: list[Path] = []
    for path in ROOT.rglob("*"):
        if any(part in SKIP_DIRS for part in path.relative_to(ROOT).parts):
            continue
        if not path.is_file() or path.name in SKIP_FILES:
            continue
        if path.suffix in TEXT_SUFFIXES or path.name in {"Cargo.lock", "Dockerfile", "Makefile"}:
            files.append(path)
    return files


def main(argv: list[str]) -> int:
    check = "--check" in argv
    rules = build_rules()
    if not rules:
        print("no crates to rename (no crafts/*/crates manifests found)", file=sys.stderr)
        return 1
    changed: list[Path] = []
    for path in candidate_files():
        try:
            before = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        after = before
        for pattern, repl in rules:
            after = pattern.sub(repl, after)
        if after != before:
            changed.append(path)
            if not check:
                path.write_text(after, encoding="utf-8")
    if check:
        for path in changed:
            print(f"error: upstream crate name in {path.relative_to(ROOT)}", file=sys.stderr)
        if changed:
            print(f"{len(changed)} files carry an upstream crate name; run tools/rename_crates.py", file=sys.stderr)
            return 1
        print("every crate carries its Redrob name")
        return 0
    print(f"renamed crates in {len(changed)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
