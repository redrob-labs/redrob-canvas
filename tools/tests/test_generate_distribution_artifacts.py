# SPDX-License-Identifier: MIT OR Apache-2.0
"""The third-party notice generator, checked without cargo.

The generator's one network-free input that cannot be faked cheaply is `cargo metadata`, so these
tests drive `generate_notice` directly with a small synthetic package set and check the properties
the committed THIRD_PARTY_NOTICES.md relies on: every locked package is inventoried, every licence
file inside a package is reproduced byte for byte and keyed by its hash, identical texts appear once,
a package without a licence expression is refused, and the output is byte-stable. The end-to-end
`--check` against the real Cargo.lock runs in CI (`Distribution artifacts are current`).
"""
from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
TOOL_PATH = ROOT / "tools" / "generate_distribution_artifacts.py"
SPEC = importlib.util.spec_from_file_location("generate_distribution_artifacts", TOOL_PATH)
assert SPEC is not None and SPEC.loader is not None
gen = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gen
SPEC.loader.exec_module(gen)

MIT = b"MIT License\n\nCopyright (c) 2026 Example\n"
APACHE = b"                                 Apache License\n                           Version 2.0, January 2004\n"


def package(root: Path, name: str, version: str, files: dict[str, bytes], license_expr: str | None,
            source: str | None = "registry+https://github.com/rust-lang/crates.io-index",
            checksum: str | None = "deadbeef") -> tuple[dict, dict]:
    directory = root / f"{name}-{version}"
    directory.mkdir()
    (directory / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{version}"\n')
    for relative, data in files.items():
        (directory / relative).write_bytes(data)
    metadata = {
        "name": name,
        "version": version,
        "source": source,
        "license": license_expr,
        "license_file": None,
        "manifest_path": str(directory / "Cargo.toml"),
    }
    locked = {"name": name, "version": version}
    if source:
        locked["source"] = source
    if checksum:
        locked["checksum"] = checksum
    return metadata, locked


class NoticeGeneration(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def build(self, *packages: tuple[dict, dict]) -> bytes:
        by_key = {gen.package_key(meta): meta for meta, _ in packages}
        locked = sorted((lock for _, lock in packages), key=lambda p: (p["name"], p["version"]))
        return gen.generate_notice(by_key, locked)

    def test_every_package_and_every_licence_file_is_reproduced_once(self) -> None:
        a = package(self.root, "alpha", "1.0.0", {"LICENSE-MIT": MIT, "LICENSE-APACHE": APACHE}, "MIT OR Apache-2.0")
        b = package(self.root, "beta", "2.0.0", {"LICENSE": MIT}, "MIT")
        notice = self.build(a, b).decode("utf-8")
        self.assertIn("| `alpha` | `1.0.0` |", notice)
        self.assertIn("| `beta` | `2.0.0` |", notice)
        self.assertIn("Resolved packages: **2**", notice)
        # The MIT text is shared by both packages and appears exactly once, keyed by its hash.
        digest = hashlib.sha256(MIT).hexdigest()
        self.assertEqual(notice.count(f"### `{digest}`"), 1)
        self.assertIn("- alpha 1.0.0 - LICENSE-MIT", notice)
        self.assertIn("- beta 2.0.0 - LICENSE", notice)
        self.assertIn("Unique reproduced license/notice texts: **2**", notice)
        self.assertIn(MIT.decode(), notice)
        self.assertIn(APACHE.decode(), notice)

    def test_a_package_without_a_licence_expression_is_refused(self) -> None:
        bad = package(self.root, "gamma", "0.1.0", {"LICENSE": MIT}, None)
        with self.assertRaises(RuntimeError):
            self.build(bad)

    def test_workspace_and_git_packages_record_their_identity(self) -> None:
        ws = package(self.root, "redrob-canvas", "0.7.0", {"LICENSE-MIT": MIT}, "MIT OR Apache-2.0", source=None, checksum=None)
        git = package(self.root, "engine", "0.1.0", {"LICENSE": MIT}, "MIT",
                      source="git+https://github.com/redrob-labs/redrob-code?tag=v0.6.4#bcb8af0d", checksum=None)
        notice = self.build(ws, git).decode("utf-8")
        self.assertIn("| workspace source | `workspace` |", notice)
        self.assertIn("| `git:bcb8af0d` |", notice)

    def test_output_is_byte_stable(self) -> None:
        a = package(self.root, "alpha", "1.0.0", {"LICENSE": MIT}, "MIT")
        self.assertEqual(self.build(a), self.build(a))

    def test_header_claims_no_network_and_names_the_permissive_licence(self) -> None:
        a = package(self.root, "alpha", "1.0.0", {"LICENSE": MIT}, "MIT")
        notice = self.build(a).decode("utf-8")
        self.assertTrue(notice.startswith("<!-- Generated by tools/generate_distribution_artifacts.py; do not edit. -->"))
        self.assertIn("--locked --offline", notice)
        self.assertIn("MIT OR Apache-2.0", notice)
        self.assertNotIn("GPL", notice)
        self.assertNotIn("SOURCE_OFFER", notice)


class StaleCheckWiring(unittest.TestCase):
    def test_ci_populates_the_cache_then_checks_offline(self) -> None:
        for workflow in ("ci.yml", "release.yml"):
            text = (ROOT / ".github" / "workflows" / workflow).read_text(encoding="utf-8")
            fetch_at = text.find("cargo fetch --locked")
            check_at = text.find("generate_distribution_artifacts.py --check")
            self.assertGreater(fetch_at, -1, f"{workflow} never populates the registry cache")
            self.assertGreater(check_at, fetch_at, f"{workflow} checks the notices before fetching")
            self.assertNotIn("SOURCE_OFFER", text)
            self.assertNotIn("--babl", text)
            self.assertNotIn("--lcms", text)


if __name__ == "__main__":
    unittest.main()
