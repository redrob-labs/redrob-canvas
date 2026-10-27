# AGENTS.md

Operating notes for an agent working in `redrob-labs/redrob-canvas`. Everything here was read out
of this tree, out of the organisation's `.github` repository, or out of a command that was actually
run. Where a fact was not verified it says so.

## What this repository is

Redrob Canvas is an agentic graphics editor: one egui/eframe binary (`apps/redrob-canvas`) hosting
three complete editors as modes, Photo, Vector and Layout, under `editors/{photo,vector,layout}`,
plus the libraries they share under `editors/shared`. The same binary is the command line:
`redrob-canvas photo|vector|layout ARGS` (`apps/redrob-canvas/src/cli`) converts, inspects, runs
command batches and serves MCP without opening the window. Redrob's own code is the shell, the
editors, `crates/redrob-agent` (the Redrob Code client) and `crates/redrob-ai` (the AI tools layer,
in progress). Everything is Rust; there is no Qt, CMake, Node or QML anywhere in the build. If a
document, script or comment you find talks about Qt, `native/`, `qml/`, `redrob-core`, `redrob-ffi`,
Krita or GIMP, it describes the tree before commit `d4854f2` and is stale.

The editors started as four MIT OR Apache-2.0 projects and are no longer synced with them: they are
our code now and are edited here like any other file. `SOURCE_ORIGINS.md` records where each one
came from and at which commit.

