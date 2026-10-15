# Licensing

Redrob Canvas is distributed under **MIT OR Apache-2.0**, at the recipient's option
(`LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`). Every input is under the same or a more permissive
licence, so nothing in a release binds a distributor to anything beyond attribution.

Until 0.6.x the project was GPL-3.0-or-later, because it translated Krita code and copied Graphite
and GEGL material. All of that was removed in commit `d4854f2` together with the Qt Quick shell, and
the relicensing in 0.7.0 rests on that removal: no GPL, LGPL or otherwise copyleft source remains in
the tree, and `scripts/verify-upstream.sh` refuses any `code` pin whose licence is not permissive so
it cannot come back unnoticed.

## Vendored source versus spoken protocol

Every upstream in `docs/upstream-sources.toml` declares a `kind`:

| kind | What happens | Does upstream's licence bind our distribution? |
|---|---|---|
| `code` | Source is vendored into this repository as a git subtree under `crafts/` | Yes |
| `protocol` | A wire format we speak (the Redrob Code daemon API) | No |

The four `code` sources are PhotoCraft, VectorCraft, DesignCraft and craft-libs from
[storytold](https://github.com/storytold), each MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team
and their contributors. Their `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE` files stay intact under
`crafts/<name>/`; their NOTICE texts are reproduced verbatim in `NOTICE`; and `COPYRIGHT` names them
beside us. The pins live in `upstream-base.json` and `docs/upstream-sources.toml`, which the script
checks against each other and against the subtree squash commits in `git log`. See `UPSTREAM.md`.

Two rules from the organisation's [FORKS.md](https://github.com/redrob-labs/.github/blob/main/FORKS.md)
are enforced here rather than described:

- **Never replace an upstream copyright notice.** Ours goes beside theirs. `upstream-base.json`'s
  `attribution` list says which strings `NOTICE`, `COPYRIGHT`, `LICENSE-MIT` and `LICENSE-APACHE`
  must keep, and the script fails when one goes missing.
- **Never use anything under a prefix upstream licenses separately.** The ArtCraft name, wordmark and
  logos under `crafts/*/docs/brand/` are trademarks of the ArtCraft Team, not open source. They stay
  in the tree only because the subtree carries them; nothing outside `crafts/` may reference them,
  and the script greps for that. Redrob Canvas's mark comes from the Redrob design system
  (`DESIGN_SYSTEM_PIN.json`).

## Three notice files, and why

- `THIRD_PARTY_NOTICES.md` is **generated** by `tools/generate_distribution_artifacts.py` from the
  exact `Cargo.lock` and `cargo metadata --locked --offline`, carries a do-not-edit header, and is
  byte-checked in CI. It inventories **every Rust package the build resolves** with its registry
  checksum, licence expression and the licence/notice texts inside its source. Run
  `cargo fetch --locked` first on a cold machine; the generator never touches the network.
- `UPSTREAM_NOTICES.md` is maintained by hand and records **source vendored into this repository**,
  one section per subtree with its pinned commit and the update recipe. It cannot live in the
  generated file.
- `NOTICE` is the Apache-2.0 section 4(d) file: our line, then each upstream NOTICE verbatim.

Other licensing facts worth knowing:

- Fonts, icons and dictionaries each editor ships (Inter, JetBrains Mono, Source Sans 3, Lucide,
  SCOWL, hyphenation patterns, Adobe CMaps, the FOGRA39 data, the Foxit standard-14 substitutes) are
  listed with their licences in the upstream NOTICE texts reproduced in `NOTICE`, with per-file
  hashes in each editor's `ATTRIBUTION.md` or `ASSETS.md`.
- `crafts/craft-libs/vendor/` holds patched copies of `krilla` and `hayro` (MIT OR Apache-2.0) that
  the root `Cargo.toml` patches in by path; their upstream licence files are retained there.
- The Redrob Code engine is not bundled. The agent speaks to the one shared daemon on the machine
  through `redrob-engine-client`, pinned by tag in `Cargo.toml`.
- The Redrob design-system tokens and icons under `third_party/redrob-ui/` are vendored from
  `@redrob-labs/ui` and pinned by SHA-256 in `DESIGN_SYSTEM_PIN.json`.
- New source files use `SPDX-License-Identifier: MIT OR Apache-2.0`. Files under `crafts/` keep
  whatever header upstream gave them.
