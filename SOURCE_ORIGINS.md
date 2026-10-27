# Source origins

Where the editor code in this repository came from. It is maintained by hand. `THIRD_PARTY_NOTICES.md`
is the generated inventory of the packages the build resolves; this file is about source that was
copied into the tree.

The three editors and their shared libraries started as four projects of the ArtCraft Team,
each MIT OR Apache-2.0, the same terms as Redrob Canvas. They were taken at the commits below and are
**no longer synced**: Redrob Canvas develops them here as its own code. Each one's `LICENSE-MIT`,
`LICENSE-APACHE` and `NOTICE` stay intact in its folder, and the copyright lines are never replaced:
ours sit beside theirs in `NOTICE` and `COPYRIGHT`.

The Krita, GIMP, GEGL and Graphite material that earlier versions carried or translated was removed
in commit `d4854f2` together with the Qt shell; none of it remains, which is what made the
relicensing from GPL-3.0-or-later possible.

## editors/photo

Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors. MIT OR Apache-2.0.

From <https://github.com/storytold/photocraft> at `ad96df566b587ebc056644c9e2bf621ddba39a92`. The
raster editor: layers, masks, adjustment layers, brushes, selections, colour management, PSD/PSB
and ORA/KRA codecs, RAW decoding.

## editors/vector

Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

From <https://github.com/storytold/vectorcraft> at `b12a52f7249e0aa73f030587e05ed9ce0c73ab67`. The
vector editor: paths, booleans, text on paths, gradients, SVG/AI/EPS/PDF/DXF import and export,
WebAssembly plug-ins.

## editors/layout

Copyright (c) 2026 ArtCraft Team and the DesignCraft contributors. MIT OR Apache-2.0.

From <https://github.com/storytold/designcraft> at `a930276fd7462f3d9abd2fbb38544c94c0c911c8`. The
page-layout editor: master pages, text frames and threading, hyphenation, IDML/PDF/PPTX/EPUB.

## editors/shared

Copyright (c) 2026 ArtCraft Team and the craft-libs contributors. MIT OR Apache-2.0.

From <https://github.com/storytold/craft-libs> at `8f5a2831614e632f91ad367f7af2a6e5b61b9529`. The
libraries the three editors share: docking and panel UI, colour management, printing, and the
patched `krilla` / `hayro` PDF writer and reader under `vendor/`, which the root `Cargo.toml` patches
in by path. Its NOTICE records the PdfCraft, FilmCraft and EffectCraft revisions parts of it were
extracted from.

## Names and brand assets

The origin projects' names, wordmarks and logos are trademarks of the ArtCraft Team and are not
covered by MIT or Apache-2.0. Redrob Canvas does not display, install or package any of them; the
code was renamed to Photo, Vector and Layout, and the brand assets were removed. Redrob Canvas's own
mark comes from the Redrob design system (`DESIGN_SYSTEM_PIN.json`). A few identifiers that saved
files or automation scripts carry (old file extensions, the `"format"` value inside documents,
droplet and print-option keys) keep their original spelling so existing files keep opening; each is
marked `legacy` in the code.
