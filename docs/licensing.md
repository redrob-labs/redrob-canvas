# Licensing

Redrob Canvas is distributed under **MIT OR Apache-2.0**, at the recipient's option
(`LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`). Every input is under the same or a more permissive
licence, so nothing in a release binds a distributor to anything beyond attribution.

Until 0.6.x the project was GPL-3.0-or-later, because it translated Krita code and copied Graphite
and GEGL material. All of that was removed in commit `d4854f2` together with the Qt Quick shell, and
the relicensing in 0.7.0 rests on that removal: no GPL, LGPL or otherwise copyleft source remains in
the tree. Do not vendor or translate copyleft source into it.

## Where the editors came from

The three editors and their shared libraries under `editors/` started as four projects of the
ArtCraft Team, each MIT OR Apache-2.0, Copyright (c) 2026 the ArtCraft Team and their contributors.
They are no longer synced with those projects. Their `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE`
files stay intact under `editors/<name>/`; their NOTICE texts are reproduced verbatim in `NOTICE`;
`COPYRIGHT` names them beside us; `SOURCE_ORIGINS.md` records each origin and commit.

Two rules from the organisation's [FORKS.md](https://github.com/redrob-labs/.github/blob/main/FORKS.md):

- **Never replace an original author's copyright notice.** Ours goes beside theirs.
- **Never use what the origin projects license separately.** Their names, wordmarks and logos are
  trademarks of the ArtCraft Team, not open source; none of them is in the tree or in a release.
  Redrob Canvas's mark comes from the Redrob design system (`DESIGN_SYSTEM_PIN.json`).

## Three notice files, and why

- `THIRD_PARTY_NOTICES.md` is **generated** by `tools/generate_distribution_artifacts.py` from the
  exact `Cargo.lock` and `cargo metadata --locked --offline`, carries a do-not-edit header, and is
  byte-checked in CI. It inventories **every Rust package the build resolves** with its registry
  checksum, licence expression and the licence/notice texts inside its source. Run
  `cargo fetch --locked` first on a cold machine; the generator never touches the network.
- `SOURCE_ORIGINS.md` is maintained by hand and records **where the editor source came from**, one
  section per editor with its origin commit. It cannot live in the generated file.
- `NOTICE` is the Apache-2.0 section 4(d) file: our line, then each origin project's NOTICE verbatim.

Other licensing facts worth knowing:

- Fonts, icons and dictionaries each editor ships (Inter, JetBrains Mono, Source Sans 3, Lucide,
  SCOWL, hyphenation patterns, Adobe CMaps, the FOGRA39 data, the Foxit standard-14 substitutes) are
  listed with their licences in the origin NOTICE texts reproduced in `NOTICE`, with per-file
  hashes in each editor's `ATTRIBUTION.md` or `ASSETS.md`.
- `editors/shared/vendor/` holds patched copies of `krilla` and `hayro` (MIT OR Apache-2.0) that
  the root `Cargo.toml` patches in by path; their own licence files are retained there.
- The Redrob Code engine is not bundled. The agent speaks to the one shared daemon on the machine
  through `redrob-engine-client`, pinned by tag in `Cargo.toml`.
- The Redrob design-system tokens and icons under `third_party/redrob-ui/` are vendored from
  `@redrob-labs/ui` and pinned by SHA-256 in `DESIGN_SYSTEM_PIN.json`.
- New source files use `SPDX-License-Identifier: MIT OR Apache-2.0`. Files from the origin
  projects keep the headers they came with.
