<!-- Keep the title imperative and under 70 characters. -->

## What this changes

<!-- The behaviour that is different, not a restatement of the diff. -->

## Why

<!-- The problem. If it is a bug, say how it reproduced. -->

## How it was verified

<!-- The commands you actually ran, and what they printed. -->

```
./scripts/verify-upstream.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cmake -S native -B build/qt && cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
```

## Checklist

- [ ] No upstream tree is committed. `upstream/` is ignored and
      `scripts/verify-upstream.sh` still passes.
- [ ] Upstream commit pins changed only in `docs/upstream-sources.toml`; no hash
      is duplicated elsewhere.
- [ ] If a dependency or licence-relevant file changed,
      `python3 tools/generate_distribution_artifacts.py --check` was rerun and
      `THIRD_PARTY_NOTICES.md` / `SOURCE_OFFER.md` are current.
- [ ] GPL obligations intact: `LICENSE` is still the verbatim GPL-3.0 text and
      the install still ships the notices and corresponding source.
- [ ] No GIMP/Krita/GEGL name reaches a user-visible string except where it
      legitimately identifies upstream.
