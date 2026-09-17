# Contributing

**English** · [한국어](./CONTRIBUTING.ko.md)

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

Two long-lived branches. `develop` is where work lands; `main` is what has been released.

- **`develop`** is the default branch and the integration branch. Cut working branches from it and
  open pull requests back into it. Clone the repository and you are on `develop`.
- **`main`** is the released state. It takes pull requests only from `release/*` and `hotfix/*`
  branches, and release tags are cut from it. Nothing else merges here.
- **Working branches** are `<type>/<short-slug>` off `develop`, e.g. `fix/svg-namespace`, `feat/layer-blend`. Types: `feat`,
  `fix`, `chore`, `docs`, `test`, `refactor`, `perf`.
- **Merging is squash-only**, and the branch is deleted on merge. One pull request becomes one
  commit, so `git log develop` reads as a list of changes rather than a graph. The squash commit's
  body is the pull request body, not a concatenation of your work-in-progress messages.

Both branches are enforced by GitHub rulesets rather than by this document:

- no direct pushes — every change arrives as a pull request;
- no force pushes and no deletion of the branch;
- linear history;
- required status checks must pass — they do not have to pass against the newest tip, so a queue of
  bot updates does not have to rebase and re-run one at a time;
- review threads must be resolved before merge.

A ruleset cannot express *which* branch a pull request comes from, so "only `release/*` and
`hotfix/*` merge into `main`" is a convention this document carries and reviewers uphold. One part
of it is machine-checked: the release workflow refuses to build a tag whose commit is not reachable
from `origin/main`, so tagging straight off `develop` fails instead of shipping.

### Releasing

```bash
git switch develop && git pull
git switch -c release/v0.2.0
# bump the version, update the changelog, run the release check
# open a pull request into main and merge it, then tag main:
git switch main && git pull
git tag -a v0.2.0 -m "Redrob Canvas v0.2.0"
git push origin v0.2.0
# bring main's release commit back so develop does not fall behind:
git switch -c chore/sync-main-to-develop main
# open a pull request into develop
```

A hotfix is the same shape with `hotfix/*` cut from `main` rather than `develop`, and it merges into
both.

Fork the repository, push your branch to your fork, and open the pull request from there. You do not
need write access to contribute, and pull requests from forks run CI with no repository secrets.

## Day to day

```bash
git switch develop && git pull
git switch -c feat/short-description

cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# open a pull request into develop
```

Commands go through the typed command system in `crates/redrob-core`. A QML surface that mutates a
document directly is the thing this architecture exists to prevent: it bypasses history, so undo
silently stops covering it.

### Commits

Subject in the imperative. Use the body to say _why_, and when a change follows an upstream reference,
name the version you checked it against.

## What CI checks

`.github/workflows/verify-upstream.yml` runs on every pull request and on pushes to `main` and `develop`. It
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
