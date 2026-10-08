#!/usr/bin/env python3
"""Emit the release build matrix, the platform archive names the draft must therefore carry,
and the release-note line describing what was built -- all from ONE list of legs.

Two things drove this out of the workflow file. First, a static matrix cannot skip a single
leg, and the Windows leg has to be skippable: Authenticode signing needs a certificate that is
not configured yet, and a leg that fails on every tag makes the whole release red, which hides
real regressions instead of surfacing them. Second, the verify job hardcoded the per-platform
archive suffixes it demanded and the release notes hardcoded the platform list they promised,
so skipping a leg would have made a correct gate fail for the wrong reason and left the notes
claiming a binary nobody can download.

Windows is SKIPPED rather than built unsigned. An unsigned executable that reaches a user is
worse than none: it trains people to click through the warning.
"""
import json
import os
import sys

LEGS = [
    {
        "label": "Linux x86_64",
        "runner": "ubuntu-24.04",
        "aqt_host": "linux",
        "aqt_arch": "linux_gcc_64",
        "qt_dir": "gcc_64",
        "slug": "linux-x86_64",
        "archive_suffix": "linux-x86_64.tar.gz",
        "notes": "Linux x86_64",
    },
    # Qt's macOS build is universal (arm64 + x86_64), so one runner covers both architectures
    # and the bundle is genuinely universal rather than two halves.
    {
        # arm64, not universal: the binary was only ever built for the runner's own Apple silicon
        # (`file` on the 0.5.2 bundle says "Mach-O 64-bit arm64"), so the old "universal" name
        # promised Intel Macs a build that does not start on them.
        "label": "macOS arm64",
        "runner": "macos-15",
        "aqt_host": "mac",
        "aqt_arch": "clang_64",
        "qt_dir": "macos",
        "slug": "macos-arm64",
        "archive_suffix": "macos-arm64.zip",
        "notes": "macOS arm64 for Apple silicon (signed, notarized, stapled)",
    },
    {
        "label": "Windows x64",
        "runner": "windows-2025",
        "aqt_host": "windows",
        "aqt_arch": "win64_msvc2022_64",
        "qt_dir": "msvc2022_64",
        "slug": "windows-x86_64",
        "archive_suffix": "windows-x86_64.zip",
        "notes": "Windows x64 (signed and timestamped)",
        "requires_windows_signing": True,
    },
]

MATRIX_KEYS = ("label", "runner", "aqt_host", "aqt_arch", "qt_dir", "slug")

windows_signing_ready = (
    os.environ.get("WINDOWS_SIGNING_READY", "").strip().lower() == "true"
)

included = [
    leg
    for leg in LEGS
    if not leg.get("requires_windows_signing") or windows_signing_ready
]
skipped = [leg for leg in LEGS if leg not in included]

for leg in skipped:
    print(
        f"::notice title=Platform skipped::{leg['label']} is not built because the "
        'repository variable WINDOWS_SIGNING_READY is not "true". Set it once a '
        "code-signing certificate is in WIN_CSC_LINK; nothing else needs to change."
    )

if not included:
    print("Every platform is excluded; there would be nothing to release.", file=sys.stderr)
    sys.exit(1)

matrix = [{key: leg[key] for key in MATRIX_KEYS} for leg in included]
archive_suffixes = [leg["archive_suffix"] for leg in included]
notes_line = ", ".join(leg["notes"] for leg in included)

if sys.argv[1:] == ["--notes"]:
    # tools/release/local-release.sh writes the draft's notes from this same list.
    print(notes_line)
    sys.exit(0)

print(f"Building: {', '.join(leg['label'] for leg in included)}")
print(f"Platform archives required on the draft: {' '.join(archive_suffixes)}")

output = os.environ.get("GITHUB_OUTPUT")
if output:
    with open(output, "a", encoding="utf-8") as handle:
        handle.write(f"matrix={json.dumps(matrix, separators=(',', ':'))}\n")
        handle.write(f"archive_suffixes={' '.join(archive_suffixes)}\n")
        handle.write(f"notes_platforms={notes_line}\n")
        handle.write(f"windows_signing_ready={str(windows_signing_ready).lower()}\n")
