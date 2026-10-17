#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Write or check `redrob-launcher.json`, the one asset Redrob Launcher reads to learn what a
release of this app can do.

    tools/release_manifest.py --write <release-dir>   # before the checksums and the upload
    tools/release_manifest.py --check <release-dir>   # fails unless the file is there and right

The launcher fetches `redrob-launcher.json` from the release's own download URL and reads two
fields: `schema` (1) and `sharedAccount`. `sharedAccount: true` says this release reads the
launcher's shared account file, so signing in to the launcher is all a person needs; it has been
true since the shared sign-in landed in develop. The launcher's quota-free path learns the asset
names from the SHA256SUMS file, so the manifest is written BEFORE the sums are computed and is
listed in them like every other asset.

The content is a constant, not a template: the launcher compares it field by field, and a
release that carried a different shape would silently fall back to the launcher's built-in table.
The published copy carries one more field, `"icon": "redrob-canvas.png"`: the shared publish job
(release-publish.yml, `launcher_icon`) writes it over this file once the 512 px app icon is among
the release assets, so the launcher's catalog shows this release's own logo. `--check` accepts
either shape; it never checks for the icon because the local Linux build has no such asset.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys

NAME = "redrob-launcher.json"
SCHEMA = 1
# True only because develop carries the shared-account change (the agent panel's
# "Signed in with Redrob Launcher"). A release without that change must say false.
SHARED_ACCOUNT = True
CONTENT = '{ "schema": %d, "sharedAccount": %s }\n' % (SCHEMA, "true" if SHARED_ACCOUNT else "false")


def write(release_dir: Path) -> Path:
    """Write the manifest into `release_dir` and return its path."""
    release_dir.mkdir(parents=True, exist_ok=True)
    path = release_dir / NAME
    path.write_text(CONTENT, encoding="utf-8")
    return path


def problems(release_dir: Path) -> list[str]:
    """Why `release_dir` is not a release directory the launcher can read. Empty means it is."""
    path = release_dir / NAME
    if not path.is_file():
        return [f"{NAME} is missing from {release_dir}; run tools/release_manifest.py --write before the checksums"]
    text = path.read_text(encoding="utf-8")
    found: list[str] = []
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        return [f"{NAME} is not JSON: {exc}"]
    if not isinstance(data, dict):
        return [f"{NAME} must be a JSON object, got {type(data).__name__}"]
    if data.get("schema") != SCHEMA:
        found.append(f"{NAME}: schema must be {SCHEMA}, got {data.get('schema')!r}")
    if data.get("sharedAccount") is not SHARED_ACCOUNT:
        found.append(f"{NAME}: sharedAccount must be {str(SHARED_ACCOUNT).lower()}, got {data.get('sharedAccount')!r}")
    if not text.endswith("\n"):
        found.append(f"{NAME} is not newline-terminated")
    sums = sorted(release_dir.glob("*SHA256SUMS.txt"))
    for sums_file in sums:
        listed = {line.split()[-1].lstrip("*").removeprefix("./") for line in sums_file.read_text(encoding="utf-8").splitlines() if line.strip()}
        if NAME not in listed:
            found.append(f"{sums_file.name} does not list {NAME}; write the manifest before computing the sums")
    return found


def main(argv: list[str]) -> int:
    if len(argv) != 2 or argv[0] not in ("--write", "--check"):
        print(__doc__, file=sys.stderr)
        return 2
    release_dir = Path(argv[1])
    if argv[0] == "--write":
        print(write(release_dir))
        return 0
    found = problems(release_dir)
    for line in found:
        print(f"::error::{line}")
    if not found:
        print(f"{release_dir / NAME} is present, schema {SCHEMA}, sharedAccount {str(SHARED_ACCOUNT).lower()}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
