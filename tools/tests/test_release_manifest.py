# SPDX-License-Identifier: GPL-3.0-or-later
"""Every release carries `redrob-launcher.json`, and the release gates refuse one that does not.

tools/release_manifest.py writes the file and checks a release directory for it. The content is
checked here against the launcher's contract (schema 1, sharedAccount true, newline-terminated),
the check is checked by taking the file away and by corrupting it, and the wiring is checked:
the scripts write it before the sums, and both release workflows demand it on the draft.
"""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
TOOL_PATH = ROOT / "tools" / "release_manifest.py"
SPEC = importlib.util.spec_from_file_location("release_manifest", TOOL_PATH)
assert SPEC is not None and SPEC.loader is not None
release_manifest = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = release_manifest
SPEC.loader.exec_module(release_manifest)

RELEASE_YML = ROOT / ".github" / "workflows" / "release.yml"
RELEASE_CI_YML = ROOT / ".github" / "workflows" / "release-ci.yml"
BUILD_LINUX = ROOT / "tools" / "release" / "build-linux.sh"


class ManifestContent(unittest.TestCase):
    def test_written_file_is_the_launcher_contract(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = release_manifest.write(Path(tmp))
            self.assertEqual(path.name, "redrob-launcher.json")
            text = path.read_text(encoding="utf-8")
            self.assertTrue(text.endswith("\n"), repr(text))
            self.assertEqual(text, '{ "schema": 1, "sharedAccount": true }\n')
            data = json.loads(text)
            self.assertEqual(data, {"schema": 1, "sharedAccount": True})
            self.assertIs(data["sharedAccount"], True)

    def test_check_passes_on_a_written_directory_and_its_sums(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            release_manifest.write(out)
            (out / "redrob-canvas-0.5.7-linux-x86_64.tar.gz").write_bytes(b"x")
            sums = subprocess.run(
                ["sha256sum", "redrob-launcher.json", "redrob-canvas-0.5.7-linux-x86_64.tar.gz"],
                cwd=out, check=True, capture_output=True, text=True,
            ).stdout
            (out / "redrob-canvas-0.5.7-linux-x86_64-SHA256SUMS.txt").write_text(sums, encoding="utf-8")
            self.assertEqual(release_manifest.problems(out), [])
            result = subprocess.run([sys.executable, str(TOOL_PATH), "--check", str(out)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_check_fails_when_the_file_is_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            (out / "redrob-canvas-0.5.7-linux-x86_64.tar.gz").write_bytes(b"x")
            found = release_manifest.problems(out)
            self.assertEqual(len(found), 1)
            self.assertIn("missing", found[0])
            result = subprocess.run([sys.executable, str(TOOL_PATH), "--check", str(out)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            self.assertIn("::error::", result.stdout)

    def test_check_fails_on_the_wrong_content(self) -> None:
        cases = {
            "not json": "{ schema: 1 }\n",
            "schema 2": '{ "schema": 2, "sharedAccount": true }\n',
            "sharedAccount false": '{ "schema": 1, "sharedAccount": false }\n',
            "sharedAccount string": '{ "schema": 1, "sharedAccount": "true" }\n',
            "array": '[1]\n',
            "no newline": '{ "schema": 1, "sharedAccount": true }',
        }
        for label, text in cases.items():
            with self.subTest(label), tempfile.TemporaryDirectory() as tmp:
                out = Path(tmp)
                (out / "redrob-launcher.json").write_text(text, encoding="utf-8")
                self.assertTrue(release_manifest.problems(out), label)

    def test_check_fails_when_the_sums_leave_it_out(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            release_manifest.write(out)
            (out / "redrob-canvas-0.5.7-linux-x86_64-SHA256SUMS.txt").write_text(
                "0" * 64 + "  redrob-canvas-0.5.7-linux-x86_64.tar.gz\n", encoding="utf-8"
            )
            found = release_manifest.problems(out)
            self.assertEqual(len(found), 1)
            self.assertIn("SHA256SUMS", found[0])


class ManifestWiring(unittest.TestCase):
    def test_build_linux_writes_it_before_the_sums_and_lists_it(self) -> None:
        text = BUILD_LINUX.read_text(encoding="utf-8")
        write_at = text.find("release_manifest.py --write")
        sums_at = text.find("sha256sum")
        self.assertGreater(write_at, -1, "build-linux.sh does not write redrob-launcher.json")
        self.assertGreater(sums_at, write_at, "build-linux.sh computes the sums before writing the manifest")
        self.assertRegex(text, r"sha256sum[^\n]*(\\\n[^\n]*)*redrob-launcher\.json", "the Linux SHA256SUMS does not list redrob-launcher.json")

    def test_release_ci_writes_it_before_the_sums(self) -> None:
        text = RELEASE_CI_YML.read_text(encoding="utf-8")
        write_at = text.find("release_manifest.py --write")
        sums_at = text.find("shasum -a 256 ./*")
        self.assertGreater(write_at, -1, "release-ci.yml does not write redrob-launcher.json")
        self.assertGreater(sums_at, write_at, "release-ci.yml computes the sums before writing the manifest")

    def test_both_workflows_demand_it_on_the_draft(self) -> None:
        for path in (RELEASE_YML, RELEASE_CI_YML):
            text = path.read_text(encoding="utf-8")
            self.assertIn("'^redrob-launcher\\.json$'", text, f"{path.name}: verify-release-assets does not demand redrob-launcher.json")

    def test_release_yml_validate_requires_it_on_the_draft(self) -> None:
        # local-release.sh creates the draft from the Linux build directory; validate is the first job
        # that sees the draft, so a draft without the manifest stops there, before any signing.
        text = RELEASE_YML.read_text(encoding="utf-8")
        validate = text[text.find("  validate:"): text.find("  sign-macos:")]
        self.assertRegex(validate, re.compile(r'"redrob-launcher\.json"'), "release.yml validate does not require redrob-launcher.json on the draft")


if __name__ == "__main__":
    unittest.main()
