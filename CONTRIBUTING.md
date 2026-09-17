# Contributing

Thanks for helping. This is the working agreement for the repository: how branches are named, what
has to be green before a merge, and the one thing that makes this repository unusual.

## What this is, and is not

Redrob Graphics is **a new product architecture, not a UI fork.** Krita and GIMP stay pinned as
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

One long-lived branch, `main`. Short-lived branches off it, merged by pull request.

- **`main`** is the trunk: no direct pushes, no force pushes, no deletion.
- **Working branches** are `<type>/<short-slug>`, e.g. `fix/selection-feather`,
  `feat/agent-tool-loop`. Types: `feat`, `fix`, `chore`, `docs`, `test`, `refactor`, `perf`.
- Release tags are cut from `main`, formatted `v<major>.<minor>.<patch>`.

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

**`.github/workflows/verify-upstream.yml`** asserts the pinned upstream references are intact, which
is what keeps "follows Krita" a checkable statement rather than a claim.

Run `cargo test --workspace` before pushing.
