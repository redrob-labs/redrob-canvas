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
`tools/merge_workspaces.py` generates the root `Cargo.toml` from the four. Before it reads them it
runs three idempotent scripts, so the tree a pull brings back is turned into ours again:

| Script | What it does to the subtree |
|---|---|
| `tools/prune_upstream.py` | deletes the upstream repository furniture (`docs/`, `book/`, `packaging/`, `.github/`, READMEs, `examples/`, `perf/`, `scorecard/`, `scripts/`, `xtask/`, `Dockerfile`); keeps `LICENSE-*`, `NOTICE`, `ATTRIBUTION.md`, `assets/`, `vendor/`, `licenses/`, `contributors/` |
| `tools/rename_crates.py` | `photocraft-*` to `redrob-photo-*`, `vectorcraft-*` to `redrob-vector-*`, `designcraft-*` to `redrob-layout-*`, `craft-*` to `redrob-craft-*`, in every manifest, `use` path and the lock |
| `tools/rename_identifiers.py` | file extensions (`.pcraft` to `.rphoto`, `.vectorcraft`/`.drawcraft` to `.rvector`, `.designcraft` to `.rlayout`), URI schemes (`redrob-photo://` ...), MIME types (`application/x-redrob-photo` ...) and env prefixes (`REDROB_PHOTO_*`, `REDROB_VECTOR_*`, `REDROB_LAYOUT_*`, `REDROB_CRAFT_*`) |

`tools/rebrand_crafts.py` then rebrands the user-visible strings, and `scripts/check_upstream_ids.py`
(the CI `checks` job) fails while any upstream identifier is left outside the attribution files
and the `// legacy:` read paths. Only `crates/*` of each editor are workspace members; their `apps/`
and web shells are excluded, because the one shell here is `apps/redrob-canvas`.

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

What a user can meet carries the Redrob name (crates, extensions, URI schemes, MIME types, env
variables; the table above), and the renames are scripts so a sync re-applies them rather than
hand-resolving a conflict in every manifest. What stays upstream's: the `[patch]` entries that point
`krilla` and `hayro` at `crafts/craft-libs/vendor/`, the engines' internal format ids and the
`"format"` value in the document headers (`"vectorcraft"`, `"designcraft"`), so a file written here
still opens in an upstream build and vice versa.

### Legacy read paths

Old documents still open: `.pcraft` (by name and by content), `.vectorcraft` and `.drawcraft`,
`.designcraft`, through the shell's routing (`apps/redrob-canvas/src/routing.rs`) and Open filters
and each editor's own open path, recovery scan and template list. The MCP servers answer the old
URI schemes (`photocraft://`, `vectorcraft://`, `designcraft://`) beside the new ones, and the
user-facing environment variables are read under the new name first, then the upstream one.
Writers emit only the Redrob values. Every such line is marked `// legacy:`; the marker is what
`tools/rename_identifiers.py` skips and `scripts/check_upstream_ids.py` allows.

## Taking upstream work

```bash
scripts/fetch-upstream.sh photocraft <sha>     # git subtree pull --squash, then merge_workspaces.py
scripts/fetch-upstream.sh                      # every subtree, to the commit upstream-base.json records
```

`fetch-upstream.sh` is the only thing that should run `git subtree pull` here; by hand the commands are

```bash
git subtree pull --prefix crafts/<name> https://github.com/storytold/<name>.git <sha> --squash
python3 tools/merge_workspaces.py        # parks the manifest, prunes, renames, regenerates Cargo.toml
python3 tools/rebrand_crafts.py
cargo update --workspace --offline       # the lock follows the renamed crates
python3 scripts/check_upstream_ids.py
```

Expect conflicts in files the rename scripts rewrote (manifests, `use` paths, extension lists):
take upstream's side, then re-run the scripts; they are idempotent. `--with-trees` compares the
parked `Cargo.upstream.toml` with upstream's `Cargo.toml` byte for byte, which the crate rename
makes false by design; run it only before `merge_workspaces.py` on a fresh pull.

After taking anything:

1. Set that entry's `base.commit` and `lastSyncedUpstream` in `upstream-base.json`, the `commit` in
   `docs/upstream-sources.toml`, and the pin in its `UPSTREAM_NOTICES.md` section.
2. Run `scripts/verify-upstream.sh`, `scripts/check_upstream_ids.py`, `cargo fmt --all -- --check`,
   then clippy and the tests of our crates with `--profile ci` (the `rust` job in `ci.yml`).
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
