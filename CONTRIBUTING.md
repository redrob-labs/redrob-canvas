# Contributing

Thanks for helping. This is the working agreement for the repository: how branches are named, what
has to be green before a merge, and the one thing that makes this repository unusual.

## What this is, and is not

Redrob Canvas is **a new product architecture, not a UI fork.** Krita and GIMP stay pinned as
executable sources of truth for behaviour, while that behaviour is moved behind a typed Rust command
system and a QML surface.

The division of responsibility is the thing to keep straight:

- **Painting behaviour follows Krita.** If the two disagree, Krita is right and the bug is ours.
- **Image processing follows GIMP and GEGL.**
- **Product UX and agent workflows are ours**, and owe nothing to either.

So a pull request that changes brush behaviour needs to say which upstream it was checked against.
"It feels better" is not a reason to diverge from a pinned reference; measuring the reference and
finding it different is.

## Branch model

One long-lived branch, `main`. Short-lived branches off it, merged by pull request. That is the whole
model — there is no `develop`, no release branch and no long-lived integration branch to keep in sync.

- **`main`** is the trunk. A GitHub ruleset enforces it rather than trusting this document:
  - no direct pushes — every change arrives as a pull request;
  - no force pushes and no deletion of the branch;
  - linear history, so `main` reads as a list of changes rather than a graph;
  - required status checks must pass, and the branch must be up to date with `main` first;
  - review threads must be resolved before merge.
- **Working branches** are `<type>/<short-slug>`, e.g. `fix/selection-feather`,
  `feat/agent-tool-loop`. Types: `feat`, `fix`, `chore`, `docs`, `test`, `refactor`, `perf`.
- **Merging is squash-only**, and the branch is deleted on merge. One pull request becomes one commit
  on `main`, so `git log main` is the changelog. The squash commit's body is the pull request body,
  not a concatenation of your work-in-progress messages.
- Release tags are cut from `main`, formatted `v<major>.<minor>.<patch>`.

Fork the repository, push your branch to your fork, and open the pull request from there. You do not
need write access to contribute, and pull requests from forks run CI with no repository secrets — the
workflow declares `permissions: contents: read` and consumes none.

[redrob-code](https://github.com/redrob-labs/redrob-code) runs Git Flow with a `develop` branch.
This one does not, so cut from `main`.

## Day to day

```bash
git switch main && git pull
git switch -c feat/short-description

cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# open a pull request into main
```

Commands go through the typed command system in `crates/redrob-core`. A QML surface that mutates a
document directly is the thing this architecture exists to prevent: it bypasses history, so undo
silently stops covering it.

### Commits

Subject in the imperative. Use the body to say _why_, and when a change follows an upstream reference,
name the version you checked it against.

## What CI checks

`.github/workflows/verify-upstream.yml` runs on every pull request and on pushes to `main`. It
declares `permissions: contents: read` and consumes no secrets, so a pull request from a fork gets the
same run as one from a branch here. Three jobs, all required before a merge:

| Check | What it asserts |
| --- | --- |
| **Upstream pins are well-formed and reachable** | the pins in `docs/upstream-sources.toml` parse, resolve upstream, and `upstream/` still cannot be committed |
| **Rust workspace** | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace --locked` |
| **Distribution artifacts are current** | `tools/generate_distribution_artifacts.py --check` — the generated `THIRD_PARTY_NOTICES.md` and `SOURCE_OFFER.md` match the tree |

The pins job is what keeps "follows Krita" a checkable statement rather than a claim. It deliberately
does **not** download the 753 MB of upstream trees.

The distribution job is a licence gate, not a tidiness one: this project is GPL-3.0-or-later and every
install ships the notices and the corresponding-source archive, so a stale pair is a compliance
defect. If you change a dependency, the archive name, `LICENSE`, or `COPYRIGHT`, rerun
`python3 tools/generate_distribution_artifacts.py --generate --gegl OFF --krita OFF` and commit the
result. The generator reads Cargo metadata with `--locked --offline` on purpose — that offline-ness is
asserted in the notices themselves — so run `cargo fetch --locked` first on a cold cache.
