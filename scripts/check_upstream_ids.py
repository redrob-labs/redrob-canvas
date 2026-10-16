#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Refuse an upstream identifier a user could meet: a crate still named photocraft-*/vectorcraft-*/
designcraft-*/craft-*, a writer of the old file extensions, an old URI scheme, MIME type or env
prefix, or the upstream repository furniture (docs/, packaging/, README...) back under crafts/.

    scripts/check_upstream_ids.py

The three tools this calls each have a `--check` mode that reports instead of rewriting:
tools/prune_upstream.py, tools/rename_crates.py and tools/rename_identifiers.py. Attribution
files (NOTICE, COPYRIGHT, LICENSE-*, ATTRIBUTION.md, UPSTREAM_NOTICES.md, THIRD_PARTY_NOTICES.md)
and CHANGELOG.md are never scanned, and a line marked `// legacy:` (or the line beside it) is the
read path that still accepts the old value and is allowed. A `git subtree pull` brings every one
of these back; `python3 tools/merge_workspaces.py` runs the fixes, this runs the check.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import prune_upstream  # noqa: E402
import rename_crates  # noqa: E402
import rename_identifiers  # noqa: E402


def main() -> int:
    failed = 0
    for name, tool in (
        ("prune_upstream", prune_upstream),
        ("rename_crates", rename_crates),
        ("rename_identifiers", rename_identifiers),
    ):
        rc = tool.main(["--check"])
        if rc:
            print(f"error: tools/{name}.py --check failed; run python3 tools/{name}.py", file=sys.stderr)
            failed += 1
    if failed:
        return 1
    print("no upstream identifier outside attribution files and `// legacy:` read paths")
    return 0


if __name__ == "__main__":
    sys.exit(main())
