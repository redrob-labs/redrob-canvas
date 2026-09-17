#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate and verify offline distribution notices and corresponding source artifacts."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tarfile
from typing import Any, Iterable

ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "Cargo.lock"
NOTICE_PATH = ROOT / "THIRD_PARTY_NOTICES.md"
SOURCE_PATH = ROOT / "SOURCE_OFFER.md"
SOURCE_IDENTITY_PATH = ROOT / ".redrob-source-identity.json"
LICENSE_PREFIXES = (
    "LICENSE",
    "LICENCE",
    "COPYING",
    "NOTICE",
    "COPYRIGHT",
    "UNLICENSE",
    "AUTHORS",
)
EXCLUDED_SOURCE_PARTS = {
    ".git",
    ".kiro",
    "build",
    "target",
    "semantic-review",
    "__pycache__",
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(command: list[str], *, allow_failure: bool = False) -> str:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if completed.returncode != 0 and not allow_failure:
        raise RuntimeError(
            f"command failed ({' '.join(command)}):\n{completed.stderr.strip()}"
        )
    return completed.stdout.strip() if completed.returncode == 0 else ""


def cargo_metadata() -> dict[str, Any]:
    cargo = os.environ.get("CARGO", "cargo")
    output = run(
        [cargo, "metadata", "--locked", "--offline", "--format-version", "1"]
    )
    return json.loads(output)


def lock_packages() -> list[dict[str, Any]]:
    packages: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for raw_line in LOCK_PATH.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "[[package]]":
            if current is not None:
                packages.append(current)
            current = {}
            continue
        if current is None or " = " not in line:
            continue
        key, value = line.split(" = ", 1)
        if key in {"name", "version", "source", "checksum"}:
            current[key] = json.loads(value)
    if current is not None:
        packages.append(current)
    if any("name" not in package or "version" not in package for package in packages):
        raise RuntimeError("could not parse Cargo.lock package inventory")
    return sorted(
        packages,
        key=lambda package: (
            package["name"],
            package["version"],
            package.get("source", ""),
        ),
    )


def package_key(package: dict[str, Any]) -> tuple[str, str, str]:
    return (
        package["name"],
        package["version"],
        package.get("source") or "",
    )


@dataclass(frozen=True)
class SourceSnapshot:
    files: dict[str, bytes]
    directories: frozenset[str]
    archive_authoritative: bool


def metadata_packages(
    metadata: dict[str, Any], locked: list[dict[str, Any]]
) -> dict[tuple[str, str, str], dict[str, Any]]:
    metadata_by_key = {package_key(package): package for package in metadata["packages"]}
    locked_keys = {package_key(package) for package in locked}
    metadata_keys = set(metadata_by_key)
    if locked_keys != metadata_keys:
        missing = sorted(locked_keys - metadata_keys)
        extra = sorted(metadata_keys - locked_keys)
        raise RuntimeError(
            f"Cargo.lock/metadata package mismatch; missing={missing}, extra={extra}"
        )
    return metadata_by_key


def archive_member_path(name: str, root: str, *, directory: bool) -> str:
    if "\0" in name or "\\" in name or name.startswith("/"):
        raise RuntimeError(f"unsafe registry archive member path: {name!r}")
    candidate = name[:-1] if directory and name.endswith("/") else name
    parts = candidate.split("/")
    if not candidate or any(part in {"", ".", ".."} for part in parts):
        raise RuntimeError(f"non-canonical registry archive member path: {name!r}")
    if parts[0] != root:
        raise RuntimeError(
            f"registry archive member has wrong root (expected {root!r}): {name!r}"
        )
    relative_parts = parts[1:]
    if not relative_parts:
        if not directory:
            raise RuntimeError(f"registry archive root is not a directory: {name!r}")
        return ""
    return "/".join(relative_parts)


def registry_archive_snapshot(
    archive_path: Path, locked: dict[str, Any]
) -> SourceSnapshot:
    archive_data = archive_path.read_bytes()
    if sha256(archive_data) != locked.get("checksum"):
        raise RuntimeError(
            f"registry archive checksum does not match Cargo.lock: {package_key(locked)}"
        )

    expected_root = f"{locked['name']}-{locked['version']}"
    files: dict[str, bytes] = {}
    directories: set[str] = set()
    explicit_paths: set[str] = set()
    saw_member = False
    try:
        with tarfile.open(fileobj=io.BytesIO(archive_data), mode="r:*") as archive:
            for member in archive:
                saw_member = True
                if not member.isdir() and not member.isfile():
                    raise RuntimeError(
                        f"registry archive contains link or special entry: {member.name!r}"
                    )
                relative = archive_member_path(
                    member.name, expected_root, directory=member.isdir()
                )
                if relative in explicit_paths:
                    raise RuntimeError(
                        f"duplicate registry archive member: {member.name!r}"
                    )
                explicit_paths.add(relative)

                parents: list[str] = []
                if relative:
                    components = relative.split("/")
                    parents = ["/".join(components[:index]) for index in range(1, len(components))]
                for parent in parents:
                    if parent in files:
                        raise RuntimeError(
                            f"registry archive file/directory conflict: {member.name!r}"
                        )
                    directories.add(parent)

                if member.isdir():
                    if relative in files:
                        raise RuntimeError(
                            f"registry archive file/directory conflict: {member.name!r}"
                        )
                    if relative:
                        directories.add(relative)
                    continue
                if relative in directories:
                    raise RuntimeError(
                        f"registry archive file/directory conflict: {member.name!r}"
                    )
                extracted = archive.extractfile(member)
                if extracted is None:
                    raise RuntimeError(
                        f"could not read registry archive member: {member.name!r}"
                    )
                files[relative] = extracted.read()
    except tarfile.TarError as error:
        raise RuntimeError(f"invalid registry archive: {archive_path}") from error
    if not saw_member:
        raise RuntimeError(f"registry archive is empty: {archive_path}")
    return SourceSnapshot(files, frozenset(directories), True)


def unpacked_registry_snapshot(package_root: Path) -> SourceSnapshot:
    files: dict[str, bytes] = {}
    directories: set[str] = set()

    def visit(directory: Path) -> None:
        with os.scandir(directory) as entries:
            for entry in sorted(entries, key=lambda item: item.name):
                path = Path(entry.path)
                relative = path.relative_to(package_root).as_posix()
                mode = path.lstat().st_mode
                if stat.S_ISDIR(mode):
                    directories.add(relative)
                    visit(path)
                elif stat.S_ISREG(mode):
                    if relative == ".cargo-ok":
                        continue
                    files[relative] = path.read_bytes()
                else:
                    raise RuntimeError(
                        f"unpacked registry source contains link or special entry: {path}"
                    )

    visit(package_root)
    return SourceSnapshot(files, frozenset(directories), False)


def compare_registry_snapshots(
    archive: SourceSnapshot,
    unpacked: SourceSnapshot,
    locked: dict[str, Any],
) -> None:
    archive_types = {
        **{path: "directory" for path in archive.directories},
        **{path: "file" for path in archive.files},
    }
    unpacked_types = {
        **{path: "directory" for path in unpacked.directories},
        **{path: "file" for path in unpacked.files},
    }
    type_mismatches = sorted(
        path
        for path in archive_types.keys() & unpacked_types.keys()
        if archive_types[path] != unpacked_types[path]
    )
    if type_mismatches:
        raise RuntimeError(
            f"unpacked registry source type mismatch for {package_key(locked)}: {type_mismatches}"
        )
    missing = sorted(archive_types.keys() - unpacked_types.keys())
    if missing:
        raise RuntimeError(
            f"unpacked registry source missing archive paths for {package_key(locked)}: {missing}"
        )
    extra = sorted(unpacked_types.keys() - archive_types.keys())
    if extra:
        raise RuntimeError(
            f"unpacked registry source has extra paths for {package_key(locked)}: {extra}"
        )
    changed = sorted(
        path
        for path, data in archive.files.items()
        if unpacked.files.get(path) != data
    )
    if changed:
        raise RuntimeError(
            f"unpacked registry source content mismatch for {package_key(locked)}: {changed}"
        )


def vendored_source_snapshot(
    package_root: Path, locked: dict[str, Any]
) -> SourceSnapshot:
    checksum_path = package_root / ".cargo-checksum.json"
    if not checksum_path.is_file():
        raise RuntimeError(f"missing vendored checksum metadata: {checksum_path}")
    bundled_checksum = json.loads(checksum_path.read_text(encoding="utf-8"))
    if bundled_checksum.get("package") != locked.get("checksum"):
        raise RuntimeError(
            f"vendored package checksum does not match Cargo.lock: {package_key(locked)}"
        )
    files: dict[str, bytes] = {}
    for path in sorted(package_root.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"vendored source refuses symlink: {path}")
        if not path.is_file() or path.name in {".cargo-ok", ".cargo-checksum.json"}:
            continue
        relative = path.relative_to(package_root).as_posix()
        files[relative] = path.read_bytes()
    file_checksums = {path: sha256(data) for path, data in files.items()}
    if bundled_checksum.get("files") != file_checksums:
        raise RuntimeError(f"vendored file checksum mismatch: {package_key(locked)}")
    return SourceSnapshot(files, frozenset(), False)


def verified_registry_sources(
    metadata_by_key: dict[tuple[str, str, str], dict[str, Any]],
    locked: list[dict[str, Any]],
) -> dict[tuple[str, str, str], SourceSnapshot]:
    sources: dict[tuple[str, str, str], SourceSnapshot] = {}
    for locked_package in locked:
        source = locked_package.get("source")
        if not source:
            continue
        if not source.startswith("registry+"):
            raise RuntimeError(
                "distribution artifacts do not support non-registry dependency "
                f"{package_key(locked_package)}"
            )
        key = package_key(locked_package)
        package = metadata_by_key[key]
        package_root = Path(package["manifest_path"]).resolve().parent
        expected_root = f"{locked_package['name']}-{locked_package['version']}"
        if package_root.name != expected_root:
            raise RuntimeError(
                "registry package root does not match Cargo.lock "
                f"(expected {expected_root!r}): {package_root}"
            )
        if package_root.is_relative_to(ROOT / "vendor"):
            sources[key] = vendored_source_snapshot(package_root, locked_package)
            continue
        source_bucket = package_root.parent
        archive_path = (
            source_bucket.parent.parent
            / "cache"
            / source_bucket.name
            / f"{expected_root}.crate"
        )
        try:
            archive_mode = archive_path.lstat().st_mode
        except OSError as error:
            raise RuntimeError(f"missing registry archive: {archive_path}") from error
        if not stat.S_ISREG(archive_mode):
            raise RuntimeError(f"registry archive is not a regular file: {archive_path}")
        archive_snapshot = registry_archive_snapshot(archive_path, locked_package)
        unpacked_snapshot = unpacked_registry_snapshot(package_root)
        compare_registry_snapshots(archive_snapshot, unpacked_snapshot, locked_package)
        sources[key] = archive_snapshot
    return sources


def markdown_cell(value: str) -> str:
    return value.replace("|", "\\|").replace("\n", " ")


def display_source(source: str | None) -> str:
    return source or "workspace source"


def package_notice_files(
    package: dict[str, Any], snapshot: SourceSnapshot | None
) -> list[tuple[str, bytes]]:
    package_root = Path(package["manifest_path"]).resolve().parent
    if snapshot is not None:
        candidates: set[str] = set()
        license_file = package.get("license_file")
        if license_file:
            candidate = (package_root / license_file).resolve()
            if not candidate.is_relative_to(package_root):
                raise RuntimeError(f"unsafe notice path: {candidate}")
            candidates.add(candidate.relative_to(package_root).as_posix())
        for relative in snapshot.files:
            if "/" not in relative and Path(relative).name.upper().startswith(
                LICENSE_PREFIXES
            ):
                candidates.add(relative)
        if not any(
            Path(relative).name.upper().startswith(
                ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE")
            )
            for relative in candidates
        ):
            for fallback in ("README.md", "AUTHORS"):
                if fallback in snapshot.files:
                    candidates.add(fallback)
        if not candidates:
            raise RuntimeError(
                f"{package['name']} {package['version']} has no local license/notice text"
            )
        missing = sorted(candidates - snapshot.files.keys())
        if missing:
            raise RuntimeError(
                f"unsafe or missing notice paths for {package_key(package)}: {missing}"
            )
        return [
            (relative, snapshot.files[relative])
            for relative in sorted(candidates, key=str.casefold)
        ]

    candidates: set[Path] = set()
    license_file = package.get("license_file")
    if license_file:
        candidates.add((package_root / license_file).resolve())
    for child in package_root.iterdir():
        if child.is_file() and child.name.upper().startswith(LICENSE_PREFIXES):
            candidates.add(child.resolve())
    candidates.add((ROOT / "LICENSE").resolve())
    if not candidates:
        raise RuntimeError(
            f"{package['name']} {package['version']} has no local license/notice text"
        )
    for candidate in candidates:
        if not candidate.is_file() or (
            not candidate.is_relative_to(package_root) and candidate != ROOT / "LICENSE"
        ):
            raise RuntimeError(f"unsafe or missing notice path: {candidate}")
    return [
        (
            "LICENSE"
            if path == ROOT / "LICENSE"
            else path.relative_to(package_root).as_posix(),
            path.read_bytes(),
        )
        for path in sorted(candidates, key=lambda path: path.name.casefold())
    ]


def generate_notice(
    gegl: str,
    krita: str,
    metadata_by_key: dict[tuple[str, str, str], dict[str, Any]],
    locked: list[dict[str, Any]],
    registry_sources: dict[tuple[str, str, str], SourceSnapshot],
) -> bytes:
    corpus: dict[str, dict[str, Any]] = {}
    rows: list[str] = []
    for locked_package in locked:
        key = package_key(locked_package)
        package = metadata_by_key[key]
        license_expression = package.get("license")
        if not license_expression:
            raise RuntimeError(
                f"{package['name']} {package['version']} has no declared license expression"
            )
        references: list[str] = []
        snapshot = registry_sources.get(key)
        for label, data in package_notice_files(package, snapshot):
            try:
                text = data.decode("utf-8")
            except UnicodeDecodeError as error:
                raise RuntimeError(
                    f"notice is not UTF-8: {package_key(package)} {label}"
                ) from error
            digest = sha256(data)
            references.append(f"{label}@{digest[:16]}")
            record = corpus.setdefault(
                digest,
                {"data": data, "text": text, "origins": set()},
            )
            record["origins"].add(
                f"{package['name']} {package['version']} — {label}"
            )
        checksum = locked_package.get("checksum", "workspace")
        rows.append(
            "| `{}` | `{}` | {} | `{}` | `{}` | {} |".format(
                markdown_cell(package["name"]),
                markdown_cell(package["version"]),
                markdown_cell(display_source(package.get("source"))),
                markdown_cell(checksum),
                markdown_cell(license_expression),
                "<br>".join(f"`{markdown_cell(reference)}`" for reference in references),
            )
        )

    lock_bytes = LOCK_PATH.read_bytes()
    lines = [
        "<!-- Generated by tools/generate_distribution_artifacts.py; do not edit. -->",
        "# Complete third-party notices",
        "",
        "This artifact was generated without network access from the exact `Cargo.lock` resolution, Cargo metadata, and locally available crate source files. It inventories every resolved workspace and registry package, records each exact source identifier, registry checksum, declared license expression, and locally packaged license, notice, or attribution text. The corpus is a UTF-8 rendering; each heading records the SHA-256 and byte length of the original local file.",
        "",
        f"- Cargo.lock SHA-256: `{sha256(lock_bytes)}`",
        f"- Resolved packages: **{len(locked)}**",
        f"- Unique reproduced license/notice texts: **{len(corpus)}**",
        "- Generation mode: Cargo metadata was read with `--locked --offline`; no network content was used.",
        "",
        "## Native and system dependency status",
        "",
        "- **Qt 6:** required system/toolchain dependency, version 6.8 or newer; linked components are Core, Concurrent, Gui, Qml, Quick, QuickControls2, and Svg. Qt is not vendored or represented in Cargo.lock. Redistributors must preserve the notices and source/relocation obligations of the exact Qt package they ship; a system-provided Qt is not copied into the source bundle.",
        f"- **GEGL:** build selection `{gegl}`. When ON, CMake requires the system pkg-config module `gegl-0.4 >= 0.4.66`; when OFF, no GEGL library is linked or shipped.",
        f"- **Krita:** build selection `{krita}`. The optional scaffold verifies source commit `fdbf33b2146735465bb8aa59928fbc1890ceb160` but does not link Krita libraries or expose product operations. When OFF, no Krita source or binary is consumed.",
        "- **Platform libraries:** Unix builds link the system `dl`, `pthread`, and `m` interfaces. These are system/toolchain dependencies and are not vendored.",
        "- **Build-only tools:** CMake 3.21 or newer, a C/C++20 toolchain, Rust/Cargo 1.92, Python 3, and Qt build tools are required to configure, generate compliance artifacts, and compile; they are not installed or bundled by this project.",
        "- **Project assets:** the Redrob SVG icon and QML sources are GPL-3.0-or-later project files. The `font8x8` package notice and its bundled bitmap provenance are reproduced in the corpus below.",
        "",
        "## Cargo.lock package inventory",
        "",
        "| Package | Version | Source | Cargo checksum | License expression | Local text references |",
        "|---|---:|---|---|---|---|",
        *rows,
        "",
        "## Exact local license and notice corpus",
        "",
        "Each section is keyed by the SHA-256 of the original local file bytes. Identical texts used by multiple packages are reproduced once.",
        "",
    ]
    for digest in sorted(corpus):
        record = corpus[digest]
        text = record["text"]
        trailing_newline = "yes" if record["data"].endswith((bytes([10]), bytes([13]))) else "no"
        fence = "`" * max(4, max((len(part) for part in text.split("`") if part == ""), default=0) + 1)
        lines.extend(
            [
                f"### `{digest}`",
                "",
                "Used by:",
                *[f"- {origin}" for origin in sorted(record["origins"])],
                "",
                f"Original byte length: `{len(record['data'])}`; trailing newline: `{trailing_newline}`.",
                "",
                f"{fence}text",
                text + ("" if text.endswith(("\n", "\r")) else "\n"),
                fence,
                "",
            ]
        )
    return "\n".join(lines).encode("utf-8")


def archived_source_identity() -> dict[str, str]:
    if not SOURCE_IDENTITY_PATH.is_file():
        return {}
    value = json.loads(SOURCE_IDENTITY_PATH.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or not all(
        isinstance(value.get(key, ""), str)
        for key in ("branch", "commit", "repository", "remote")
    ):
        raise RuntimeError("invalid archived source identity")
    return value


def git_identity() -> tuple[str, str]:
    branch = run(["git", "symbolic-ref", "--short", "HEAD"], allow_failure=True)
    commit = run(["git", "rev-parse", "--verify", "HEAD"], allow_failure=True)
    if branch or commit:
        return branch or "(detached or unavailable)", commit
    archived = archived_source_identity()
    return archived.get("branch", "(detached or unavailable)"), archived.get("commit", "")


def git_remote() -> str:
    # Read the repository-local value without Git's URL rewrite rules so the
    # distributed identity records the public origin, not a credential-routing URL.
    remote = run(
        ["git", "config", "--local", "--get", "remote.origin.url"],
        allow_failure=True,
    )
    if remote:
        return remote
    return archived_source_identity().get("remote", "")


def declared_repository() -> str:
    in_workspace_package = False
    for raw_line in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line.startswith("["):
            in_workspace_package = line == "[workspace.package]"
        elif in_workspace_package and line.startswith("repository = "):
            repository = json.loads(line.split(" = ", 1)[1])
            if isinstance(repository, str) and repository:
                return repository
    raise RuntimeError("Cargo.toml is missing workspace.package.repository")


def generate_source_manifest(notice: bytes, gegl: str, krita: str) -> bytes:
    repository = declared_repository()
    content = f"""<!-- Generated by tools/generate_distribution_artifacts.py; do not edit. -->
# Source offer and corresponding-source manifest

Redrob Canvas is licensed under GPL-3.0-or-later. Every distributable install produced by this repository installs this source offer, the complete generated third-party notices, and `redrob-canvas-corresponding-source.tar.gz`. The archive contains the project source used by the build, the exact checksum-verified Cargo registry sources under `vendor/`, an offline Cargo source-replacement configuration, and an internal `SOURCE_BUNDLE_SHA256SUMS` covering every archived file.

## Exact source identity

- Declared repository URL: `{repository}`
- Exact bundle identity: each corresponding-source archive contains `.redrob-source-identity.json`, recording the Git branch, actual commit when one exists, configured origin, and declared repository URL at bundle generation time. An unborn worktree records an empty commit; no commit hash is fabricated.
- Cargo.lock SHA-256: `{sha256(LOCK_PATH.read_bytes())}`
- Third-party notice SHA-256: `{sha256(notice)}`
- Native selection represented by these artifacts: `REDROB_ENABLE_GEGL={gegl}`, `REDROB_ENABLE_KRITA={krita}`.

## Obtaining the same source

Recipients should first use the installed `redrob-canvas-corresponding-source.tar.gz`; it is the authoritative exact source snapshot and its internal checksum manifest covers every archived file. The archive includes every registry package source selected by `Cargo.lock`, verifies each package against its Cargo checksum metadata, and configures Cargo to use only the bundled `vendor/` directory. Its `.redrob-source-identity.json` records the generation-time source identity; when the commit field is non-empty, recipients can additionally clone the declared repository URL over HTTPS and check out that commit. All package license/notice texts used for this build are reproduced in `THIRD_PARTY_NOTICES.md` without network access.

Qt is a required system/toolchain dependency (6.8 or newer; Core, Concurrent, Gui, Qml, Quick, QuickControls2, Svg). GEGL is a system pkg-config dependency only when enabled (`gegl-0.4 >= 0.4.66`). The Krita option is a source-verified, non-linking scaffold at commit `fdbf33b2146735465bb8aa59928fbc1890ceb160`. Exact Qt/GEGL binaries are not copied into the project source archive; recipients may use compatible system packages or obtain their corresponding source from the distributor of those packages under the applicable license terms.

## Reproducing and building

```sh
python3 tools/generate_distribution_artifacts.py --check --gegl {gegl} --krita {krita}
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cmake -S native -B build/qt -DREDROB_ENABLE_GEGL={gegl} -DREDROB_ENABLE_KRITA={krita}
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
cmake --install build/qt --prefix /desired/prefix
```

The CMake build runs the offline artifact check and creates the corresponding-source archive as an `ALL` target. Installation fails if the generated notice, this manifest, or the source archive is absent. To refresh after changing `Cargo.lock`, the source tree, or adapter selection, run:

```sh
python3 tools/generate_distribution_artifacts.py --generate --gegl {gegl} --krita {krita}
```
"""
    return content.encode("utf-8")


def source_files() -> Iterable[Path]:
    for path in ROOT.rglob("*"):
        if path.is_symlink():
            raise RuntimeError(f"source bundle refuses symlink: {path}")
        if not path.is_file():
            continue
        relative = path.relative_to(ROOT)
        if relative.parts[0] == "vendor" or relative.as_posix() in {
            ".cargo/config.toml",
            SOURCE_IDENTITY_PATH.name,
        }:
            continue
        if any(part in EXCLUDED_SOURCE_PARTS for part in relative.parts):
            continue
        if path.name.endswith((".tar.gz", ".pyc")):
            continue
        yield path


def registry_source_records(
    locked: list[dict[str, Any]],
    registry_sources: dict[tuple[str, str, str], SourceSnapshot],
) -> list[tuple[str, bytes]]:
    records: list[tuple[str, bytes]] = []
    vendor_names: set[str] = set()
    for locked_package in locked:
        if not locked_package.get("source"):
            continue
        key = package_key(locked_package)
        snapshot = registry_sources[key]
        vendor_name = f"{locked_package['name']}-{locked_package['version']}"
        if vendor_name in vendor_names:
            raise RuntimeError(f"ambiguous vendored package directory: {vendor_name}")
        vendor_names.add(vendor_name)
        file_checksums = {
            relative: sha256(data) for relative, data in snapshot.files.items()
        }
        records.extend(
            (f"vendor/{vendor_name}/{relative}", data)
            for relative, data in sorted(snapshot.files.items())
        )
        checksum_data = json.dumps(
            {"files": file_checksums, "package": locked_package["checksum"]},
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        records.append((f"vendor/{vendor_name}/.cargo-checksum.json", checksum_data))
    config = b'''# Generated inside the corresponding-source bundle.\n[source.crates-io]\nreplace-with = "vendored-sources"\n\n[source.vendored-sources]\ndirectory = "vendor"\n\n[net]\noffline = true\n'''
    records.append((".cargo/config.toml", config))
    return records


def render_source_bundle(
    locked: list[dict[str, Any]],
    registry_sources: dict[tuple[str, str, str], SourceSnapshot],
) -> bytes:
    project_records = [
        (path.relative_to(ROOT).as_posix(), path.read_bytes())
        for path in source_files()
    ]
    branch, commit = git_identity()
    identity = json.dumps(
        {
            "branch": branch,
            "commit": commit,
            "remote": git_remote(),
            "repository": declared_repository(),
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    project_records.append((SOURCE_IDENTITY_PATH.name, identity))
    records = sorted(
        [*project_records, *registry_source_records(locked, registry_sources)],
        key=lambda record: record[0],
    )
    names = [name for name, _ in records]
    if len(names) != len(set(names)):
        raise RuntimeError("duplicate path in corresponding-source bundle")
    manifest = "".join(f"{sha256(data)}  {name}\n" for name, data in records).encode()
    records.append(("SOURCE_BUNDLE_SHA256SUMS", manifest))

    tar_bytes = io.BytesIO()
    with tarfile.open(fileobj=tar_bytes, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, data in records:
            info = tarfile.TarInfo(f"redrob-canvas-source/{name}")
            info.size = len(data)
            info.mode = 0o644
            info.mtime = 0
            info.uid = 0
            info.gid = 0
            info.uname = "root"
            info.gname = "root"
            archive.addfile(info, io.BytesIO(data))
    output = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as compressed:
        compressed.write(tar_bytes.getvalue())
    return output.getvalue()


def build_source_bundle(
    destination: Path,
    locked: list[dict[str, Any]],
    registry_sources: dict[tuple[str, str, str], SourceSnapshot],
) -> str:
    content = render_source_bundle(locked, registry_sources)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(content)
    return sha256(content)


def check_source_bundle(
    destination: Path,
    locked: list[dict[str, Any]],
    registry_sources: dict[tuple[str, str, str], SourceSnapshot],
) -> str:
    actual = destination.read_bytes()
    expected = render_source_bundle(locked, registry_sources)
    if actual != expected:
        raise RuntimeError(f"stale or incomplete source bundle: {destination}")
    return sha256(expected)


def write_if_changed(path: Path, content: bytes) -> None:
    if path.is_file() and path.read_bytes() == content:
        return
    path.write_bytes(content)


def main() -> int:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--source-bundle", type=Path)
    mode.add_argument("--check-source-bundle", type=Path)
    parser.add_argument("--gegl", choices=("ON", "OFF"), default="OFF")
    parser.add_argument("--krita", choices=("ON", "OFF"), default="OFF")
    args = parser.parse_args()

    metadata = cargo_metadata()
    locked = lock_packages()
    metadata_by_key = metadata_packages(metadata, locked)
    registry_sources = verified_registry_sources(metadata_by_key, locked)
    expected_notice = generate_notice(
        args.gegl, args.krita, metadata_by_key, locked, registry_sources
    )
    expected_source = generate_source_manifest(
        expected_notice, args.gegl, args.krita
    )

    if args.source_bundle or args.check_source_bundle:
        if NOTICE_PATH.read_bytes() != expected_notice or SOURCE_PATH.read_bytes() != expected_source:
            raise RuntimeError("generated distribution artifacts are stale; run --generate")
        if args.source_bundle:
            destination = args.source_bundle.resolve()
            digest = build_source_bundle(destination, locked, registry_sources)
            print(f"source bundle: {destination} sha256={digest}")
        else:
            destination = args.check_source_bundle.resolve()
            digest = check_source_bundle(destination, locked, registry_sources)
            print(f"checked source bundle: {destination} sha256={digest}")
        return 0

    notice = expected_notice
    source = expected_source
    if args.generate:
        write_if_changed(NOTICE_PATH, notice)
        write_if_changed(SOURCE_PATH, source)
        print(f"generated {NOTICE_PATH.name} sha256={sha256(notice)}")
        print(f"generated {SOURCE_PATH.name} sha256={sha256(source)}")
        return 0

    failures = []
    for path, expected in ((NOTICE_PATH, notice), (SOURCE_PATH, source)):
        if not path.is_file():
            failures.append(f"missing {path.name}")
        elif path.read_bytes() != expected:
            failures.append(f"stale {path.name}")
        else:
            print(f"checked {path.name} sha256={sha256(expected)}")
    if failures:
        print("; ".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError) as error:
        print(f"distribution artifact error: {error}", file=sys.stderr)
        raise SystemExit(1)
