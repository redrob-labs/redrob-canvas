# SPDX-License-Identifier: MIT OR Apache-2.0
"""Release asset names are what Redrob Launcher reads, and the release is exactly four targets.

The launcher installs an asset only when its name is <stem>-<version>-<os>-<arch>[-portable].<ext>
with an os-arch token and a package kind it knows. tools/release_matrix.py is the single list the
names come from, and the workflow and the build scripts spell a few of them out by hand, so both
are checked: the generator against the launcher's grammar, and every hand-written name against the
generator. The fourth target, the web bundle, is not a launcher asset and has its own name.

The release ships macOS arm64 (DMG), Linux x86_64 (tar.gz + deb), Windows x64 (setup.exe) and the
web bundle (zip) -- nothing else. The words that used to name the rest (Flatpak, AppImage, rpm,
FreeBSD, snap, Intel / universal macOS, 32-bit Windows) must not come back into the release files.
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

# Platform::tokens in the launcher (crates/core/src/platform.rs) that this release uses. Not
# `windows-x86_64`, not `amd64`, not `arm64` on Linux, not `darwin`.
LAUNCHER_TOKENS = (
    "linux-x86_64",
    "macos-arm64",
    "windows-x64",
)
# PackageKind suffixes the launcher installs, by OS, among the kinds this release builds. A
# `-setup.exe` and a plain `.zip` are not here.
INSTALLABLE = {
    "windows": ("-portable.zip",),
    "macos": (".dmg",),
    "linux": (".deb", ".tar.gz"),
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

VERSION = "0.7.0"

# Files that describe what the release builds. Anything naming a dropped target here is a promise
# the workflow no longer keeps.
RELEASE_FILES = [
    ROOT / ".github" / "workflows" / "release.yml",
    ROOT / "tools" / "release_matrix.py",
    ROOT / "tools" / "release" / "README.md",
    ROOT / "tools" / "release" / "build-linux.sh",
    ROOT / "tools" / "release" / "stage.py",
    ROOT / "tools" / "release" / "make_deb.py",
]
# Case-insensitive, whole words where the word is also an English one.
DROPPED_TARGETS = (
    r"flatpak",
    r"appimage",
    r"freebsd",
    r"\brpm\b",
    r"\bsnap\b",
    r"x86_64-apple-darwin",
    r"macos-x86_64",
    r"macos-universal",
    r"i686-pc-windows",
    r"windows-x86\b",
)


class AssetNames(unittest.TestCase):
    def test_exactly_four_targets(self) -> None:
        # Three native legs and the web bundle, each named once.
        self.assertEqual([leg["slug"] for leg in release_matrix.LEGS], ["linux-x86_64", "macos-arm64", "windows-x64"])
        self.assertEqual(release_matrix.WEB["slug"], "web")
        self.assertEqual(release_matrix.WEB["target"], "wasm32-unknown-unknown")
        self.assertEqual(
            release_matrix.asset_names(VERSION),
            [
                "redrob-canvas-0.7.0-linux-x86_64.tar.gz",
                "redrob-canvas-0.7.0-linux-x86_64.deb",
                "redrob-canvas-0.7.0-macos-arm64.zip",
                "redrob-canvas-0.7.0-macos-arm64.dmg",
                "redrob-canvas-0.7.0-windows-x64-portable.zip",
                "redrob-canvas-0.7.0-windows-x64-setup.exe",
                "redrob-canvas-web-0.7.0.zip",
            ],
        )

    def test_every_leg_publishes_something_the_launcher_installs(self) -> None:
        for leg in release_matrix.LEGS:
            os_name = leg["slug"].split("-", 1)[0]
            names = release_matrix.asset_names(VERSION, [leg], web=False)
            installable = [n for n in names if INSTALLABLE_RE.match(n)]
            self.assertTrue(installable, f"{leg['label']} publishes {names}; none is a name the launcher installs")
            for name in installable:
                m = INSTALLABLE_RE.match(name)
                assert m is not None
                self.assertEqual(m["version"], VERSION, name)
                self.assertTrue(m["token"].startswith(os_name + "-"), f"{name}: token does not say {os_name}")
                self.assertIn(m["ext"], INSTALLABLE[os_name], f"{name}: {m['ext']} is not installable on {os_name}")

    def test_no_platform_asset_uses_a_token_the_launcher_does_not_know(self) -> None:
        for name in release_matrix.asset_names(VERSION, web=False):
            rest = name.removeprefix(f"redrob-canvas-{VERSION}-")
            self.assertNotEqual(rest, name, name)
            ext = next((e for e in ALL_EXTENSIONS if rest.endswith(e)), None)
            self.assertIsNotNone(ext, f"{name}: unknown extension")
            token = rest[: -len(ext)]
            self.assertIn(token, LAUNCHER_TOKENS, f"{name}: {token!r} is not a launcher os-arch token")

    def test_web_bundle_is_named_apart_from_the_launcher_assets(self) -> None:
        # `redrob-canvas-web-<version>.zip`: the launcher must NOT take it for a platform asset
        # (no os-arch token can be read out of it), and the workflow writes the same name by hand.
        name = release_matrix.web_asset_name(VERSION)
        self.assertEqual(name, "redrob-canvas-web-0.7.0.zip")
        self.assertIsNone(INSTALLABLE_RE.match(name), f"{name} reads as a launcher-installable asset")
        text = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        for needle, why in (
            ("redrob-canvas-web-${VERSION}.zip", "release.yml does not zip the web build under the generator's name"),
            ("redrob-canvas-web-${{ needs.validate.outputs.version }}.zip", "verify-release-assets does not demand the web bundle"),
            ("name: release-web", "the web job does not upload a release-* artifact for the shared publish job"),
            ("trunk build --release", "the web job does not build with trunk"),
            ('cargo install trunk --locked --version "$TRUNK_VERSION"', "trunk is not installed at a pinned version"),
            ("wasm32-unknown-unknown", "the web job does not add the wasm target"),
        ):
            self.assertTrue(needle in text, why)
        self.assertIsNotNone(re.search(r'TRUNK_VERSION: "\d+\.\d+\.\d+"', text), "trunk is not pinned")

    def test_windows_zip_is_the_portable_one(self) -> None:
        # The launcher cannot install a setup.exe, so the Windows leg must offer a -portable.zip
        # (or an .msi, which this toolchain does not build).
        windows = [leg for leg in release_matrix.LEGS if leg["slug"].startswith("windows-")]
        self.assertEqual(len(windows), 1)
        for leg in windows:
            self.assertEqual(leg["slug"], "windows-x64")
            self.assertEqual(leg["target"], "x86_64-pc-windows-msvc")
            self.assertTrue(leg["archive_suffix"].endswith("-portable.zip"), leg["archive_suffix"])
            self.assertTrue(leg["archive_suffix"].startswith(leg["slug"]), leg["archive_suffix"])
            self.assertEqual(leg["installer_suffix"], "windows-x64-setup.exe")

    def test_macos_is_arm64_only_and_linux_is_tar_plus_deb(self) -> None:
        macos = [leg for leg in release_matrix.LEGS if leg["slug"].startswith("macos-")]
        self.assertEqual(len(macos), 1)
        self.assertEqual(macos[0]["target"], "aarch64-apple-darwin")
        self.assertEqual(macos[0]["installer_suffix"], "macos-arm64.dmg")
        linux = [leg for leg in release_matrix.LEGS if leg["slug"].startswith("linux-")]
        self.assertEqual(len(linux), 1)
        self.assertEqual(linux[0]["target"], "x86_64-unknown-linux-gnu")
        self.assertEqual(linux[0]["archive_suffix"], "linux-x86_64.tar.gz")
        self.assertEqual(linux[0]["installer_suffix"], "linux-x86_64.deb")

    def test_dropped_targets_are_gone_from_the_release_files(self) -> None:
        for path in RELEASE_FILES:
            text = path.read_text(encoding="utf-8")
            for pattern in DROPPED_TARGETS:
                self.assertIsNone(
                    re.search(pattern, text, re.IGNORECASE),
                    f"{path.relative_to(ROOT)} still mentions a dropped target ({pattern})",
                )

    def test_hand_written_names_agree_with_the_generator(self) -> None:
        # Where the workflows and release scripts spell a slug out by hand, it must be one of the
        # generator's: these are the strings a rename is most likely to miss.
        slugs = {leg["slug"] for leg in release_matrix.LEGS}
        files = [
            ROOT / ".github" / "workflows" / "release.yml",
            ROOT / "tools" / "release" / "build-linux.sh",
        ]
        pattern = re.compile(r"redrob-canvas-\$\{?(?:version|VERSION)\}?-((?:linux|macos|windows)-[a-z0-9_]+)")
        suffix_pattern = re.compile(r"^\s*SUFFIX:\s*(\S+)\s*$", re.MULTILINE)
        found = set()
        for path in files:
            text = path.read_text(encoding="utf-8")
            for m in pattern.finditer(text):
                found.add(m.group(1))
                self.assertIn(m.group(1), slugs, f"{path.name}: {m.group(0)!r} names a slug the matrix does not have")
            for m in suffix_pattern.finditer(text):
                self.assertIn(m.group(1), slugs, f"{path.name}: SUFFIX {m.group(1)!r} is not a matrix slug")
        # release.yml names platforms through the matrix slug, so the only hand-written slug left is the
        # rehearsal script's Linux one; the Windows name is the generator's to keep.
        self.assertIn("linux-x86_64", found, "build-linux.sh no longer names the Linux archive")
        self.assertIn("windows-x64", slugs, "the matrix no longer has the Windows leg")
        for path in files + [ROOT / "README.md", ROOT / "README.ko.md", ROOT / "installer" / "windows" / "README.md"]:
            self.assertFalse("windows-x86_64" in path.read_text(encoding="utf-8"), f"{path.name} still says windows-x86_64")

    def test_windows_zip_in_the_workflow_is_published_portable(self) -> None:
        # The signed/packaged zip is written by hand in the workflow; it has to land under the
        # archive suffix the generator demands, or verify-release-assets fails every release.
        for path in (ROOT / ".github" / "workflows" / "release.yml",):
            text = path.read_text(encoding="utf-8")
            self.assertTrue("-portable.zip' -Force" in text, f"{path.name}: the Windows Compress-Archive does not write a -portable.zip")
            self.assertNotRegex(text, r"Compress-Archive .*\$\{?(?:BASE|base)\}?\.zip'", f"{path.name}: a plain .zip is still packaged on Windows")


if __name__ == "__main__":
    unittest.main()
