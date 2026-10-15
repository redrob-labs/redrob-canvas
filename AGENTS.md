# AGENTS.md

Operating notes for an agent working in `redrob-labs/redrob-canvas`. Everything here was read out
of this tree, out of the organisation's `.github` repository, or out of a command that was actually
run. Where a fact was not verified it says so.

## What this repository is

Redrob Canvas is an agentic graphics editor: one egui/eframe window (`apps/redrob-canvas`) hosting
three complete editors as modes, each vendored whole as a git subtree under `crafts/` from
[storytold](https://github.com/storytold): the Photo editor (Photo), the Vector editor (Vector), the Layout editor
(Layout), plus their shared craft-libs. Redrob's own code is the shell, `crates/redrob-agent` (the
Redrob Code client) and `crates/redrob-ai` (the AI tools layer, in progress). Everything is Rust;
there is no Qt, CMake, Node or QML anywhere in the build. If a document, script or comment you find
talks about Qt, `native/`, `qml/`, `redrob-core`, `redrob-ffi`, Krita or GIMP, it describes the tree
before commit `d4854f2` and is stale.

The project is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`). Until 0.6 it was
GPL-3.0-or-later; the relicensing rests on no copyleft input remaining, and
`scripts/verify-upstream.sh` refuses any `code` pin whose licence is not permissive. Do not vendor or
translate GPL or LGPL source into this tree.

## The two rules you cannot learn late

1. **Never edit a file under `crafts/` by hand.** The four directories are git subtrees. A fix an
   editor needs goes upstream (the storytold repository) and comes back by
   `scripts/fetch-upstream.sh <name> <sha>`; a local edit turns the next pull into a conflict hunt and
   is lost at the next squash. The one exception is the parked `crafts/<name>/Cargo.upstream.toml`,
   which `tools/merge_workspaces.py` renames and which you also do not edit.
2. **Never replace an upstream copyright notice, and never touch `crafts/*/docs/brand/`.** The upstream
   name and logos there are trademarks, not open source. `NOTICE` reproduces each upstream NOTICE
   verbatim beneath ours; `upstream-base.json`'s `attribution` list says which strings must stay, and
   the verify script fails when one goes missing or when anything outside `crafts/` references the
   brand assets.

## Layout

- `Cargo.toml` at the root is **generated** by `tools/merge_workspaces.py` from the four
  `crafts/*/Cargo.upstream.toml`; its header says so. Edit the script, run it, commit both. The
  workspace members are `crates/*`, `apps/*` and each subtree's `crates/*`; the subtrees' `apps/`,
  `xtask` and web shells are excluded. `Cargo.lock` is one lock file for all of it.
- `apps/redrob-canvas/` is the shell: `main.rs` (args, eframe options), `shell.rs` (`Mode`, lazy
  editors, top bar), `services.rs` (config directory, prefs). Config lives in `ai.redrob.canvas`'s
  directory (`~/Library/Application Support/Redrob Canvas`, `%APPDATA%\Redrob Canvas`,
  `~/.config/redrob-canvas`).
- `crates/redrob-agent/` is the pure-Rust Redrob Code client; `crates/redrob-ai/` the AI layer.
- `UPSTREAM.md` + `upstream-base.json` + `docs/upstream-sources.toml` are the pins; `UPSTREAM_NOTICES.md`
  the hand-written attribution; `THIRD_PARTY_NOTICES.md` the generated one (do not edit).
- `tools/`: `merge_workspaces.py`, `generate_distribution_artifacts.py`, `check_design_system.py`,
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

The gates, in the order CI runs them. The first and the last three need no Rust build and finish in
seconds; run them first to fail fast.

```bash
scripts/verify-upstream.sh                                   # pins, attribution, brand boundary, GitHub reachability
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p redrob-canvas                       # the shell links
cargo fetch --locked && python3 tools/generate_distribution_artifacts.py --check
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"
```

Verified in this tree on 2026-10-15: `scripts/verify-upstream.sh` (exit 0, also with `--with-trees`),
`tools/check_design_system.py` (exit 0), the tool tests (28 tests, OK), and the workflow files parse
as YAML. `cargo clippy`, `cargo test` and `cargo build` were not run while these notes were written
(another change to `apps/` was in flight), so their status on this branch is not asserted here.

The workspace is large (four editors); a cold `cargo build --release` is long. Prefer
`cargo check -p redrob-canvas` or `cargo test -p <crate>` while iterating, and never wait on a build
with a shell loop that greps for its own command (`while pgrep -f 'cargo ...'` matches itself).

Running the app: `target/release/redrob-canvas [--mode photo|vector|layout]`; `--version` prints and
exits, which is what the release smoke test relies on.

## Distribution artifacts

`THIRD_PARTY_NOTICES.md` is GENERATED and committed. `tools/generate_distribution_artifacts.py
--check` regenerates it in memory from `Cargo.lock` and `cargo metadata --locked --offline` and fails
on any byte difference; `--generate` writes it. Run `--generate` after any change to `Cargo.lock`
and commit the result. On a cold machine run `cargo fetch --locked` first; the generator never
touches the network and that is deliberate. If `cargo metadata --locked` itself fails, `Cargo.lock`
is out of step with a `Cargo.toml` and that is the thing to fix, not the generator.

`SOURCE_OFFER.md` and the corresponding-source bundle no longer exist. Do not recreate them.

## Upstream pins

Update a subtree with `scripts/fetch-upstream.sh <name> <sha>` on a clean tree: it runs
`git subtree pull --prefix crafts/<name> <repo> <sha> --squash`, then `tools/merge_workspaces.py`,
and commits the regenerated manifest. Then record the sha in `upstream-base.json`
(`base.commit`, `lastSyncedUpstream`), `docs/upstream-sources.toml` (`commit`) and the subtree's
section of `UPSTREAM_NOTICES.md`, run `scripts/verify-upstream.sh`, regenerate the notices if
`Cargo.lock` moved, and open the pull request from a `sync/upstream-<name>-<sha7>` branch. Merge it
as a **merge commit**, never a squash: a squash destroys the subtree's merge base.

The verify script also checks that `redrob_code`'s tag in `docs/upstream-sources.toml` equals the
`redrob-engine-client` tag in the root `Cargo.toml`; bump both together.

## What CI actually runs

`.github/workflows/verify-upstream.yml` runs on every pull request and on pushes to `develop` and
`main`: `Upstream pins are well-formed and reachable`, `Rust workspace` (fmt, clippy, test, release
build of the shell), `Distribution artifacts are current` (cargo fetch, the notice check, the design
guard, the tool tests) and `Rust security advisories` (`cargo audit`). `Rust workspace` and
`Distribution artifacts are current` are the two checks branch protection required before the rewrite;
whether the protection rules were updated for this branch was not verified here.

`.github/workflows/gitflow.yml` and `pr-policy.yml` are the organisation templates and only call
`redrob-labs/.github`: branch names are `<type>/<slug>` from the org list
(`feat fix chore docs test refactor perf ci build sync hotfix release`), `main` must be contained in
`develop`, at most three open pull requests per author.

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
- Working branches are `<type>/<slug>` off `develop`, squash-merged; upstream syncs are merge commits.
- New source files carry `SPDX-License-Identifier: MIT OR Apache-2.0`. Files under `crafts/` keep
  upstream's headers.
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
