<!-- Keep the title imperative and under 70 characters. -->

## What this changes

<!-- The behaviour that is different, not a restatement of the diff. -->

## Why

<!-- The problem. If it is a bug, say how it reproduced. -->

## How it was verified

<!-- The commands you actually ran, and what they printed. -->

```
scripts/verify-upstream.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"
```

## Checklist

- [ ] Nothing under `crafts/` was edited by hand. An editor change went upstream and
      arrives as a pin bump (`scripts/fetch-upstream.sh`), merged as a merge commit.
- [ ] Upstream commits changed only in `upstream-base.json`, `docs/upstream-sources.toml`
      and `UPSTREAM_NOTICES.md`, together, and `scripts/verify-upstream.sh` still passes.
- [ ] If `Cargo.lock` changed, `cargo fetch --locked` then
      `python3 tools/generate_distribution_artifacts.py --generate` was run and
      `THIRD_PARTY_NOTICES.md` is committed.
- [ ] No upstream copyright notice was replaced; `NOTICE` still reproduces each
      upstream NOTICE verbatim. Nothing references `crafts/*/docs/brand/`.
- [ ] No hex colour literal in `apps/redrob-canvas/src` outside a marked document colour.
