# Upstream

Redrob Canvas hosts four editors from [storytold](https://github.com/storytold), the ArtCraft
family, each MIT OR Apache-2.0 and each vendored as a git subtree under `crafts/`:

| Subtree | Upstream | Copyright |
|---|---|---|
| `crafts/photocraft` | [storytold/photocraft](https://github.com/storytold/photocraft) | ArtCraft Team and the PhotoCraft contributors |
| `crafts/vectorcraft` | [storytold/vectorcraft](https://github.com/storytold/vectorcraft) | ArtCraft Team and the VectorCraft contributors |
| `crafts/designcraft` | [storytold/designcraft](https://github.com/storytold/designcraft) | ArtCraft Team and the DesignCraft contributors |
| `crafts/craft-libs` | [storytold/craft-libs](https://github.com/storytold/craft-libs) | ArtCraft Team and the craft-libs contributors |

This file records where each came from, how upstream work is taken, and what keeps the attribution
from being deleted by accident. The pins themselves are in `upstream-base.json` (machine-readable)
and `docs/upstream-sources.toml` (the list `scripts/verify-upstream.sh` validates), and the two
must agree.

## What kind of fork this is

Not a fork of any one project. This repository's own history begins as a GPL-3.0 Qt Quick editor
that ported behaviour from Krita and GIMP; that shell and every file derived from Krita, GIMP, GEGL
or Graphite were removed in `d4854f2`. The four ArtCraft trees arrived afterwards, one
`git subtree add --squash` each, so every subtree has a real merge base: its squash commit
(`Squashed 'crafts/<name>/' content from commit <sha>`) names the exact upstream commit, and
`git subtree pull --squash` performs an ordinary three-way merge against it. Upstream work is
therefore taken by merge, not file by file.

Each upstream's own `Cargo.toml` is parked beside it as `crafts/<name>/Cargo.upstream.toml`, and
`tools/merge_workspaces.py` generates the root `Cargo.toml` from the four and `tools/rebrand_crafts.py` rebrands their user-visible strings. Only `crates/*` of each
editor are workspace members; their `apps/`, `xtask` and web shells are excluded, because the one
shell here is `apps/redrob-canvas`.

## The measured base

| Subtree | Commit | Import commit |
|---|---|---|
| `crafts/photocraft` | `ad96df566b587ebc056644c9e2bf621ddba39a92` | `b217dea7b` |
| `crafts/vectorcraft` | `b12a52f7249e0aa73f030587e05ed9ce0c73ab67` | `c751d3287` |
| `crafts/designcraft` | `a930276fd7462f3d9abd2fbb38544c94c0c911c8` | `b37bb0306` |
| `crafts/craft-libs` | `8f5a2831614e632f91ad367f7af2a6e5b61b9529` | `c4d3b270b` |

Recorded, not remembered: the commit is the one the subtree squash commit names, and
`scripts/verify-upstream.sh` reads that commit message out of `git log` and refuses a pin that
differs from it. It also asks GitHub whether each commit still exists, so a force-pushed or
garbage-collected upstream commit is caught before it becomes a pin nobody can reproduce.

Traces of the derivation are load-bearing and are deliberately left alone: crate names
(`photocraft-*`, `vectorcraft-*`, `designcraft-*`, `craft-*`), the `.pcraft` document format, and
the `[patch]` entries that point `krilla` and `hayro` at `crafts/craft-libs/vendor/`. Renaming a
crate is how a merge conflict on every subsequent sync is manufactured.

## Taking upstream work

```bash
scripts/fetch-upstream.sh photocraft <sha>     # git subtree pull --squash, then merge_workspaces.py
scripts/fetch-upstream.sh                      # every subtree, to the commit upstream-base.json records
```

`fetch-upstream.sh` is the only thing that should run `git subtree pull` here; by hand the command is

```bash
git subtree pull --prefix crafts/<name> https://github.com/storytold/<name>.git <sha> --squash
python3 tools/merge_workspaces.py
```

After taking anything:

1. Set that entry's `base.commit` and `lastSyncedUpstream` in `upstream-base.json`, the `commit` in
   `docs/upstream-sources.toml`, and the pin in its `UPSTREAM_NOTICES.md` section.
2. Run `scripts/verify-upstream.sh`, `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
3. If `Cargo.lock` moved, `cargo fetch --locked` then
   `python3 tools/generate_distribution_artifacts.py --generate` and commit `THIRD_PARTY_NOTICES.md`.

Use a `sync/upstream-<name>-<sha7>` branch and merge it as a **merge commit**, never a squash, so
the subtree's merge base survives and the next pull does not replay work already taken. Conflicts
cluster where Redrob Canvas has edited an upstream crate; keep ours, take their surrounding change,
and never resolve a conflict by renaming an identifier the other side still uses.

## What the boundary guard enforces

`scripts/verify-upstream.sh` fails when:

1. `upstream-base.json` or `docs/upstream-sources.toml` is missing or unparseable, or a `code`
   entry in one is absent from the other, or the two record different commits for one subtree.
2. A `code` entry's commit is not a full 40-hex sha, its repository is not `https`, or it declares
   no `license`, no `boundary` or no `prefix`.
3. A `code` entry's licence is anything other than MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 or
   Unicode: Redrob Canvas is MIT OR Apache-2.0, and a copyleft input would make `LICENSE-MIT` false.
4. `crafts/<prefix>` is not in the tree, or the newest `Squashed '<prefix>/' content from commit`
   message in `git log` does not name the pinned commit.
5. `UPSTREAM_NOTICES.md` has no `## <name>` section for a `code` entry, or that file does not
   contain the pinned commit.
6. A file named in `attribution` lacks one of its `mustContain` strings. **Never replace an upstream
   copyright notice**; ours goes beside theirs.
7. Anything outside the subtree references a path under `excludedPrefixes` (the ArtCraft brand
   assets, which are trademarks, not open source).
8. A pinned commit no longer exists on GitHub, or `redrob_code`'s tag has moved off its commit, or
   that tag is not the one the root `Cargo.toml` names for `redrob-engine-client`.

Pass `--with-trees` to additionally check that `crafts/<name>/Cargo.upstream.toml` is byte-identical
to the pinned upstream's `Cargo.toml` (one `git fetch` per subtree; off by default so CI stays fast).
