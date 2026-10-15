#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Emit the release build matrix, the platform archive names the draft must therefore carry,
and the release-note line describing what was built -- all from ONE list of legs.

Two things drove this out of the workflow file. First, a static matrix cannot skip a single
leg, and the Windows leg has to be skippable: Authenticode signing needs the organization's Azure
Artifact Signing set up for this repository, and a leg that fails on every tag makes the whole
release red, which hides real regressions instead of surfacing them. Second, the verify job hardcoded the per-platform
archive suffixes it demanded and the release notes hardcoded the platform list they promised,
so skipping a leg would have made a correct gate fail for the wrong reason and left the notes
claiming a binary nobody can download.

Windows is SKIPPED rather than built unsigned. An unsigned executable that reaches a user is
worse than none: it trains people to click through the warning.

Asset names follow what Redrob Launcher reads, <stem>-<version>-<os>-<arch>[-portable].<ext>:
the stem is the repository name, the os-arch token is one of the launcher's (linux-x86_64,
macos-arm64, windows-x64 -- NOT windows-x86_64), and the Windows archive carries the `-portable`
suffix because a `-setup.exe` is something the launcher cannot install and a plain `.zip` is
something it does not recognise. tools/tests/test_release_asset_names.py holds the pattern.

The release is exactly FOUR targets: the three native legs below and the web build (WEB), a
trunk-built wasm bundle zipped as `redrob-canvas-web-<version>.zip`. The web zip is not a launcher
asset (nothing installs it; a host unzips it behind a web server), so it carries no os-arch token
and is built by its own job in release.yml rather than by the native matrix. Nothing else is
built: no other Linux package format, no 32-bit Windows, no Intel or universal macOS build.
"""
import json
import os
import sys

STEM = "redrob-canvas"

# The web build: `trunk build --release` in apps/redrob-canvas-web, `dist/` zipped. One asset,
# no installer, no signing. `archive_name(version)` is the whole file name.
WEB = {
    "label": "Web (wasm)",
    "runner": "ubuntu-24.04",
    "target": "wasm32-unknown-unknown",
    "slug": "web",
    "notes": "Web (WebAssembly, for self-hosting)",
}


def web_asset_name(version):
    """The web bundle's file name on the release: `redrob-canvas-web-<version>.zip`."""
    return f"{STEM}-web-{version}.zip"


LEGS = [
    {
        "label": "Linux x86_64",
        "runner": "ubuntu-24.04",
        "target": "x86_64-unknown-linux-gnu",
        "slug": "linux-x86_64",
        "archive_suffix": "linux-x86_64.tar.gz",
        # The .deb (tools/release/make_deb.py), built with the archive by the Linux leg.
        "installer_suffix": "linux-x86_64.deb",
        "notes": "Linux x86_64",
    },
    {
        # arm64, not universal: one target, built on the runner's own Apple silicon, so the name
        # promises exactly what the binary runs on.
        "label": "macOS arm64",
        "runner": "macos-15",
        "target": "aarch64-apple-darwin",
        "slug": "macos-arm64",
        "archive_suffix": "macos-arm64.zip",
        # The disk image (installer/macos/), built, notarized and stapled by the macOS leg.
        "installer_suffix": "macos-arm64.dmg",
        "notes": "macOS arm64 for Apple silicon (signed, notarized, stapled)",
    },
    {
        "label": "Windows x64",
        "runner": "windows-2025",
        "target": "x86_64-pc-windows-msvc",
        # x64, the launcher's token, not x86_64: the launcher installs the first asset whose
        # os-arch token it knows, and `windows-x86_64` is not one it knows.
        "slug": "windows-x64",
        # `-portable`: the unpacked app, which the launcher unzips and runs in place. The signed
        # zip IS that app, so it is published under this name rather than as a second copy.
        "archive_suffix": "windows-x64-portable.zip",
        # The branded setup (installer/windows/), built and signed by the Windows leg. For people;
        # the launcher cannot install an exe and takes the portable zip instead.
        "installer_suffix": "windows-x64-setup.exe",
        "notes": "Windows x64 (signed and timestamped)",
        "requires_windows_signing": True,
    },
]

MATRIX_KEYS = ("label", "runner", "target", "slug")


def asset_names(version, legs=LEGS, web=True):
    """Every platform asset a release of `version` publishes, in the names the draft must carry.

    The archive and the installer of each leg, then the web bundle (unless `web` is False); the
    checksum files, the corresponding source and the licence texts are not platform assets and
    are not listed.
    """
    names = []
    for leg in legs:
        names.append(f"{STEM}-{version}-{leg['archive_suffix']}")
        if leg.get("installer_suffix"):
            names.append(f"{STEM}-{version}-{leg['installer_suffix']}")
    if web:
        names.append(web_asset_name(version))
    return names


def main():
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
            'repository variable WINDOWS_SIGNING_READY is not "true". Set it once Azure '
            "Artifact Signing is configured for this repository; nothing else needs to change."
        )

    if not included:
        print("Every platform is excluded; there would be nothing to release.", file=sys.stderr)
        sys.exit(1)

    matrix = [{key: leg[key] for key in MATRIX_KEYS} for leg in included]
    archive_suffixes = [leg["archive_suffix"] for leg in included]
    # Installers are listed separately from the archives Redrob Launcher installs; release.yml builds
    # and demands both.
    installer_suffixes = [leg["installer_suffix"] for leg in included if leg.get("installer_suffix")]
    # The web bundle is always built (its job has no signing to be ready for); release.yml names
    # it by hand as redrob-canvas-web-<version>.zip, which the tests hold to `web_asset_name`.
    notes_line = ", ".join([leg["notes"] for leg in included] + [WEB["notes"]])

    if sys.argv[1:] == ["--notes"]:
        print(notes_line)
        sys.exit(0)

    print(f"Building: {', '.join([leg['label'] for leg in included] + [WEB['label']])}")
    print(f"Platform archives required on the release: {' '.join(archive_suffixes)}")
    print(f"Installers required on the release: {' '.join(installer_suffixes) or '(none)'}")
    print(f"Web bundle required on the release: {web_asset_name('<version>')}")

    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"matrix={json.dumps(matrix, separators=(',', ':'))}\n")
            handle.write(f"archive_suffixes={' '.join(archive_suffixes)}\n")
            handle.write(f"installer_suffixes={' '.join(installer_suffixes)}\n")
            handle.write(f"notes_platforms={notes_line}\n")
            handle.write(f"windows_signing_ready={str(windows_signing_ready).lower()}\n")


if __name__ == "__main__":
    main()
