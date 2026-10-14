# SPDX-License-Identifier: GPL-3.0-or-later
from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


TOOL_PATH = Path(__file__).resolve().parents[1] / "generate_distribution_artifacts.py"
SPEC = importlib.util.spec_from_file_location("generate_distribution_artifacts", TOOL_PATH)
assert SPEC is not None and SPEC.loader is not None
generator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = generator
SPEC.loader.exec_module(generator)


class RegistryFixture:
    name = "demo"
    version = "1.2.3"
    bucket = "example-registry"
    source = "registry+https://example.invalid/index"

    def __init__(self, root: Path) -> None:
        self.root = root
        self.package_root = root / "registry" / "src" / self.bucket / f"{self.name}-{self.version}"
        self.archive_path = root / "registry" / "cache" / self.bucket / f"{self.name}-{self.version}.crate"
        self.archive_files = {
            "Cargo.toml": b"[package]\nname = \"demo\"\nversion = \"1.2.3\"\n",
            "LICENSE": b"authoritative license\n",
            "src/lib.rs": b"pub fn answer() -> u8 { 42 }\n",
        }
        self.archive_directories = {"empty"}
        self.package_root.mkdir(parents=True)
        for relative, data in self.archive_files.items():
            path = self.package_root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        for relative in self.archive_directories:
            (self.package_root / relative).mkdir(parents=True, exist_ok=True)
        (self.package_root / ".cargo-ok").write_bytes(b"")
        self.write_archive(self.regular_members())

    @property
    def archive_root(self) -> str:
        return f"{self.name}-{self.version}"

    def regular_members(self) -> list[tuple[str, str, bytes]]:
        members = [
            (f"{self.archive_root}/{relative}", "file", data)
            for relative, data in self.archive_files.items()
        ]
        members.extend(
            (f"{self.archive_root}/{relative}", "directory", b"")
            for relative in self.archive_directories
        )
        return members

    def write_archive(self, members: list[tuple[str, str, bytes]]) -> None:
        self.archive_path.parent.mkdir(parents=True, exist_ok=True)
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w:gz", format=tarfile.PAX_FORMAT) as archive:
            for name, kind, data in members:
                info = tarfile.TarInfo(name)
                if kind == "file":
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
                elif kind == "directory":
                    info.type = tarfile.DIRTYPE
                    archive.addfile(info)
                elif kind == "symlink":
                    info.type = tarfile.SYMTYPE
                    info.linkname = "LICENSE"
                    archive.addfile(info)
                elif kind == "hardlink":
                    info.type = tarfile.LNKTYPE
                    info.linkname = f"{self.archive_root}/LICENSE"
                    archive.addfile(info)
                elif kind == "special":
                    info.type = tarfile.CHRTYPE
                    archive.addfile(info)
                else:
                    raise AssertionError(f"unknown member kind: {kind}")
        self.archive_path.write_bytes(output.getvalue())

    def write_pax_path_archive(self, path: str) -> None:
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w:gz", format=tarfile.PAX_FORMAT) as archive:
            info = tarfile.TarInfo(f"{self.archive_root}/placeholder")
            info.pax_headers = {"path": path}
            info.size = 4
            archive.addfile(info, io.BytesIO(b"data"))
        self.archive_path.write_bytes(output.getvalue())

    def locked(self) -> dict[str, str]:
        return {
            "name": self.name,
            "version": self.version,
            "source": self.source,
            "checksum": hashlib.sha256(self.archive_path.read_bytes()).hexdigest(),
        }

    def package(self) -> dict[str, object]:
        return {
            "name": self.name,
            "version": self.version,
            "source": self.source,
            "manifest_path": str(self.package_root / "Cargo.toml"),
            "license": "MIT",
            "license_file": "LICENSE",
        }

    def verify(self) -> generator.SourceSnapshot:
        locked = self.locked()
        key = generator.package_key(locked)
        return generator.verified_registry_sources({key: self.package()}, [locked])[key]


class RegistrySourceIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.fixture = RegistryFixture(Path(self.temporary.name))

    def assert_verify_error(self, pattern: str) -> None:
        with self.assertRaisesRegex(RuntimeError, pattern):
            self.fixture.verify()

    def test_valid_archive_ignores_only_regular_cargo_ok(self) -> None:
        snapshot = self.fixture.verify()
        self.assertEqual(snapshot.files, self.fixture.archive_files)
        self.assertEqual(snapshot.directories, frozenset({"src", "empty"}))
        self.assertTrue(snapshot.archive_authoritative)

    def test_modified_license_is_rejected_even_with_valid_archive(self) -> None:
        (self.fixture.package_root / "LICENSE").write_bytes(b"modified license\n")
        self.assert_verify_error("content mismatch.*LICENSE")

    def test_missing_file_is_rejected(self) -> None:
        (self.fixture.package_root / "src/lib.rs").unlink()
        self.assert_verify_error("missing archive paths.*src/lib.rs")

    def test_extra_file_is_rejected(self) -> None:
        (self.fixture.package_root / "extra.txt").write_bytes(b"extra")
        self.assert_verify_error("extra paths.*extra.txt")

    def test_file_directory_type_mismatch_is_rejected(self) -> None:
        license_path = self.fixture.package_root / "LICENSE"
        license_path.unlink()
        license_path.mkdir()
        self.assert_verify_error("type mismatch.*LICENSE")

    def test_content_mismatch_is_rejected(self) -> None:
        (self.fixture.package_root / "src/lib.rs").write_bytes(b"tampered\n")
        self.assert_verify_error("content mismatch.*src/lib.rs")

    def test_missing_empty_directory_is_rejected(self) -> None:
        (self.fixture.package_root / "empty").rmdir()
        self.assert_verify_error("missing archive paths.*empty")

    def test_extra_empty_directory_is_rejected(self) -> None:
        (self.fixture.package_root / "extra-empty").mkdir()
        self.assert_verify_error("extra paths.*extra-empty")

    def test_non_regular_cargo_ok_is_not_ignored(self) -> None:
        marker = self.fixture.package_root / ".cargo-ok"
        marker.unlink()
        marker.mkdir()
        self.assert_verify_error("extra paths.*cargo-ok")

    def test_unpacked_symlink_is_rejected(self) -> None:
        link = self.fixture.package_root / "source-link"
        try:
            link.symlink_to("LICENSE")
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.assert_verify_error("link or special entry")

    def test_archive_bytes_remain_authoritative_after_equality_check(self) -> None:
        snapshot = self.fixture.verify()
        (self.fixture.package_root / "LICENSE").write_bytes(b"changed after verify\n")
        (self.fixture.package_root / "src/lib.rs").write_bytes(b"changed after verify\n")
        locked = self.fixture.locked()
        key = generator.package_key(locked)
        package = self.fixture.package()

        # Adapter selections by keyword, not position: a positional call is what
        # made an unrelated signature change look like a test failure.
        notice = generator.generate_notice(
            babl="OFF",
            lcms="OFF",
            metadata_by_key={key: package},
            locked=[locked],
            registry_sources={key: snapshot},
        )
        self.assertIn(b"authoritative license", notice)
        self.assertNotIn(b"changed after verify", notice)

        records = dict(generator.registry_source_records([locked], {key: snapshot}))
        self.assertEqual(
            records[f"vendor/{self.fixture.archive_root}/src/lib.rs"],
            self.fixture.archive_files["src/lib.rs"],
        )
        self.assertEqual(
            records[f"vendor/{self.fixture.archive_root}/LICENSE"],
            self.fixture.archive_files["LICENSE"],
        )

    def test_unsafe_archive_member_paths_are_rejected(self) -> None:
        unsafe_names = {
            "absolute": f"/{self.fixture.archive_root}/LICENSE",
            "backslash": f"{self.fixture.archive_root}\\LICENSE",
            "dot-dot": f"{self.fixture.archive_root}/../LICENSE",
            "dot": f"{self.fixture.archive_root}/./LICENSE",
            "wrong-root": "other-1.0.0/LICENSE",
        }
        for label, name in unsafe_names.items():
            with self.subTest(label=label):
                self.fixture.write_archive([(name, "file", b"data")])
                with self.assertRaisesRegex(RuntimeError, "unsafe|canonical|wrong root"):
                    generator.registry_archive_snapshot(
                        self.fixture.archive_path, self.fixture.locked()
                    )
        self.fixture.write_pax_path_archive(
            f"{self.fixture.archive_root}/bad\0name"
        )
        with self.assertRaisesRegex(RuntimeError, "unsafe"):
            generator.registry_archive_snapshot(
                self.fixture.archive_path, self.fixture.locked()
            )

    def test_duplicate_archive_member_is_rejected(self) -> None:
        name = f"{self.fixture.archive_root}/LICENSE"
        self.fixture.write_archive([(name, "file", b"one"), (name, "file", b"two")])
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            generator.registry_archive_snapshot(self.fixture.archive_path, self.fixture.locked())

    def test_archive_links_and_special_entries_are_rejected(self) -> None:
        for kind in ("symlink", "hardlink", "special"):
            with self.subTest(kind=kind):
                self.fixture.write_archive(
                    [(f"{self.fixture.archive_root}/unsafe", kind, b"")]
                )
                with self.assertRaisesRegex(RuntimeError, "link or special"):
                    generator.registry_archive_snapshot(
                        self.fixture.archive_path, self.fixture.locked()
                    )

    def test_archive_file_directory_conflicts_are_rejected(self) -> None:
        root = self.fixture.archive_root
        conflicts = (
            [(f"{root}/node", "file", b"file"), (f"{root}/node/child", "file", b"child")],
            [(f"{root}/node/child", "file", b"child"), (f"{root}/node", "file", b"file")],
        )
        for members in conflicts:
            with self.subTest(members=members):
                self.fixture.write_archive(members)
                with self.assertRaisesRegex(RuntimeError, "file/directory conflict"):
                    generator.registry_archive_snapshot(
                        self.fixture.archive_path, self.fixture.locked()
                    )

    def test_registry_package_root_name_must_match_lock(self) -> None:
        locked = self.fixture.locked()
        key = generator.package_key(locked)
        package = self.fixture.package()
        wrong_root = self.fixture.package_root.parent / "wrong-root"
        wrong_root.mkdir()
        (wrong_root / "Cargo.toml").write_bytes(b"[package]\n")
        package["manifest_path"] = str(wrong_root / "Cargo.toml")
        with self.assertRaisesRegex(RuntimeError, "package root does not match"):
            generator.verified_registry_sources({key: package}, [locked])

    def test_registry_archive_symlink_is_rejected(self) -> None:
        archive_data = self.fixture.archive_path.read_bytes()
        archive_target = self.fixture.archive_path.with_suffix(".real-crate")
        archive_target.write_bytes(archive_data)
        self.fixture.archive_path.unlink()
        try:
            self.fixture.archive_path.symlink_to(archive_target)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        locked = {
            **self.fixture.locked(),
            "checksum": hashlib.sha256(archive_data).hexdigest(),
        }
        key = generator.package_key(locked)
        with self.assertRaisesRegex(RuntimeError, "not a regular file"):
            generator.verified_registry_sources(
                {key: self.fixture.package()}, [locked]
            )

    def test_already_vendored_checksum_validation_is_preserved(self) -> None:
        vendor_root = Path(self.temporary.name) / "vendor" / "demo-1.2.3"
        vendor_root.mkdir(parents=True)
        (vendor_root / "LICENSE").write_bytes(b"vendor license\n")
        locked = {**self.fixture.locked(), "checksum": "package-checksum"}
        checksum = {
            "files": {
                "LICENSE": hashlib.sha256(b"vendor license\n").hexdigest(),
            },
            "package": "package-checksum",
        }
        (vendor_root / ".cargo-checksum.json").write_text(
            json.dumps(checksum), encoding="utf-8"
        )
        snapshot = generator.vendored_source_snapshot(vendor_root, locked)
        self.assertEqual(snapshot.files, {"LICENSE": b"vendor license\n"})
        (vendor_root / "LICENSE").write_bytes(b"tampered\n")
        with self.assertRaisesRegex(RuntimeError, "vendored file checksum mismatch"):
            generator.vendored_source_snapshot(vendor_root, locked)


