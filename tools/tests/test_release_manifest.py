# SPDX-License-Identifier: GPL-3.0-or-later
"""Every release carries `redrob-launcher.json`, and the release gates refuse one that does not.

tools/release_manifest.py writes the file and checks a release directory for it. The content is
checked here against the launcher's contract (schema 1, sharedAccount true, newline-terminated),
the check is checked by taking the file away and by corrupting it, and the wiring is checked:
the Linux build script writes it before the sums, and the release workflow has the shared publish
job write it (release-publish.yml's `launcher_manifest: true`) over the published assets.
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

    def test_release_yml_has_the_shared_publish_job_write_it(self) -> None:
        # The shared redrob-labs publish job writes redrob-launcher.json and SHA256SUMS.txt over every
        # asset at once; this workflow must call it with the manifest switched on, and must not write
        # the file itself as well (two writers of one asset is how a stale copy ships).
        text = RELEASE_YML.read_text(encoding="utf-8")
        publish = text[text.find("  publish:"):]
        self.assertIn("redrob-labs/.github/.github/workflows/release-publish.yml@", publish, "release.yml does not call the shared publish job")
        self.assertRegex(publish, re.compile(r"launcher_manifest:\s*true"), "release.yml publishes without redrob-launcher.json")
        self.assertNotIn("release_manifest.py --write", text, "release.yml writes redrob-launcher.json itself as well as through the publish job")


if __name__ == "__main__":
    unittest.main()
