# Licensing

redrob-graphics is distributed under GPL-3.0-or-later so GPL-compatible Krita and GIMP components can be integrated deliberately.

- Krita carries file-level GPL/LGPL identifiers; audit every imported or linked component.
- GIMP application core is GPL-3.0-or-later. Its public libraries are generally LGPL, and plugins communicating through PDB are described by GIMP as separate aggregation.
- GEGL, babl, LCMS, Qt, codecs, fonts, icons, brushes, and test assets retain their own licenses. GEGL is an optional system native C dependency discovered through pkg-config, not a Cargo crate; the default OFF build does not link it.
- Krita is an optional exact-source-boundary scaffold. The default build does not link Krita, and enabling the scaffold does not by itself create a distributed Krita binary dependency or supported operation.
- The deterministic semantic text path pins the Rust crate `font8x8` exactly at `0.3.1` (MIT) and uses only its compiled-in `BASIC_FONTS` bitmap. The upstream bitmap font data is identified as public domain. `font_family` is metadata and is never used for OS lookup. See `THIRD_PARTY_NOTICES.md` for provenance links and notice details.
- The bounded core format adapters and every transitive Rust package are inventoried from the exact `Cargo.lock`; versions, registry checksums, license expressions, and byte-exact local license/notice texts are recorded in the generated `THIRD_PARTY_NOTICES.md`.
- Upstream repositories are references and are not vendored by default. Pins in `upstream-sources.toml` are not claims of ownership.
- New source files use `SPDX-License-Identifier: GPL-3.0-or-later`.

`tools/generate_distribution_artifacts.py` generates and checks the complete notices and `SOURCE_OFFER.md` without network access. The Qt CMake configure, `redrob_distribution_check`, and `redrob_source_bundle` targets require those artifacts to match `Cargo.lock` and the selected adapters. The deterministic source archive includes checksum-verified local copies of every locked registry source and an offline Cargo source-replacement configuration. Direct installation reruns the artifact check, byte-checks the complete source bundle, and installs all compliance artifacts with the executable.