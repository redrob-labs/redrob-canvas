# AGENTS.md

Operating notes for an agent working in `redrob-labs/redrob-canvas`. Everything here was read out
of this tree, out of the GitHub branch-protection API, or out of a command that was actually run.
Where a fact was not verified it says so.

## What this repository is

Redrob Canvas is a cross-platform, agentic graphics editor: a typed Rust core plus a Qt 6 / QML
desktop shell that talks to it across a stable C ABI. It is a new product architecture and not a
fork of an existing editor: Krita and GIMP are pinned by exact commit in
`docs/upstream-sources.toml` and used as executable references for painting and image-processing
behaviour, so a claim like "this matches Krita" can be checked against a specific tree instead of
being remembered. Painting behaviour follows Krita, image processing follows GIMP and GEGL, and the
product UX and agent workflows are first-party.

The whole project is GPL-3.0-or-later. That is not a header detail: two committed files and one
build target exist only to satisfy the licence, and CI will fail a pull request that lets them go
stale. See "Distribution artifacts" below, and read it before you touch `Cargo.lock`.

## Branch model

`develop` is the default branch and the base of every pull request. Clone the repository and you
land on `develop`. `main` is released state and moves only by merging `develop` forward, in
practice through a `release/*` branch. Release tags are cut from `main`, and the release workflow
enforces that: its `validate` job runs `git merge-base --is-ancestor "$GITHUB_SHA" origin/main` and
fails the release outright if the tag commit is not reachable from `origin/main`, so tagging
straight off `develop` cannot ship.

A hotfix branches from `main`, merges into `main`, releases, and then `main` is merged back into
`develop`. That last step is the one that gets skipped, and skipping it is a real failure with a
real cost: the sibling repository redrob-code spent a month with a `Cargo.lock` its own default
branch could not install from, because a fix that landed on `main` was never brought back.

Both branches are protected identically. Read from the API on 2026-09-17: pull requests only, force
pushes blocked, branch deletion blocked, `required_approving_review_count` 0, admin enforcement
off, and `strict` false so a check does not have to pass against the newest tip. Two checks are
required to merge:

- `Rust workspace`
- `Distribution artifacts are current`

Note a gap between the document and the setting. `CONTRIBUTING.md` says three jobs are required
before a merge, but `Upstream pins are well-formed and reachable` is NOT in either branch's
required-check list. It still runs on every pull request; it just cannot block one. Run
`./scripts/verify-upstream.sh` yourself rather than trusting the merge button.

### Which branch work is actually landing on

Measured on 2026-09-17, after pull request #16 merged:

```
git rev-list --count origin/develop..origin/main   ->  0
git rev-list --count origin/main..origin/develop   ->  1
```

So `main` holds nothing that `develop` lacks, and `develop` is one commit ahead. There is no
back-merge debt right now, and this is the correct shape: work accumulates on `develop`, `main`
trails until a release.

That is a recent correction, and the reason to check the numbers before you trust them. Pull
requests #1 through #15 all targeted `main` even though `develop` was already the default branch,
which means fifteen changes bypassed the integration branch entirely. Pull request #14 adopted the
two-branch model and wrote it into `CONTRIBUTING.md`, and #16 is the first pull request in the
repository's history to target `develop`. The same inversion is how a sibling repository ended up
with a default branch 22 commits behind its own released state.

One open pull request, #17, targets `main` from `release/v0.1.0`. That is not the inversion: a
`release/*` branch into `main` is exactly the documented release path. The thing to be suspicious of
is a `feat/`, `fix/`, `chore/`, `docs/`, `test/`, `refactor/` or `perf/` branch aimed at `main`.

Your own pull request goes into `develop`, from a `<type>/<short-slug>` branch cut off `develop`.
Merges are squash-only and the branch is deleted on merge, so the squash body should be the pull
request body.

## Layout

`Cargo.toml` at the root is a cargo workspace, `resolver = "3"`, with exactly three members:

- `crates/redrob-core` is the document model, layers, raster engine, selection, commands, history,
  and the format codecs (RRG, PNG, JPEG, lossless WebP, OpenRaster, a deliberately limited SVG
  subset). It is Qt-independent by design, which is why the required Rust check needs no Qt, no
  display, and none of the upstream trees.
- `crates/redrob-agent` is the streaming client for the hosted OpenAI-compatible API, the tool
  loop, and approvals.
- `crates/redrob-ffi` is the stable C ABI the Qt host links against. Its `crate-type` is
  `["staticlib", "cdylib", "rlib"]`, and the header it must stay in step with is committed at
  `crates/redrob-ffi/include/redrob_ffi.h`.

