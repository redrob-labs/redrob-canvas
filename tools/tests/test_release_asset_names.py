# SPDX-License-Identifier: GPL-3.0-or-later
"""Release asset names are what Redrob Launcher reads.

The launcher installs an asset only when its name is <stem>-<version>-<os>-<arch>[-portable].<ext>
with an os-arch token and a package kind it knows. tools/release_matrix.py is the single list the
names come from, and the workflows and release scripts spell a few of them out by hand (the
unsigned archive the signing job downloads, the three names the draft must carry), so both are
checked: the generator against the launcher's grammar, and every hand-written name against the
generator.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
import re
import sys
import unittest


ROOT = Path(__file__).resolve().parents[2]
TOOL_PATH = ROOT / "tools" / "release_matrix.py"
SPEC = importlib.util.spec_from_file_location("release_matrix", TOOL_PATH)
assert SPEC is not None and SPEC.loader is not None
release_matrix = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = release_matrix
SPEC.loader.exec_module(release_matrix)

# Platform::tokens in the launcher (crates/core/src/platform.rs). Not `windows-x86_64`, not
# `amd64`, not `arm64` on Linux, not `darwin`.
LAUNCHER_TOKENS = (
    "linux-x86_64",
    "linux-aarch64",
    "linux-riscv64",
    "macos-universal",
    "macos-aarch64",
    "macos-arm64",
    "macos-x86_64",
    "windows-x64",
    "windows-arm64",
    "windows-x86",
    "freebsd-x86_64",
)
# PackageKind suffixes the launcher installs, by OS. A `-setup.exe` and a plain `.zip` are not here.
INSTALLABLE = {
    "windows": (".msi", "-portable.zip"),
    "macos": (".dmg",),
    "linux": (".AppImage", ".deb", ".rpm", ".flatpak", ".tar.gz"),
    "freebsd": (".tar.gz",),
}
# Extras a release may carry beside the launcher-installable ones: people install from them.
EXTRA = ("-setup.exe", ".zip")
# Longest first, so `-portable.zip` is read before `.zip` and `.tar.gz` before anything shorter.
ALL_EXTENSIONS = tuple(sorted({e for v in INSTALLABLE.values() for e in v} | set(EXTRA), key=len, reverse=True))

INSTALLABLE_RE = re.compile(
    r"^(?P<stem>redrob-canvas)-(?P<version>\d[^-]*(?:-rc\.\d+)?)-(?P<token>"
    + "|".join(re.escape(t) for t in LAUNCHER_TOKENS)
    + r")(?P<ext>"
    + "|".join(re.escape(e) for e in ALL_EXTENSIONS if e not in EXTRA)
    + r")$"
)

VERSION = "0.5.7"


class AssetNames(unittest.TestCase):
    def test_every_leg_publishes_something_the_launcher_installs(self) -> None:
        for leg in release_matrix.LEGS:
            os_name = leg["slug"].split("-", 1)[0]
            names = release_matrix.asset_names(VERSION, [leg])
            installable = [n for n in names if INSTALLABLE_RE.match(n)]
            self.assertTrue(installable, f"{leg['label']} publishes {names}; none is a name the launcher installs")
            for name in installable:
                m = INSTALLABLE_RE.match(name)
                assert m is not None
                self.assertEqual(m["version"], VERSION, name)
                self.assertTrue(m["token"].startswith(os_name + "-"), f"{name}: token does not say {os_name}")
                self.assertIn(m["ext"], INSTALLABLE[os_name], f"{name}: {m['ext']} is not installable on {os_name}")

    def test_no_platform_asset_uses_a_token_the_launcher_does_not_know(self) -> None:
        for name in release_matrix.asset_names(VERSION):
            rest = name.removeprefix(f"redrob-canvas-{VERSION}-")
            self.assertNotEqual(rest, name, name)
            ext = next((e for e in ALL_EXTENSIONS if rest.endswith(e)), None)
            self.assertIsNotNone(ext, f"{name}: unknown extension")
            token = rest[: -len(ext)]
            self.assertIn(token, LAUNCHER_TOKENS, f"{name}: {token!r} is not a launcher os-arch token")

    def test_windows_zip_is_the_portable_one(self) -> None:
        # The launcher cannot install a setup.exe, so the Windows leg must offer a -portable.zip
        # (or an .msi, which this toolchain does not build).
        windows = [leg for leg in release_matrix.LEGS if leg["slug"].startswith("windows-")]
        self.assertTrue(windows)
        for leg in windows:
            self.assertEqual(leg["slug"], "windows-x64")
            self.assertTrue(leg["archive_suffix"].endswith("-portable.zip"), leg["archive_suffix"])
            self.assertTrue(leg["archive_suffix"].startswith(leg["slug"]), leg["archive_suffix"])

    def test_hand_written_names_agree_with_the_generator(self) -> None:
        # Where the workflows and release scripts spell a slug out by hand, it must be one of the
        # generator's: these are the strings a rename is most likely to miss.
        slugs = {leg["slug"] for leg in release_matrix.LEGS}
        files = [
            ROOT / ".github" / "workflows" / "release.yml",
            ROOT / ".github" / "workflows" / "release-ci.yml",
            ROOT / "tools" / "release" / "local-release.sh",
            ROOT / "tools" / "release" / "build-windows.ps1",
            ROOT / "tools" / "release" / "build-macos.sh",
            ROOT / "tools" / "release" / "build-linux.sh",
        ]
        pattern = re.compile(r"redrob-canvas-\$\{?(?:version|VERSION)\}?-((?:linux|macos|windows|freebsd)-[a-z0-9_]+)")
        suffix_pattern = re.compile(r"^\s*SUFFIX:\s*(\S+)\s*$", re.MULTILINE)
        found = set()
        for path in files:
            text = path.read_text(encoding="utf-8")
            for m in pattern.finditer(text):
                found.add(m.group(1))
                self.assertIn(m.group(1), slugs, f"{path.name}: {m.group(0)!r} names a slug the matrix does not have")
            for m in suffix_pattern.finditer(text):
                self.assertIn(m.group(1), slugs, f"{path.name}: SUFFIX {m.group(1)!r} is not a matrix slug")
        self.assertIn("windows-x64", found, "release.yml/local-release.sh no longer name the Windows unsigned archive")
        for path in files + [ROOT / "README.md", ROOT / "README.ko.md", ROOT / "installer" / "windows" / "README.md"]:
            self.assertFalse("windows-x86_64" in path.read_text(encoding="utf-8"), f"{path.name} still says windows-x86_64")

    def test_windows_zip_in_the_workflows_is_published_portable(self) -> None:
        # The signed/packaged zip is written by hand in both workflows; it has to land under the
        # archive suffix the generator demands, or verify-release-assets fails every release.
        for path in (ROOT / ".github" / "workflows" / "release.yml", ROOT / ".github" / "workflows" / "release-ci.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertTrue("-portable.zip' -Force" in text, f"{path.name}: the Windows Compress-Archive does not write a -portable.zip")
            self.assertNotRegex(text, r"Compress-Archive .*\$\{?(?:BASE|base)\}?\.zip'", f"{path.name}: a plain .zip is still packaged on Windows")


if __name__ == "__main__":
    unittest.main()
