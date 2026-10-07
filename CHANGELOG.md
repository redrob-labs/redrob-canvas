# Changelog

The release workflow's notes point here, so this file is what a download's notes actually say.
Versions follow the `vMAJOR.MINOR.PATCH` tags that trigger a release; while the major is 0 a
minor bump is where behaviour may change.

## 0.5.1 — 2026-10-08

A patch on the version number, but it adds two things: an AI tab and a chat-style Agent tab. It
also fixes the Windows build that kept 0.5.0 from shipping there.

### Added

- **AI tab (IOPaint).** IOPaint 1.6.0 (Apache-2.0) runs on this computer. "Install AI engine"
  puts it and PyTorch in the app's own folder (about 1.5 GB; needs uv or Python 3.11). Then:
  - erase or replace what is selected, with IOPaint's erase models (LaMa, MI-GAN, MAT, FcF, ZITS,
    LDM, Manga, OpenCV) or its diffusion models (Stable Diffusion 1.5/2, SDXL, PowerPaint,
    Kandinsky, AnyText, Paint by Example, InstructPix2Pix), with prompt, steps, strength,
    guidance, seed, sampler, ControlNet, BrushNet and LCM LoRA;
  - expand the canvas and fill the new edges;
  - select a subject by clicking it (Segment Anything), or in one click (background removal or
    anime segmentation);
  - remove the background, upscale with RealESRGAN (×2 to ×4), restore faces (GFPGAN,
    RestoreFormer), and erase a whole folder in a batch.
  Every result is a new layer (or the selection), so the original stays and Undo takes it back.
  Models download the first time they are used.
- **Agent tab is a chat.** Type and press Enter. With redrob-code connected, each message
  continues the same conversation, so it remembers what came before; "New chat" starts over.
  Without it, the Redrob agent answers in the same thread. Edits still wait for Apply.

### Fixed

- **Windows:** the token file's owner-only check now reads the Windows access list itself; Qt's
  permission bits always reported "everyone", so the start-up check failed and the Windows build
  of 0.5.0 never shipped.
- **macOS:** the app reported version 0.3 in Finder and crash reports; it now takes the version
  from the build.

### Notes

- The AI engine listens on 127.0.0.1 only and has no password of its own; any program on this
  computer can use it while it runs. It stops with the app.

## 0.5.0 — 2026-10-07

A minor bump: the editor gains the Photoshop-style working surface -- tool groups, menus, keys,
layer operations and dialogs -- over the engine that 0.3.0 and 0.4.0 filled in. 249 commits.

### Added

- **Layers:** duplicate (Ctrl+J), merge down (Ctrl+E), merge visible (Ctrl+Shift+E), flatten,
  select several layers to group, delete and align, clipping masks (Ctrl+Alt+G), locks
  (transparent pixels, image, position, all), linked layers, blend-if ranges, artboards with a
  draggable handle and PNG export per artboard, smart objects that re-render transforms and warps
  from the source, and smart filters stacked on a smart object.
- **Edit:** new document (Ctrl+N), copy, cut and paste through the system clipboard, stroke
  selection, content-aware fill (Shift+F5), puppet warp, crop to selection, clear outside the
  selection, and editing an adjustment layer's filter in place.
- **Image and view:** image size and canvas size dialogs, colour mode and precision menus, navigator,
  rotate view (R), rulers and guides (Ctrl+R, Ctrl+;), snapping to guides and canvas edges.
- **Brushes:** paint shows while the pointer is down, flow with Shift+digit keys, dab angle and
  angle from pen tilt, a mixer brush with Photoshop's drying, tip pickup, Sample All Layers and
  Load/Clean.
- **Text:** installed outline fonts, stored by name as Photoshop does, with kerning, OpenType
  shaping (ligatures, Hangul, Arabic), paragraph width and alignment.
- **Colour:** CMYK soft proof through Little CMS (Ctrl+Y), an Image > Mode > CMYK document mode,
  and export to CMYK TIFF and layered CMYK PSD through the proof profile.
- **Files:** open PSD, KRA, XCF and TIFF; PSD export keeps clipping and blend modes; swatches
  persist and load `.gpl` / `.aco`.
- **Selection:** color range by sampled colour or tone range.
- **Actions:** record edits, save them and play them back as one step.
- **Agent:** a loopback MCP endpoint for `redrob-code`, running a multi-step task against the
  canvas with every edit held for approval, and connecting to the Redrob console with a device code.
- **Keys:** Photoshop's shortcut layout, Alt-click for the clone source, Ctrl held for Move.

### Changed

- The window, menus, layer panel, filter browser and option tabs are split out of `Main.qml`.
- File and colour pickers open inside the window, so keys typed in them no longer reach the canvas,
  and every save picker asks before replacing an existing file.
- Slow frames render on a worker so painting does not stall under heavy adjustment layers.

### Fixed

- 16- and 32-bit documents: resize, crop and the 8-bit tools no longer corrupt deep layers, and
  faint paint keeps the deep detail beneath it.

## 0.4.0 — 2026-10-03

A minor bump rather than a patch: the pen overlay now shows something it never showed, and the
document projection gained fields for it.

### Added

- **A committed pen path keeps its bezier handles on canvas.** Committing a path used to clear the
  pen's tool state, and the overlay had nothing else to read, so the handles that shaped the curve
  disappeared at the moment the curve existed. The overlay now reads the active vector node's own
  geometry back out of the document — which also means an undo removes the handles with the node
  instead of leaving a ghost frame over an empty canvas.
- Each anchor's outgoing control point is drawn as a leash and a round knob, distinct from the
  square anchor handles. A control point sitting on its own anchor is a corner and draws nothing.

### Changed

- The engine provenance pin moves to `redrob-code` **v0.4.1** (`cc0c4bea`), from v0.1.0.

### Fixed

- **The pin guard never checked that a recorded tag points at the recorded commit.** The pins file
  says the tag is the durable half of a pin, and that half was the unchecked one: a tag is a movable
  ref, so it can be repointed and the two halves then disagree while each one, read alone, still
  resolves. `scripts/verify-upstream.sh` now asserts the pairing, dereferencing annotated tags, and
  reports a moved tag and a deleted tag differently.

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
