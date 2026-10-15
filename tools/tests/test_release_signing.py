# SPDX-License-Identifier: MIT OR Apache-2.0
"""The Windows signing rules of the shared redrob-labs release shape, checked against release.yml.

The rules (redrob-labs/.github, actions/sign-windows): the Azure credential is the job's OpenID
Connect login done AFTER the build and right before signing; the job runs in the `release`
environment with `id-token: write`; there is no certificate file and no client secret anywhere;
the executable is signed BEFORE it is zipped and before the setup embeds that zip; and the release
is created by the shared publish job, not by hand.
"""
from __future__ import annotations

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
RELEASE_YML = ROOT / ".github" / "workflows" / "release.yml"
WORKFLOWS = sorted((ROOT / ".github" / "workflows").glob("*.yml"))


class WindowsSigning(unittest.TestCase):
    def setUp(self) -> None:
        self.text = RELEASE_YML.read_text(encoding="utf-8")
        self.build = self.text[self.text.find("  build:"): self.text.find("  verify-release-assets:")]

    def test_no_certificate_secret_or_pfx_remains(self) -> None:
        for path in WORKFLOWS + sorted((ROOT / "tools" / "release").iterdir()):
            if path.is_dir():
                continue
            text = path.read_text(encoding="utf-8")
            for token in ("WIN_CSC_LINK", "CSC_LINK", "CSC_KEY_PASSWORD", "Import-PfxCertificate", ".pfx", "Set-AuthenticodeSignature"):
                self.assertNotIn(token, text, f"{path.name} still carries the certificate-file signing path ({token})")

    def test_job_runs_in_the_release_environment_with_an_id_token(self) -> None:
        self.assertRegex(self.build, re.compile(r"^    environment: release$", re.MULTILINE))
        self.assertRegex(self.build, re.compile(r"^      id-token: write$", re.MULTILINE))

    def test_login_comes_after_the_build_and_before_the_first_signature(self) -> None:
        build_at = self.build.find("- name: Build\n")
        stage_at = self.build.find("- name: Stage\n")
        login_at = self.build.find("uses: azure/login@")
        sign_at = self.build.find("uses: redrob-labs/.github/actions/sign-windows@")
        self.assertGreater(build_at, -1)
        self.assertGreater(stage_at, build_at, "the binary is staged before it is built")
        self.assertGreater(login_at, stage_at, "azure/login runs before the Windows build is staged")
        self.assertGreater(sign_at, login_at, "signing runs before azure/login")

    def test_executable_is_signed_before_the_zip_and_the_setup(self) -> None:
        sign_exe_at = self.build.find("- name: Sign the executable (Windows)")
        zip_at = self.build.find("-portable.zip' -Force")
        setup_at = self.build.find("- name: Build the setup (Windows)")
        sign_setup_at = self.build.find("- name: Sign the setup (Windows)")
        self.assertGreater(sign_exe_at, -1)
        self.assertGreater(zip_at, sign_exe_at, "the portable zip is made before the executable is signed")
        self.assertGreater(setup_at, zip_at, "the setup is built before the zip it embeds")
        self.assertGreater(sign_setup_at, setup_at, "the setup is signed before it is built")
        self.assertEqual(self.build.count("uses: redrob-labs/.github/actions/sign-windows@"), 3, "executable, uninstaller and setup are each signed once")

    def test_no_hand_rolled_release_creation(self) -> None:
        self.assertNotIn("gh release create", self.text)
        self.assertNotIn("gh release upload", self.text)
        self.assertNotRegex(self.text, re.compile(r"publish:\s*false"))


if __name__ == "__main__":
    unittest.main()
