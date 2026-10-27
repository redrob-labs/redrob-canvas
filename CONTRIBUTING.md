# Contributing

**English** · [한국어](./CONTRIBUTING.ko.md)

Thanks for helping. This is the working agreement for the repository: how branches are named, what
has to be green before a merge, and the one thing that makes this repository unusual. The
organisation-wide agreement in
[redrob-labs/.github CONTRIBUTING.md](https://github.com/redrob-labs/.github/blob/main/CONTRIBUTING.md)
and [FORKS.md](https://github.com/redrob-labs/.github/blob/main/FORKS.md) applies too; where the two
disagree this file wins, because it knows its own build.

## What this is, and is not

Redrob Canvas is one app with three complete editors as modes: Photo, Vector and Layout, under
`editors/`, with the code they share in `editors/shared`. Around them sit the shell
(`apps/redrob-canvas`, which is also the `redrob-canvas photo|vector|layout` command line), the agent
(`crates/redrob-agent`) and the AI tools layer (`crates/redrob-ai`).

The editors started as four MIT OR Apache-2.0 projects and are no longer synced with them. They are
our code: fix an editor bug in its folder here, like any other change. Keep the origin projects'
copyright lines and NOTICE files as they are (`SOURCE_ORIGINS.md`).

## Branch model

Two long-lived branches. `develop` is where work lands; `main` is what has been released.

- **`develop`** is the default branch and the integration branch. Cut working branches from it and
  open pull requests back into it.
- **`main`** is the released state. It takes pull requests only from `release/*` and `hotfix/*`
  branches, and release tags are cut from it.
- **Working branches** are `<type>/<short-slug>` off `develop`, e.g. `fix/mode-switch-focus`,
  `feat/ai-panel`. The type list is the organisation's one list, checked by the shared
  `gitflow.yml`: `feat fix chore docs test refactor perf ci build sync hotfix release`. A branch name
  says what the change is, never what produced it, so a tool or agent prefix fails.
- **Merging is squash-only** for working branches, and the branch is deleted on merge.
  **Promotions to `main` are merge commits.**

Both branches are protected by GitHub rulesets: no direct pushes, no force pushes, required checks,
review threads resolved. The release workflow refuses to build a tag whose commit is not reachable
from `origin/main`, so tagging straight off `develop` fails instead of shipping.

### Releasing

```bash
git switch develop && git pull
git switch -c release/v0.7.0
# bump `version` under [workspace.package] in the root Cargo.toml, update CHANGELOG.md
# open a pull request into main and merge it (merge commit), then tag main:
git switch main && git pull
git tag -a v0.7.0 -m "Redrob Canvas v0.7.0"
git push origin v0.7.0
gh workflow run release.yml --ref main -f tag=v0.7.0
```

A hotfix is the same shape with `hotfix/*` cut from `main` rather than `develop`, and it merges into
both. The back-merge of `main` into `develop` is opened by the shared gitflow job.

Fork the repository, push your branch to your fork, and open the pull request from there. You do not
need write access to contribute, and pull requests from forks run CI with no repository secrets.

## Day to day

```bash
export PATH="$HOME/.cargo/bin:$PATH"        # rust-toolchain.toml selects 1.95.0
git switch develop && git pull
git switch -c feat/short-description

cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"

# open a pull request into develop
```

If `Cargo.lock` moved, run `cargo fetch --locked` then
`python3 tools/generate_distribution_artifacts.py --generate` and commit `THIRD_PARTY_NOTICES.md`.

The shell paints from the vendored design-system tokens (`third_party/redrob-ui/`); a hex colour
literal in `apps/redrob-canvas/src` fails the guard unless the line is marked as a document colour.

### Commits

Subject in the imperative, lower case, with a conventional prefix. Use the body to say _why_. No em
dashes anywhere.

## What CI checks

`.github/workflows/ci.yml` runs on every pull request. It declares `permissions: contents: read` and
consumes no secrets, so a pull request from a fork gets the same run as one from a branch here.

| Job | What it asserts |
| --- | --- |
| **checks** | the generated `THIRD_PARTY_NOTICES.md` matches `Cargo.lock`; the design-system guard; the tool tests |
| **rust** | `cargo fmt --check`, then `cargo clippy -D warnings` and `cargo test` of our crates (`redrob-canvas`, `redrob-ai`, `redrob-agent`) with `--profile ci`; skipped when no Rust input changed or the pull request targets `main` |

The distribution job is an attribution gate: every archive ships `LICENSE-MIT`, `LICENSE-APACHE`,
`COPYRIGHT`, `NOTICE` and `THIRD_PARTY_NOTICES.md`, so a stale notice file is a defect in what we
promise recipients. The generator reads Cargo metadata with `--locked --offline` on purpose, so run
`cargo fetch --locked` first on a cold cache.

## Dependency updates

Dependabot **version updates are off**: they produced a standing queue of pull requests, each
needing its own CI run, and on this repository the churn cost more than it caught. Two things
replace them:

- **Dependabot security updates are on.** A dependency with a known advisory still gets a pull
  request opened automatically.
- **The advisory job gates every pull request.** `cargo audit` runs against the committed
  lockfile, so a vulnerable dependency cannot merge even if nobody read an alert.

Routine bumps are deliberate: bump what you need for the change you are making, in the
same pull request, and say why in the body.

## License of your contribution

By contributing you agree your work is released under MIT OR Apache-2.0, the licence in
`LICENSE-MIT` and `LICENSE-APACHE`. There is no CLA and no sign-off requirement. Never replace an
original author's copyright notice; ours goes beside theirs.
