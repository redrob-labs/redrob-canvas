#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Generate and verify THIRD_PARTY_NOTICES.md from the exact Cargo.lock, offline.

    python3 tools/generate_distribution_artifacts.py --generate   # rewrite the file
    python3 tools/generate_distribution_artifacts.py --check      # exit 1 when it is stale

The file inventories every package `Cargo.lock` resolves (name, version, source, registry checksum,
declared licence expression) and reproduces, byte for byte, the licence and notice texts shipped
inside each package's own source as Cargo unpacked it. Nothing is fetched while it runs: metadata
is read with `cargo metadata --locked --offline`, so on a cold machine run `cargo fetch --locked`
first (the two CI jobs that call this do exactly that).

This used to also write SOURCE_OFFER.md and a GPL corresponding-source bundle. Redrob Canvas is
MIT OR Apache-2.0 since 0.7.0 and carries no copyleft input, so those are gone; what remains is
the attribution the permissive licences ask for.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
LOCK_PATH = ROOT / "Cargo.lock"
NOTICE_PATH = ROOT / "THIRD_PARTY_NOTICES.md"
LICENSE_PREFIXES = (
    "LICENSE",
    "LICENCE",
    "COPYING",
    "NOTICE",
    "COPYRIGHT",
    "UNLICENSE",
    "AUTHORS",
)
# Fallbacks when a package ships no licence file at all; the declared SPDX expression still names
# the terms, and the README is where small crates state their copyright.
FALLBACK_FILES = ("README.md", "README", "AUTHORS")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cargo_metadata() -> dict[str, Any]:
    cargo = os.environ.get("CARGO", "cargo")
    completed = subprocess.run(
        [cargo, "metadata", "--locked", "--offline", "--format-version", "1"],
        cwd=ROOT,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "cargo metadata --locked --offline failed; run `cargo fetch --locked` first:\n"
            + completed.stderr.strip()
        )
    return json.loads(completed.stdout)


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
    return sorted(packages, key=lambda p: (p["name"], p["version"], p.get("source", "")))


def package_key(package: dict[str, Any]) -> tuple[str, str, str]:
    return (package["name"], package["version"], package.get("source") or "")


def metadata_by_key(
    metadata: dict[str, Any], locked: list[dict[str, Any]]
) -> dict[tuple[str, str, str], dict[str, Any]]:
    by_key = {package_key(p): p for p in metadata["packages"]}
    locked_keys = {package_key(p) for p in locked}
    if locked_keys != set(by_key):
        missing = sorted(locked_keys - set(by_key))
        extra = sorted(set(by_key) - locked_keys)
        raise RuntimeError(f"Cargo.lock/metadata package mismatch; missing={missing}, extra={extra}")
    return by_key


def package_notice_files(package: dict[str, Any]) -> list[tuple[str, bytes]]:
    """The licence/notice files inside the package's own unpacked source, relative path and bytes."""
    package_root = Path(package["manifest_path"]).resolve().parent
    candidates: set[Path] = set()
    license_file = package.get("license_file")
    if license_file:
        candidate = (package_root / license_file).resolve()
        if candidate.is_relative_to(package_root) and candidate.is_file():
            candidates.add(candidate)
    for child in package_root.iterdir():
        if child.is_file() and child.name.upper().startswith(LICENSE_PREFIXES):
            candidates.add(child.resolve())
    if not candidates:
        for name in FALLBACK_FILES:
            fallback = package_root / name
            if fallback.is_file():
                candidates.add(fallback.resolve())
                break
    return [
        (path.relative_to(package_root).as_posix(), path.read_bytes())
        for path in sorted(candidates, key=lambda p: p.name.casefold())
    ]


def markdown_cell(value: str) -> str:
    return value.replace("|", "\\|").replace("\n", " ")


