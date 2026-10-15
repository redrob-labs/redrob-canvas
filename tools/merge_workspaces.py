#!/usr/bin/env python3
"""Build the root Cargo.toml from the subtree manifests under crafts/.

The four storytold upstreams (photocraft, vectorcraft, designcraft, craft-libs) each ship
their own Cargo workspace. Cargo allows one workspace per tree, so their `[workspace]`
tables are folded into ours: dependencies are unioned (features merged, the highest
version kept), the craft-libs git dependencies become path dependencies on the subtree,
and the krilla/hayro patches all point at craft-libs' vendored copies.

Run after every `git subtree pull`:

    python3 tools/merge_workspaces.py && cargo metadata --offline >/dev/null

Each subtree's own `Cargo.toml` is renamed to `Cargo.upstream.toml`: cargo would otherwise
treat it as a nested workspace root and refuse the crates as members of the wrong workspace.
After a `git subtree pull` the upstream file reappears as `Cargo.toml`; this script reads
whichever exists and renames it again.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRAFTS = ["photocraft", "vectorcraft", "designcraft", "craft-libs"]
CRAFT_LIBS = {
    f"craft-{n}": f"crafts/craft-libs/crates/{n}"
    for n in ["atomic", "cms", "pdf", "print", "quit", "settings", "ui"]
}
VENDORED = ["krilla", "hayro", "hayro-interpret", "hayro-syntax", "hayro-jbig2"]

OUR_PACKAGE = {
    "version": "0.7.0",
    "edition": "2024",
    "license": "MIT OR Apache-2.0",
    "rust-version": "1.95",
    "repository": "https://github.com/redrob-labs/redrob-canvas",
}

# Dependencies our own crates need that no upstream declares.
OUR_DEPS = {
    "async-trait": "0.1",
    # redrob-ai sends images to the local IOPaint server as PNG data URLs.
    "base64": "0.22",
    # The shared Redrob Code daemon client; the agent crate talks to that daemon only.
    "redrob-engine-client": {"git": "https://github.com/redrob-labs/redrob-code", "tag": "v0.6.4"},
    "futures-util": "0.3",
    "reqwest": {
        "version": "0.12",
        "default-features": False,
        "features": ["json", "rustls-tls", "stream"],
    },
    "tokio": {"version": "1", "features": ["rt-multi-thread", "macros", "sync", "time", "process"]},
    # The in-app update check verifies the downloaded asset against the release's SHA256SUMS.txt.
    "sha2": "0.10",
    "thiserror": "2",
    "tempfile": "3",
    "mockito": "1",
    "wiremock": "0.6",
}


def toml_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, str):
        return '"' + v.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{k} = {toml_value(x)}" for k, x in v.items()) + " }"
    raise TypeError(type(v))


def version_key(s: str):
    s = s.lstrip("=^~")
    return tuple(int(p) if p.isdigit() else 0 for p in re.split(r"[.-]", s))


def merge_spec(a, b):
    """Union two dependency specs. Keeps the higher version, unions features, and only
    disables default features when every contributor disabled them."""
    da = {"version": a} if isinstance(a, str) else dict(a)
    db = {"version": b} if isinstance(b, str) else dict(b)
    out = dict(da)
    for k, v in db.items():
        if k == "version":
            if "version" not in out or version_key(v) > version_key(out["version"]):
                out["version"] = v
        elif k == "features":
            out["features"] = sorted(set(out.get("features", [])) | set(v))
        elif k == "default-features":
            out["default-features"] = out.get("default-features", True) and v
        elif k in out and out[k] != v:
            raise SystemExit(f"conflicting {k!r} for dependency: {da} vs {db}")
        else:
            out[k] = v
    if out.get("default-features", True) is True:
        out.pop("default-features", None)
    if set(out) == {"version"}:
        return out["version"]
    return out


def main() -> None:
    deps: dict = {}
    lints_rust: dict = {}
    lints_clippy: dict = {}
    for craft in CRAFTS:
        live = ROOT / "crafts" / craft / "Cargo.toml"
        parked = live.with_name("Cargo.upstream.toml")
        if live.exists():
            live.replace(parked)
    # A pull brings upstream's docs, packaging, CI and READMEs back: drop that furniture before
    # the manifests are read (tools/prune_upstream.py).
    sys.path.insert(0, str(ROOT / "tools"))
    import prune_upstream  # noqa: E402

    prune_upstream.main([])
    for craft in CRAFTS:
        parked = ROOT / "crafts" / craft / "Cargo.upstream.toml"
        data = tomllib.load(parked.open("rb"))
        ws = data["workspace"]
        for name, spec in ws.get("dependencies", {}).items():
            if isinstance(spec, dict) and "path" in spec and name not in CRAFT_LIBS:
                # Workspace-level path dependencies are relative to the workspace root,
                # which is now ours, so re-root them onto the subtree.
                spec = dict(spec)
                spec["path"] = f"crafts/{craft}/{spec['path']}"
            if name in CRAFT_LIBS:
                spec = dict(spec) if isinstance(spec, dict) else {}
                for k in ("git", "rev", "version"):
                    spec.pop(k, None)
                spec["path"] = CRAFT_LIBS[name]
            deps[name] = merge_spec(deps[name], spec) if name in deps else spec
        lints = ws.get("lints", {})
        for k, v in lints.get("rust", {}).items():
            lints_rust.setdefault(k, v)
        for k, v in lints.get("clippy", {}).items():
            lints_clippy.setdefault(k, v)
    # The upstream subtrees are linted by their own projects on their own clippy version; on
    # ours (1.95) craft-ui and photocraft-raw trip collapsible_match, and since `-D warnings`
    # covers every workspace member we cannot gate our crates without silencing it here.
    lints_clippy["collapsible_match"] = "allow"
    lints_clippy["nonminimal_bool"] = "allow"
    for name, spec in OUR_DEPS.items():
        deps[name] = merge_spec(deps[name], spec) if name in deps else spec

    # One workspace cannot depend on the same craft-libs crate twice; the path form wins.
    for name, path in CRAFT_LIBS.items():
        if name in deps:
            spec = deps[name] if isinstance(deps[name], dict) else {}
            spec = {k: v for k, v in spec.items() if k not in ("git", "rev", "version")}
            spec["path"] = path
            deps[name] = spec

    lines = [
        "# GENERATED by tools/merge_workspaces.py from crafts/*/Cargo.toml -- edit that script,",
        "# not this file. Re-run it after every `git subtree pull`.",
        "",
        "[workspace]",
        'resolver = "3"',
        "members = [",
        '  "crates/*",',
        '  "apps/*",',
        '  "crafts/photocraft/crates/*",',
        '  "crafts/vectorcraft/crates/*",',
        '  "crafts/designcraft/crates/*",',
        '  "crafts/craft-libs/crates/*",',
        "]",
        "# Upstream binaries and web shells are not built here; the Redrob Canvas shell in apps/",
        "# hosts the three editors as modes. craft-libs vendors krilla/hayro, patched below.",
        "exclude = [",
        '  "crafts/photocraft/apps",',
        '  "crafts/vectorcraft/apps",',
        '  "crafts/designcraft/apps",',
        '  "crafts/craft-libs/vendor",',
        "]",
        "",
        "[workspace.package]",
    ]
    lines += [f"{k} = {toml_value(v)}" for k, v in OUR_PACKAGE.items()]
    lines += ["", "[workspace.dependencies]"]
    for name in sorted(deps):
        lines.append(f"{name} = {toml_value(deps[name])}")
    lines += ["", "[workspace.lints.rust]"]
    lines += [f"{k} = {toml_value(v)}" for k, v in sorted(lints_rust.items())]
    lines += ["", "[workspace.lints.clippy]"]
    lines += [f"{k} = {toml_value(v)}" for k, v in sorted(lints_clippy.items())]
    lines += [
        "",
        "[patch.crates-io]",
    ]
    for name in VENDORED:
        lines.append(f'{name} = {{ path = "crafts/craft-libs/vendor/{name}" }}')
    lines += [
        "# designcraft pins a winit fork for a fix not yet upstream; one patch applies to the tree.",
        'winit = { git = "https://github.com/skylartaylor/winit.git", rev = "3d51077fd1a966f84df9ef287839dc84fd5b4ca7" }',
        "",
        '[profile.dev.package."*"]',
        "opt-level = 2",
        "",
        "[profile.dev]",
        "opt-level = 1",
        "",
        "# CI gates compile with `--profile ci`: no optimisation anywhere (the dev profile optimises every",
        "# dependency at level 2, which is what makes a cold `cargo test` take a quarter of an hour).",
        "[profile.ci]",
        'inherits = "dev"',
        "opt-level = 0",
        'debug = "line-tables-only"',
        "incremental = false",
        "",
        '[profile.ci.package."*"]',
        "opt-level = 0",
        "",
        "[profile.release]",
        'lto = "thin"',
        "codegen-units = 1",
        'panic = "abort"',
        "strip = true",
        "",
    ]
    (ROOT / "Cargo.toml").write_text("\n".join(lines))
    print(f"wrote Cargo.toml: {len(deps)} dependencies")


if __name__ == "__main__":
    main()
