<!-- Keep the title imperative and under 70 characters. -->

## What this changes

<!-- The behaviour that is different, not a restatement of the diff. -->

## Why

<!-- The problem. If it is a bug, say how it reproduced. -->

## How it was verified

<!-- The commands you actually ran, and what they printed. -->

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"
```

## Checklist

- [ ] If `Cargo.lock` changed, `cargo fetch --locked` then
      `python3 tools/generate_distribution_artifacts.py --generate` was run and
      `THIRD_PARTY_NOTICES.md` is committed.
- [ ] No original author's copyright notice was replaced; `NOTICE` still reproduces each
      origin project's NOTICE verbatim (`SOURCE_ORIGINS.md`).
- [ ] No hex colour literal in `apps/redrob-canvas/src` outside a marked document colour.