def generate_notice(
    by_key: dict[tuple[str, str, str], dict[str, Any]], locked: list[dict[str, Any]]
) -> bytes:
    corpus: dict[str, dict[str, Any]] = {}
    rows: list[str] = []
    for locked_package in locked:
        package = by_key[package_key(locked_package)]
        license_expression = package.get("license") or (
            f"see {package['license_file']}" if package.get("license_file") else ""
        )
        if not license_expression:
            raise RuntimeError(
                f"{package['name']} {package['version']} declares no license expression"
            )
        references: list[str] = []
        for label, data in package_notice_files(package):
            text = data.decode("utf-8", errors="replace")
            digest = sha256(data)
            references.append(f"{label}@{digest[:16]}")
            record = corpus.setdefault(digest, {"data": data, "text": text, "origins": set()})
            record["origins"].add(f"{package['name']} {package['version']} - {label}")
        checksum = locked_package.get("checksum")
        if not checksum:
            source = locked_package.get("source") or ""
            checksum = f"git:{source.rsplit('#', 1)[-1]}" if source.startswith("git+") else "workspace"
        rows.append(
            "| `{}` | `{}` | {} | `{}` | `{}` | {} |".format(
                markdown_cell(package["name"]),
                markdown_cell(package["version"]),
                markdown_cell(package.get("source") or "workspace source"),
                markdown_cell(checksum),
                markdown_cell(license_expression),
                "<br>".join(f"`{markdown_cell(r)}`" for r in references) or "(none shipped)",
            )
        )

    lines = [
        "<!-- Generated by tools/generate_distribution_artifacts.py; do not edit. -->",
        "# Complete third-party notices",
        "",
        "This file was generated without network access from the exact `Cargo.lock` resolution, "
        "Cargo metadata read with `--locked --offline`, and the licence and notice files inside each "
        "resolved package's own source as Cargo unpacked it. It inventories every workspace, registry "
        "and git package the build resolves; the corpus below reproduces each distinct licence or "
        "notice text once, keyed by the SHA-256 of the original bytes.",
        "",
        "Redrob Canvas itself is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`). "
        "The four vendored editors under `crafts/` and their own NOTICE texts are recorded in `NOTICE` "
        "and `UPSTREAM_NOTICES.md`; the workspace crates they contribute appear in the inventory below "
        "as `workspace source`.",
        "",
        f"- Cargo.lock SHA-256: `{sha256(LOCK_PATH.read_bytes())}`",
        f"- Resolved packages: **{len(locked)}**",
        f"- Unique reproduced license/notice texts: **{len(corpus)}**",
        "- Generation mode: `cargo metadata --locked --offline`; no network content was used.",
        "",
        "## Cargo.lock package inventory",
        "",
        "| Package | Version | Source | Cargo checksum | License expression | Local text references |",
        "|---|---:|---|---|---|---|",
        *rows,
        "",
        "## Exact local license and notice corpus",
        "",
        "Each section is keyed by the SHA-256 of the original local file bytes. Identical texts used "
        "by multiple packages are reproduced once.",
        "",
    ]
    for digest in sorted(corpus):
        record = corpus[digest]
        text = record["text"]
        trailing = "yes" if record["data"].endswith((b"\n", b"\r")) else "no"
        # A fence longer than any backtick run inside the text, so the text cannot close it.
        longest_run = max((len(run) for run in re.findall(r"`+", text)), default=0)
        fence = "`" * max(4, longest_run + 1)
        lines.extend(
            [
                f"### `{digest}`",
                "",
                "Used by:",
                *[f"- {origin}" for origin in sorted(record["origins"])],
                "",
                f"Original byte length: `{len(record['data'])}`; trailing newline: `{trailing}`.",
                "",
                f"{fence}text",
                text + ("" if text.endswith(("\n", "\r")) else "\n"),
                fence,
                "",
            ]
        )
    return "\n".join(lines).encode("utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()

    locked = lock_packages()
    by_key = metadata_by_key(cargo_metadata(), locked)
    expected = generate_notice(by_key, locked)

    if args.generate:
        if not NOTICE_PATH.is_file() or NOTICE_PATH.read_bytes() != expected:
            NOTICE_PATH.write_bytes(expected)
        print(f"generated {NOTICE_PATH.name} sha256={sha256(expected)}")
        return 0

    if not NOTICE_PATH.is_file():
        print(f"missing {NOTICE_PATH.name}; run --generate", file=sys.stderr)
        return 1
    actual = NOTICE_PATH.read_bytes()
    if actual == expected:
        print(f"checked {NOTICE_PATH.name} sha256={sha256(expected)}")
        return 0
    # "stale" alone is not a diagnosis, and this runs on hosts nobody is sitting at: print a
    # bounded diff. A checkout that rewrote LF to CRLF reads as stale too, and regenerating cannot
    # fix that -- the comparison is on bytes -- so say so when that is the shape of the difference.
    diff = list(
        difflib.unified_diff(
            expected.decode("utf-8", "replace").splitlines(),
            actual.decode("utf-8", "replace").splitlines(),
            fromfile=f"{NOTICE_PATH.name} (expected, regenerated now)",
            tofile=f"{NOTICE_PATH.name} (on disk)",
            lineterm="",
            n=1,
        )
    )
    print(f"--- how {NOTICE_PATH.name} differs ({len(diff)} diff lines) ---", file=sys.stderr)
    for line in diff[:60]:
        print(line, file=sys.stderr)
    if len(diff) > 60:
        print(f"... {len(diff) - 60} further diff lines suppressed", file=sys.stderr)
    message = f"stale {NOTICE_PATH.name}; run `python3 tools/generate_distribution_artifacts.py --generate`"
    if actual.count(b"\r\n") > expected.count(b"\r\n"):
        message += (
            "; NOTE: the file on disk carries more CRLF line endings than the generated content,"
            " so check .gitattributes and core.autocrlf before regenerating"
        )
    print(message, file=sys.stderr)
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError) as error:
        print(f"distribution artifact error: {error}", file=sys.stderr)
        raise SystemExit(1)