class GitDependencyTests(unittest.TestCase):
    """A git dependency (redrob-engine-client from redrob-code) has no registry archive and no
    Cargo checksum: what the lock pins is the commit, and the checkout Cargo made must be at it."""

    SOURCE_URL = "https://example.invalid/redrob-code"

    def make_checkout(self, root: Path) -> tuple[Path, str]:
        checkout = root / "checkout"
        package = checkout / "crates" / "engine-client"
        package.mkdir(parents=True)
        (checkout / "LICENSE").write_bytes(b"MIT at the repository root\n")
        (checkout / "NOTICE.md").write_bytes(b"notice\n")
        (checkout / "README.md").write_bytes(b"not a licence\n")
        (package / "Cargo.toml").write_bytes(b"[package]\nname = \"redrob-engine-client\"\n")
        (package / "src").mkdir()
        (package / "src" / "lib.rs").write_bytes(b"pub fn x() {}\n")
        (package / ".cargo-ok").write_bytes(b"")
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.invalid",
        }
        for command in (
            ["git", "init", "-q"],
            ["git", "add", "-A"],
            ["git", "commit", "-q", "-m", "pin"],
        ):
            subprocess.run(command, cwd=checkout, env=env, check=True, capture_output=True)
        commit = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=checkout, env=env, check=True, capture_output=True, text=True
        ).stdout.strip()
        return package, commit

    def locked(self, commit: str) -> dict:
        return {
            "name": "redrob-engine-client",
            "version": "0.1.0",
            "source": f"git+{self.SOURCE_URL}?rev={commit}#{commit}",
        }

    def test_git_source_parses_and_renders_the_vendor_config(self) -> None:
        commit = "8d5aa70adc6db724904f1c05691c3c8c511ed242"
        parsed = generator.parse_git_source(f"git+{self.SOURCE_URL}?rev={commit}#{commit}")
        self.assertEqual(parsed.url, self.SOURCE_URL)
        self.assertEqual((parsed.kind, parsed.reference, parsed.commit), ("rev", commit, commit))
        self.assertIn(f'[source."git+{self.SOURCE_URL}?rev={commit}#{commit}"]', parsed.vendor_source_table)
        self.assertIn('replace-with = "vendored-sources"', parsed.vendor_source_table)
        tagged = generator.parse_git_source(f"git+{self.SOURCE_URL}?tag=v0.6.3#{commit}")
        self.assertEqual((tagged.kind, tagged.reference), ("tag", "v0.6.3"))
        with self.assertRaises(RuntimeError):
            generator.parse_git_source(f"git+{self.SOURCE_URL}?rev=abc#abc")
        with self.assertRaises(RuntimeError):
            generator.parse_git_source(f"git+{self.SOURCE_URL}#{commit}")
        with self.assertRaises(RuntimeError):
            generator.parse_git_source("registry+https://example.invalid/index")

    def test_checkout_at_the_locked_commit_is_snapshotted_with_the_root_licence(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            package, commit = self.make_checkout(Path(temp))
            snapshot = generator.git_checkout_snapshot(package, self.locked(commit))
            self.assertEqual(
                sorted(snapshot.files),
                ["Cargo.toml", "LICENSE", "NOTICE.md", "src/lib.rs"],
                "the package files plus the repository-root licence texts, never README or .cargo-ok",
            )
            self.assertFalse(snapshot.archive_authoritative)

    def test_checkout_at_another_commit_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            package, commit = self.make_checkout(Path(temp))
            other = "0" * 40
            with self.assertRaisesRegex(RuntimeError, "does not match Cargo.lock commit"):
                generator.git_checkout_snapshot(package, self.locked(other))
            self.assertNotEqual(commit, other)

    def test_symlink_in_a_git_package_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            package, commit = self.make_checkout(Path(temp))
            (package / "link").symlink_to("Cargo.toml")
            with self.assertRaisesRegex(RuntimeError, "refuses symlink"):
                generator.git_checkout_snapshot(package, self.locked(commit))

    def test_bundle_records_carry_a_null_checksum_and_the_git_source_table(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            package, commit = self.make_checkout(Path(temp))
            locked = self.locked(commit)
            snapshot = generator.git_checkout_snapshot(package, locked)
            records = dict(
                generator.registry_source_records([locked], {generator.package_key(locked): snapshot})
            )
            checksum = json.loads(records["vendor/redrob-engine-client-0.1.0/.cargo-checksum.json"])
            self.assertIsNone(checksum["package"])
            self.assertIn("src/lib.rs", checksum["files"])
            config = records[".cargo/config.toml"].decode("utf-8")
            self.assertIn(f'[source."git+{self.SOURCE_URL}?rev={commit}#{commit}"]', config)
            self.assertIn(f'git = "{self.SOURCE_URL}"', config)
            self.assertIn(f'rev = "{commit}"', config)


if __name__ == "__main__":
    unittest.main()
