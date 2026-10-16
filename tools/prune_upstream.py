#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Delete the upstream repository furniture a `git subtree pull` brings back under crafts/<name>/.

Each vendored editor arrives as a whole repository: its own docs, book, packaging, CI, READMEs,
examples, xtask and scripts. None of that is built here (the one shell is apps/redrob-canvas and
only each editor's `crates/*` are workspace members), and every one of those files carries the
upstream product name, so they are removed. Idempotent: run it after every subtree pull
(tools/merge_workspaces.py calls it).

What stays, and why:
  - LICENSE-APACHE, LICENSE-MIT, NOTICE, ATTRIBUTION.md: the licences require them (UPSTREAM.md).
  - assets/, vendor/, licenses/: `include_bytes!` fonts and icons, the vendored krilla/hayro
    crates the root `[patch]` points at, and the licence texts of what vendor/ carries.
  - contributors/: each ui-egui crate's build.rs bakes these credits into the About window.
  - clippy.toml, rustfmt.toml, .gitignore: build configuration rustfmt and clippy read per subtree.
  - Cargo.upstream.toml, crates/, apps/: the parked manifest and the code.
  - xtask/affinity-corpus.sha256 and xtask/src/sha256.rs: `include_str!` and `#[path]` from the
    vectorcraft affinity tests.

    python3 tools/prune_upstream.py            # delete
    python3 tools/prune_upstream.py --check    # exit 1 and list what a pull brought back
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRAFTS = ["photocraft", "vectorcraft", "designcraft", "craft-libs"]

# Directories and files removed from each crafts/<name>/ root. The build references nothing here
# (grep before adding an entry: `include_str!`/`include_bytes!` paths in crates/ and build.rs).
PRUNE = [
    ".cargo",
    ".config",
    ".dockerignore",
    ".git-blame-ignore-revs",
    ".github",
    "AGENTS.md",
    "ASSETS.md",
    "CLAUDE.md",
    "CONTRIBUTING.md",
    "Cargo.lock",
    "Dockerfile",
    "README.md",
    "ROADMAP.md",
    "SECURITY.md",
    "book",
    "docs",
    "examples",
    "packaging",
    "perf",
    "scorecard",
    "scripts",
    "xtask",
]

# Files inside a pruned directory that the build still reads; the directory is emptied around them.
KEEP = {
    # include_str! from the affinity tests, and `#[path]`-included by tests/support/corpus.rs.
    "vectorcraft": ["xtask/affinity-corpus.sha256", "xtask/src/sha256.rs"],
}


def targets() -> list[Path]:
    out: list[Path] = []
    for craft in CRAFTS:
        base = ROOT / "crafts" / craft
        keep = {base / rel for rel in KEEP.get(craft, [])}
        for entry in PRUNE:
            path = base / entry
            if not path.exists() and not path.is_symlink():
                continue
            if path.is_dir() and not path.is_symlink():
                kept_inside = [k for k in keep if k.is_relative_to(path)]
                if kept_inside:
                    for child in sorted(path.rglob("*")):
                        if child.is_file() and child not in keep:
                            out.append(child)
                    continue
            out.append(path)
    return out


def main(argv: list[str]) -> int:
    check = "--check" in argv
    found = targets()
    if check:
        for path in found:
            print(f"error: upstream furniture present: {path.relative_to(ROOT)}", file=sys.stderr)
        if found:
            print(f"{len(found)} upstream files or directories are back; run tools/prune_upstream.py", file=sys.stderr)
            return 1
        print("no upstream furniture under crafts/")
        return 0
    for path in found:
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()
        print(f"removed {path.relative_to(ROOT)}")
    # A KEEP exception empties the directories around it; drop the empty ones.
    for craft, rels in KEEP.items():
        for rel in rels:
            top = ROOT / "crafts" / craft / Path(rel).parts[0]
            if top.is_dir():
                for d in sorted((p for p in top.rglob("*") if p.is_dir()), reverse=True):
                    if not any(d.iterdir()):
                        d.rmdir()
    print(f"pruned {len(found)} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
