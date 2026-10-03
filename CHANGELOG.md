# Changelog

The release workflow's notes point here, so this file is what a download's notes actually say.
Versions follow the `vMAJOR.MINOR.PATCH` tags that trigger a release; while the major is 0 a
minor bump is where behaviour may change.

## 0.3.0 — 2026-10-03

Re-derives the GIMP and Krita feature set this product was missing, as Rust and QML written
from the upstream behaviour rather than copied from it. Groups A through I of the porting plan,
90 commits.

### Added

- **Blend modes: 9 → 41.** The GIMP/Krita union, including the dodge/burn/light family, the
  HSV and LCh component modes, and the arithmetic set.
- **Filters: 12 → 71.** Blurs (motion, lens, box), edge detect, distorts (displace, fractal
  trace, warp map, bump map), noise, tone and colour operations. The map-reading filters take a
  layer to read the map FROM, not just the layer's own pixels.
- **Rail tools: 11 → 24.** Lasso, polygon and colour selection, intelligent scissors,
  foreground select, pen, measure, align, perspective, cage, warp/liquify, n-point deformation,
  enclose-and-fill, lazybrush.
- **File formats: ~6 → 23.** PSD (all colour modes and bit depths, layer masks), XCF (v11, read
  and write, indexed and greyscale, masks), KRA (Krita's native tiled layers, both directions),
  DDS (BC1/BC3 encoder), animated WebP, JPEG-XL, PDF first-page image, camera RAW, ICC profiles,
  AVIF/HEIF container classification.
- **Brush engine.** Dodge/burn, ink, MyPaint-style scatter, GIH image pipes, Krita sensor
  bindings for size AND, new in this release, for opacity and flow as separate channels.
- **Pen handles.** Anchors placed by click stay corners; anchors placed by drag pull a bezier
  handle, whose incoming control is mirrored so the curve passes smoothly through the point.
- **Draggable transform handles** on the canvas for perspective, cage and n-point, drawn as an
  overlay that stays the same size to aim at under any zoom.
- **Fourteen tool glyphs**, drawn here in the design system's geometry, for the tools redrob-ui
  has no icon for. Before this, five different tools drew the same square.
- **Linear-light compositing** in the op graph: a part-strength effect blends in linear light,
  not over display-encoded bytes.

### Fixed

Everything in this section was found by compiling and running the work, which groups A–H
deliberately deferred to the end.

- A densely-sampled stroke painted a single dab. The dab-spacing walker was rebuilt per stroke
  segment, discarding the distance it had walked, so a stroke whose samples sit closer together
  than the dab spacing never reached a second dab — exactly what a graphics tablet produces.
- Every Krita file was rejected before its importer saw it, including files this product had
  just written: KRA borrowed ORA's archive reader, which also enforced ORA's own file list.
- EXR export could not work at all: a float-only encoder was being handed an 8-bit buffer.
- "Press Enter to close the shape" did nothing, in seven tools. A QML array mutated in place
  emits no change signal, so the shortcut's enabled binding was frozen at the empty array.
- The wide-gamut colour swatch was unbound, resolving its conversion through the wrong parent.
- Encoder failures were all reported as `OutputTooLarge`, which hid the EXR bug for a whole
  porting group.

### Changed

- `SizeDynamic` is now `BrushDynamic` and `SizeSensor` is `DynamicSensor`, since the same
  binding drives three channels. The old names remain as aliases.
- `BrushSettings` is no longer `Copy` — it owns binding lists. Everything that reads settings
  takes a reference, so the paint path is unaffected.
- DDS export is reported as lossy, because block compression is.

### Known gaps

Written down rather than left to be discovered:

- HEIF is container-only. There is no pure-Rust HEVC decoder; `rav1d` covers AV1, so AVIF is
  classified separately and HEIF reports its dimensions without decoding.
- A DDS this product writes for a canvas whose width or height is not a multiple of 4 is valid
  and readable by other tools, but not by this product: the `image` crate's DXT decoder refuses
  such dimensions.
- A committed pen handle is not drawn until its path is re-opened; the handle overlay draws one
  outline through its points and does not yet understand anchor/handle pairs.
- ICC table-based profiles are refused, matrix-shaper profiles are applied. A file with a table
  profile still opens, untagged.

## 0.2.0 — 2026-10-01

Packaging and distribution hardening on top of 0.1.0: the corresponding-source bundle and
third-party notices are pinned by hash and gated in the build, text is read and written as UTF-8
regardless of host locale, line endings are not converted, and the system libraries the Rust
staticlib needs on Windows are named explicitly.

## 0.1.0 — 2026-09-17

First tagged build. Draft release.