The project is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`). Until 0.6 it was
GPL-3.0-or-later; the relicensing rests on no copyleft input remaining. Do not vendor or translate
GPL or LGPL source into this tree.

## The rule you cannot learn late

**Never replace an original author's copyright notice.** Ours goes beside theirs. `NOTICE`
reproduces each origin project's NOTICE verbatim beneath ours, `editors/*/NOTICE`,
`editors/*/LICENSE-*` and `editors/*/ATTRIBUTION.md` stay as written, and `COPYRIGHT` names them.

## Layout

- `Cargo.toml` at the root is the one workspace manifest: `crates/*`, `apps/*` and
  `editors/*/crates/*`, with `editors/shared/vendor` (patched `krilla` / `hayro`) patched in by path.
  `Cargo.lock` is one lock file for all of it.
- `apps/redrob-canvas/` is the shell: `main.rs` (args, eframe options), `cli/` (the command line),
  `shell.rs` (`Mode`, lazy editors, top bar), `services.rs` (config directory, prefs). Config lives in `ai.redrob.canvas`'s
  directory (`~/Library/Application Support/Redrob Canvas`, `%APPDATA%\Redrob Canvas`,
  `~/.config/redrob-canvas`).
- `crates/redrob-agent/` is the pure-Rust Redrob Code client; `crates/redrob-ai/` the AI layer.
- `SOURCE_ORIGINS.md` is the hand-written origin record; `THIRD_PARTY_NOTICES.md` the generated
  one (do not edit).
- `tools/`: `generate_distribution_artifacts.py`, `check_design_system.py`,
  `generate_design_tokens.py` (a pin checker), `release_matrix.py`, `release_manifest.py`,
  `release/stage.py`, `release/make_deb.py`, `release/build-linux.sh`, and `tests/`.
- `installer/macos/` (disk image, uninstaller) and `installer/windows/` (setup and uninstaller, Win32).
- `third_party/redrob-ui/` and `resources/` are the design system, pinned in `DESIGN_SYSTEM_PIN.json`.

The toolchain is pinned: `rust-toolchain.toml` selects `1.95.0` with `clippy` and `rustfmt`.
`rustfmt.toml` is the project's; do not reformat to a different width.

## Commands

Set up once on Linux (what CI installs):

```bash
sudo apt-get install -y libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev
export PATH="$HOME/.cargo/bin:$PATH"
```

The gates, in the order CI runs them. The last three need no Rust build and finish in seconds;
run them first to fail fast.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p redrob-canvas                       # the shell links
cargo fetch --locked && python3 tools/generate_distribution_artifacts.py --check
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"
```

Verified in this tree on 2026-10-15: `tools/check_design_system.py` (exit 0), the tool tests (28 tests, OK), and the workflow files parse
as YAML. `cargo clippy`, `cargo test` and `cargo build` were not run while these notes were written
(another change to `apps/` was in flight), so their status on this branch is not asserted here.

The workspace is large (three editors and their libraries); a cold `cargo build --release` is long. Prefer
`cargo check -p redrob-canvas` or `cargo test -p <crate>` while iterating, and never wait on a build
with a shell loop that greps for its own command (`while pgrep -f 'cargo ...'` matches itself).

Running the app: `target/release/redrob-canvas [--mode photo|vector|layout]`; `--version` prints and
exits, which is what the release smoke test relies on. `target/release/redrob-canvas photo --help`
(or `vector`, `layout`) prints that editor's command line.

## Distribution artifacts

`THIRD_PARTY_NOTICES.md` is GENERATED and committed. `tools/generate_distribution_artifacts.py
--check` regenerates it in memory from `Cargo.lock` and `cargo metadata --locked --offline` and fails
on any byte difference; `--generate` writes it. Run `--generate` after any change to `Cargo.lock`
and commit the result. On a cold machine run `cargo fetch --locked` first; the generator never
touches the network and that is deliberate. If `cargo metadata --locked` itself fails, `Cargo.lock`
is out of step with a `Cargo.toml` and that is the thing to fix, not the generator.

`SOURCE_OFFER.md` and the corresponding-source bundle no longer exist. Do not recreate them.

## The `redrob-code` pin

`redrob-engine-client` and `redrob-app-bridge` are git dependencies on tags of
`redrob-labs/redrob-code` (root `Cargo.toml`, `apps/redrob-canvas/Cargo.toml`); bump them together.

## What CI actually runs

`.github/workflows/ci.yml` is the one pull-request workflow (no push or schedule triggers). Job
`checks` (seconds to a couple of minutes): `cargo fetch --locked` then the notice check, the design-system guard, the tool tests. Job `rust`:
`cargo fmt --check`, then clippy and the tests of OUR crates only (`-p redrob-canvas -p redrob-ai
-p redrob-agent`) with `--profile ci`; skipped when no Rust input changed or when the pull request
targets `main`. There is no separate gitflow, packaging-lint or advisory workflow. Branch protection should require `checks` and `rust`.

`.github/workflows/pr-policy.yml` is the organisation template and only calls `redrob-labs/.github`
(at most three open pull requests per author).

`.github/workflows/release.yml` runs only on `workflow_dispatch`:
`gh workflow run release.yml --ref main -f tag=vX.Y.Z` after the tag is pushed on `main`, or
`-f dry_run=true` to build and sign without publishing. It validates the tag against `origin/main`,
`Cargo.toml` and `CHANGELOG.md` (a `## X.Y.Z ` heading that is not `- unreleased`), runs every
gate, builds `redrob-canvas` per platform from `tools/release_matrix.py`'s list, stages with
`tools/release/stage.py`, packages (`.tar.gz` + `.deb`; `.zip` + `.dmg`; `-portable.zip` +
`-setup.exe`), signs macOS (Developer ID, notarize, staple) and Windows (Azure Artifact Signing over
OIDC, login after the build, three signatures: executable, uninstaller, setup), checks every built
asset and licence file is present, and calls the shared `release-publish.yml`, which writes
`SHA256SUMS.txt` and `redrob-launcher.json` and publishes as latest. Windows is built only when the
repository variable `WINDOWS_SIGNING_READY` is `"true"`. `tools/tests/test_release_signing.py` pins
the signing order.

## Conventions worth keeping

- Commits: conventional prefix, imperative, lower-case subject; the body says why. No em dashes
  anywhere. Pull request bodies in English; Korean prose in `README.ko.md` uses 레드롭 캔버스 for the
  product and keeps Latin only for technical tokens.
- Working branches are `<type>/<slug>` off `develop`, squash-merged.
- New source files carry `SPDX-License-Identifier: MIT OR Apache-2.0`. Files from the origin
  projects keep the headers they came with.
- The shell paints from the design-system tokens; a hex literal in `apps/redrob-canvas/src` fails the
  guard unless the line is marked as a document colour.
- This app holds no Redrob console key: the shared Redrob Code daemon does, and the agent reaches
  the console only through it. Do not add a direct console path, store a key, or print one.

## Not verified here

`cargo clippy`, `cargo test` and `cargo build --release -p redrob-canvas` were not executed while
these notes were written. The macOS and Windows release legs (`stage.py` on those hosts, the
`.app` bundle, codesign/notarytool, `installer/windows/build.py` against `bin/redrob-canvas.exe`) were
rewritten for the cargo binary and read, not run; the first dry run of `release.yml` is where they
are proven. `tools/release/make_deb.py`'s `Depends` table for the egui stack was reasoned from what
winit, wgpu and rfd load, not measured on a clean machine.