Around them: `native/qt/` is the Qt 6 host (QObject models, canvas bridge, `main.cpp`),
`native/adapters/` holds the optional GEGL and Krita adapter scaffolds, `qml/Main.qml` is the
interface, `tools/` holds the two Python tools described below, `scripts/` holds the two upstream
shell scripts, and `docs/` holds architecture, the compatibility matrix, format routes, licensing
policy and the upstream pins.

The toolchain is pinned: `rust-toolchain.toml` selects `1.92.0` with `clippy` and `rustfmt`, and
`rustfmt.toml` sets `edition = "2024"` with `max_width = 100`. Do not reformat to a different width.

## Commands

These four were run in this repository on 2026-09-17 and all exited 0. They need no Qt, no network
fetch of upstream trees, and no display.

```bash
cargo fmt --all -- --check                                              # exit 0, about 0.2s
./scripts/verify-upstream.sh                                            # exit 0
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF   # exit 0
python3 -m unittest discover -s tools/tests -p "test_*.py"              # exit 0, 18 tests
```

`cargo fmt --all -- --check` is the fastest gate and the cheapest way to fail fast. The other two
Rust commands in the required check were not run during this investigation:

```bash
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Run both before pushing; `-D warnings` means a new Clippy lint fails the merge.

The desktop shell is a separate, much heavier path and needs Qt 6.11 or newer with Core, Concurrent,
Gui, Qml, Quick, QuickControls2 and Svg:

```bash
cmake -S native -B build/qt -DCMAKE_BUILD_TYPE=Release \
  -DREDROB_ENABLE_GEGL=OFF -DREDROB_ENABLE_KRITA=OFF
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
```

The floor is 6.11 and the comment in `native/qt/CMakeLists.txt` says why it is not higher: it was
briefly 6.12, and every macOS and Windows release leg died because Qt's own mirror had not published
6.12.0's archive checksums. There is a headless, Qt-free adapter check as well:

```bash
cmake -S native -B build/adapters -DREDROB_BUILD_QT=OFF
cmake --build build/adapters
ctest --test-dir build/adapters --output-on-failure
```

`native/CMakeLists.txt` registers the Python generator tests as the ctest test
`redrob_distribution_artifacts_python` under `BUILD_TESTING`, so a CMake build runs them. No GitHub
workflow does. If you change `tools/generate_distribution_artifacts.py`, run the unittest command
above by hand, because nothing in CI will catch you.

There is no `package.json`, no `justfile` and no `Makefile`. Do not look for one.

## Distribution artifacts, and the mistake to avoid

`THIRD_PARTY_NOTICES.md` and `SOURCE_OFFER.md` are GENERATED and committed. Editing either by hand
is the first thing an agent gets wrong here.

`tools/generate_distribution_artifacts.py` builds both from `Cargo.lock`, from
`cargo metadata --locked --offline`, and from the licence files inside each locked crate's own
source. Nothing is fetched over the network while it runs, and `THIRD_PARTY_NOTICES.md` asserts
that property, which is why the offline flags stay.

- `--check` regenerates both in memory and compares them to the files byte for byte. On any
  difference it prints `stale THIRD_PARTY_NOTICES.md` or `stale SOURCE_OFFER.md` to stderr and
  exits 1. A hand edit is therefore not merely unhelpful, it fails the required
  `Distribution artifacts are current` check and it fails a local CMake configure.
- `--generate` is the only correct way to change them. Run
  `python3 tools/generate_distribution_artifacts.py --generate --gegl OFF --krita OFF` after any
  change to `Cargo.lock`, to a licence-relevant file, or to the adapter selection, and commit the
  result.
- The adapter flags are part of the identity of the output, so a check run with different `--gegl` /
  `--krita` values than the build uses will disagree.

`native/qt/CMakeLists.txt` runs `--check` at CONFIGURE time via `execute_process` and raises
`FATAL_ERROR` if it exits nonzero, so a stale pair stops the build before anything compiles. The
same file declares `redrob_source_bundle` as an `ALL` target that the `redrob-canvas` executable
depends on, which is how the deterministic GPL corresponding-source archive gets produced by an
ordinary build rather than by a step someone can forget.

One cold-cache trap, and it bites in CI and on a fresh clone alike: because the generator reads
Cargo metadata with `--locked --offline`, an empty registry index makes that read fail with
`no matching package named font8x8`, and the failure looks like a broken generator rather than a
missing cache. Run `cargo fetch --locked` first. Both CI jobs that touch the generator do exactly
that, and the fix is never to drop `--offline`.

## What CI actually runs

Two workflow files, and only one of them ever sees a pull request.

`.github/workflows/verify-upstream.yml` triggers on `pull_request` (no branch filter, so every pull
request), on `push` to `main` and `develop`, and on `workflow_dispatch`. It declares
`permissions: contents: read` and consumes no secrets, so a fork's pull request gets the same run as
a local branch. Three jobs:

| Job name | What it runs |
| --- | --- |
| `Upstream pins are well-formed and reachable` | `./scripts/verify-upstream.sh` |
| `Rust workspace` | `cargo fmt --all -- --check`, then `cargo clippy --workspace --all-targets --locked -- -D warnings`, then `cargo test --workspace --locked` |
| `Distribution artifacts are current` | `cargo fetch --locked`, then the generator's `--check --gegl OFF --krita OFF` |

The last two are the required checks. The first is not, as noted above.

`.github/workflows/release.yml` triggers ONLY on a pushed `v*.*.*` tag and on `workflow_dispatch`.
It never runs on a pull request, so nothing in it gates a merge and you cannot get its signal by
opening one. It validates the tag against `origin/main`, re-runs the upstream pins, the distribution
check, fmt, Clippy and the tests, then builds each platform against Qt 6.11.2 installed through
`aqtinstall`, signs and notarizes on macOS, signs with Authenticode on Windows, and uploads to a
DRAFT GitHub Release. Publishing that draft is the release; there is no CDN and no promotion step.
The platform list comes from `tools/release_matrix.py` rather than being typed into the workflow, and
the Windows leg is omitted until the repository variable `WINDOWS_SIGNING_READY` is `"true"`, so an
unsigned Windows binary is never the fallback. A final job re-reads the draft and fails if any built
platform archive, the corresponding-source tarball, `SOURCE_OFFER.md`, `THIRD_PARTY_NOTICES.md`,
`LICENSE`, `COPYRIGHT` or a checksum file is missing.

Tag `v0.1.0` exists and is reachable from `main`, and the corresponding GitHub Release is still a
draft.

Dependabot (`.github/dependabot.yml`) runs weekly for `cargo` and for `github-actions`, the latter
because every action is SHA-pinned and that is the only thing that updates them. There is
deliberately no npm ecosystem: this project has no JavaScript build.

## Upstream pins

`docs/upstream-sources.toml` is the single place a commit hash lives. It pins Krita at
`fdbf33b2146735465bb8aa59928fbc1890ceb160`, GIMP at
`43e3d9601d17e9ebc3fbd7e29158a0ad95ede4fc`, a GEGL minimum of `0.4.66` as a system pkg-config floor
rather than a checkout, and redrob-code at `ee0ffcfe52f3964b4db660f174febb101333a8d5` as a protocol
reference. Each entry carries a `license` field, because the obligation is to audit each reused file
and an entry that quietly loses that note is the one nobody re-reads.

`scripts/fetch-upstream.sh` reads those same pins and shallow-clones Krita and GIMP into
`upstream/`. `scripts/verify-upstream.sh` deliberately does not download anything: it asserts that
`upstream/` is git-ignored (including probe paths inside it), that each pin is a full 40-character
hex sha over an `https` repository with a licence note, and that each remote is reachable via
`git ls-remote`. Pass `--with-trees` to additionally check that a local checkout's HEAD equals its
pin.

`upstream/` is about 753 MB of someone else's source and is ignored by the `/upstream/` rule in
`.gitignore`. Never commit it, and never duplicate one of those commit hashes into a second file.
If you bump a pin, bump it there and nowhere else.

## Conventions worth keeping

Mutations go through the typed command system in `crates/redrob-core`. A QML surface that mutates a
document directly is the specific thing this architecture exists to prevent, because it bypasses
history and undo silently stops covering it.

`.github/PULL_REQUEST_TEMPLATE.md` asks for the commands you actually ran and what they printed, and
its checklist covers the traps above: no upstream tree committed, pins changed only in
`docs/upstream-sources.toml`, the generated artifacts rerun if anything licence-relevant moved,
`LICENSE` still the verbatim GPL-3.0 text, and no GIMP, Krita or GEGL name reaching a user-visible
string except where it legitimately identifies upstream. Fill it in with real output.

Commit subjects are imperative; the body says why, and names the upstream version a behavioural
change was checked against.

Secrets come from the environment. `.env.example` lists `REDROB_API_KEY`, `REDROB_BASE_URL` and
`REDROB_MODEL`, and `.gitignore` excludes `.env` and `.env.*` while keeping `.env.example`. Do not
commit a real key and do not print one.

## Not verified here

`cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace
--locked` were not executed during this investigation, so their current exit status on `develop` is
unknown. Neither the Qt shell build nor `ctest` was run, so nothing here confirms the desktop
application compiles or starts on any particular host. The release workflow's signing, notarization
and upload paths were read, not exercised.
