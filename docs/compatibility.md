# Compatibility matrix

Status: `planned`, `native`, `adapter`, `parity`.

**`parity` is not a judgement, it is a citation.** A row may claim it only by naming, in its Evidence column, a
harness that regenerates the authority's own output or source and compares this product against it. The named
harness must exist; `scripts/verify-upstream.sh` checks both directions, so a `parity` row with no evidence and
an evidence key naming no harness are each an error. `native` means only that an implementation is here.

**`Works` is a separate question: can a user reach it and does it do the job?** `native` shipped on rows no button
reaches (see `docs/user-paths.md`). `Works: yes` requires an `e2e:<path>::<name>` citation in Evidence naming a
test that drives the feature the way a user does; the guard checks the file exists and contains that name, and
refuses `yes` on a `planned` row. Every row starts at `no` and is promoted only with that evidence.

The harnesses a row may cite:

| Key | What it compares | Runs |
|---|---|---|
| `golden:<probe>` | Krita's own compiled output against ours | `tools/golden/run.sh` + `cargo test --test golden_upstream` |
| `drift:<file>` | a copied Graphite file against the pinned original, token by token | `tools/upstream-diff/run.sh` |

| Domain | Feature | Authority | Initial route | Status | Evidence | Works |
|---|---|---|---|---|---|---|
| Document | layered editable project | Krita `KisDocument`/GIMP `GimpImage` | Rust document + `.rrg` | native | | no |
| Layers | raster layers, ordering, opacity, visibility | both | Rust | native | | no |
| Layers | isolated groups and raster masks | Krita | Rust | native | | no |
| Text | printable ASCII + newline, LTR hard breaks, embedded font8x8 bitmap | redrob-canvas bounded native scope | deterministic Rust semantic rasterizer | native | | no |
| Text | shaping, Unicode, bidi, rich text, platform/system fonts | Krita text stack | excluded from v2 native scope | planned | | no |
| Vector | bounded paths, optional solid fill/stroke, non-zero/even-odd fill | redrob-canvas bounded native scope | deterministic Rust semantic rasterizer | native | | no |
| Vector | deterministic SVG subset: paths/rect/line/polyline/polygon, solid literal paint, groups, and Redrob namespaced font8x8 text | redrob-canvas bounded native scope | bounded Rust XML/path adapter | native | | no |
| Vector | constructed shapes: rectangle, rounded rectangle, ellipse, regular polygon, star, line | Graphite `vector-types` | ported geometry behind `Command::AddShapeNode`; the SHAPE is recorded in history and the project file, not its expanded coordinates | parity | `drift:shapes`, `drift:misc` | no |
| Vector | curve intersection, path offsetting, spline solving, Poisson-disk sampling | Graphite `vector-types` | ported into `crates/redrob-core/src/geometry`; reachable from Rust, not yet exposed as commands | parity | `drift:bezpath_algorithms`, `drift:intersection`, `drift:offset_bezpath`, `drift:spline`, `drift:poisson_disk`, `drift:polynomial`, `drift:util`, `drift:consts`, `drift:glam_ext`, `drift:convert` | no |
| Vector | general SVG, transforms, gradients, dashes, CSS, scripts, external data, clipping, filters, animation, variable joins/caps, path booleans | Krita Flake/SVG | explicitly rejected by native subset | planned | | no |
| Layers | pass-through group projection | Krita | unsupported in v2; future Rust projection | planned | | no |
| History | grouped undo/redo with bounded memory | both | Rust command snapshots/deltas | native | | no |
| Paint | pressure-aware round brush stroke | Krita paint-op | Rust baseline | native | | no |
| Paint | dab shape: hardness, softness, aspect ratio, edge antialiasing | Krita `kis_circle_mask_generator` | translated into `crates/redrob-core/src/dab_shape.rs`, reached as `BrushSettings::shape` | parity | `golden:krita-mask-probe`, `golden:krita-antialias-probe` | no |
| Paint | image brush tips from GBR files | Krita `kis_gbr_brush` | translated into `crates/redrob-core/src/brush_tip.rs`, carried on `Command::BrushStroke` | parity | `golden:krita-gbr-probe` | no |
| Paint | image brush tips from ABR collections, versions 1, 2 and 6.1-6.2, with PackBits | Krita `kis_abr_brush_collection` | translated into `crates/redrob-core/src/abr.rs`; recovers the sampled brush Krita's own v1/v2 seek loses | parity | `golden:krita-abr-probe` | no |
| Paint | dab spacing on an ellipse, per axis, with a fraction floor and a pixel floor | Krita `kis_paintop_utils`, `kis_distance_information` | translated into `crates/redrob-core/src/spacing.rs` | parity | `golden:krita-spacing-probe` | no |
| Paint | GIH image pipes: a tip SET with a per-dab selection rule | Krita `kis_imagepipe_brush` | blocked by a concept, not effort — this product carries one tip per stroke; see `docs/krita-global-brush-coverage.md` | planned | | no |
| Paint | bucket fill with a Lab-distance tolerance and both spread policies | Krita `kis_scanline_fill`, `differenceA` via lcms2 | translated into `crates/redrob-core/src/flood_fill.rs` behind `Command::FloodFill` | parity | `golden:lab-difference-probe` | no |
| Paint | gap closing across line-art breaks | Krita `kis_gap_map` | out of scope until a line-art mode justifies it | planned | | no |
| Paint | full preset/sensor engine | Krita paint-op registry | Krita adapter then Rust ports | planned | | no |
| Selection | grayscale mask; replace/add/subtract/intersect | GIMP channels | Rust | native | | no |
| Filters | invert, grayscale, blur | GIMP/GEGL | Rust baseline | native | | no |
| Filters | arbitrary transfer curve with corner knots | Krita `kis_cubic_curve` | translated into `crates/redrob-core/src/tone_curve.rs` behind `Filter::Curves`; a tridiagonal solve replaces Eigen's sparse one, proven equivalent to 1.7e-15 | parity | `golden:krita-spline-probe` | no |
| Transform | affine transform with a scale-aware downscale filter | Krita `kis_filter_weights_buffer`, `kis_filter_strategy` | translated into `sample_rgba_filtered`; the support widens by `1/scale` when shrinking, and stops widening past a 1/256 scale as Krita's own bound does | parity | `golden:krita-downscale-probe` | no |
| Transform | perspective, warp, liquify, cage, grid interpolation, Bezier mesh | Krita transform workers | no such tool in this product; recorded consumerless in `docs/krita-image-coverage.md` | planned | | no |
| Render | dirty-region projection: a dab recomposites only what it damaged | Krita `kis_base_rects_walker`, `kis_async_merger` | translated as `Damage` + `try_render_damage`; the threading is deliberately not taken. **Cannot have a golden probe and will stay `native`**: the walker's substance is graph traversal over `KisProjectionLeaf` and `KisLayer`, the node model this product deliberately does not have, so extracting it standalone would mean reimplementing that model. What verifies this instead is SELF-equivalence — an incrementally updated projection must be byte-identical to a full render, checked after every one of 24 mixed steps — which is a stronger criterion here than matching Krita, because the full renderer is the actual correctness standard | native | | no |
| Render | layer styles: drop shadow, glows, bevel, overlays, and Photoshop ASL | Krita `layerstyles`, `psdutils/asl` | a product feature this product does not have; not a translation | planned | | no |
| Telemetry | rolling latency statistics over a filtered mean | Krita `kis_latency_tracker`, `KisFilteredRollingMean` | translated into `crates/redrob-core/src/telemetry.rs`; a monotonic deque replaces boost's fibonacci heap | parity | `golden:krita-latency-probe` | no |
| Composite | normal, multiply, screen, overlay | both | Rust baseline | native | | no |
| Color | tagged embedded ICC profiles, and non-destructive transforms through them | lcms2 | optional native C adapter behind `REDROB_ENABLE_LCMS` | native | | no |
| Color | full GEGL colour-managed operation pipeline | GIMP/GEGL | GEGL adapter | planned | | no |
| Color | sRGB u8 to linear float conversion, and back, for correct compositing | babl | optional native C adapter behind `REDROB_ENABLE_BABL` | native | | no |
| Files | RRG v1 import/v2 import-export and exact RRG/PNG compatibility wrappers | redrob-canvas | bounded Rust core + additive generic FFI v2 | native | | no |
| Files | strict generic PNG/JPEG/lossless-WebP raster codecs, explicit-frame export, JPEG matte/quality policy | codec specifications | bounded Rust `image` adapter routed through FFI and Qt/QML | native | | no |
| Files | bounded ORA raster/group hierarchy, five exact blend modes, offsets, deterministic ZIP/XML export | OpenRaster | bounded Rust ZIP/XML adapter routed through FFI and Qt/QML | native | | no |
| Files | limited deterministic SVG subset with explicit embedded-raster loss warnings | redrob-canvas bounded native scope | bounded Rust SVG adapter routed through FFI and Qt/QML | native | | no |
| Files | KRA/XCF/PSD and unrestricted codec catalog | both | isolated import/export adapters | planned | | no |
| Animation | sparse raster frame timeline, frame CRUD, playback range/loop | Krita | Rust timeline + native Qt playback | native | | no |
| Animation | onion skin rendering and controls | Krita | bounded adjacent-frame render projections | planned | | no |
| Automation | typed command/procedure registry | GIMP PDB | Rust command registry | native | | no |
| Agent | streaming Redrob tool calls | Redrob Code | native Rust HTTP/SSE | native | | no |
| Agent | generation-and-document-epoch-bound preview and explicit per-proposal approval; current proposals apply during playback in one core commit, stale/invalid proposals remain inert, command proposals create separate entries, undo/redo navigate history | redrob-canvas | command bus | native | | no |

`native` means the architecture has an owned implementation, not full upstream parity. A feature may move to `parity` only after golden-output and interaction checks against the pinned authority. Adapter compilation or dependency discovery is not behavioral support — but three of the four adapters have now moved past that, so the old blanket sentence would be wrong.

Where each stands, and why they differ:

| Adapter | State | What decided it |
|---|---|---|
| babl | operational, `ready=true`, five formats | Owns no buffer and no graph. Nothing to decide first. |
| lcms2 | operational, `ready=true`, three layouts | Same: a caller builds a transform from two profiles and applies it. |
| GEGL | operational, `ready=true`, three of its 205 operations | Owns a buffer AND a graph, so a boundary had to be chosen: `GeglBuffer` stays inside one call and never reaches our model. |
| Krita | scaffold only, `ready=false`, empty arrays | Has no linkable ABI at all — 0 `.pc` files, 0 `Config.cmake`. Source verification is all the scaffold does. |

The three operational adapters check each other arithmetically, which is how we know they are wired rather than merely compiling. babl states that byte 128 is 0.21586 in linear light. lcms2 reaches 0.21587 for the same byte through an ICC matrix-shaper pipeline. GEGL's `gegl:invert-linear` turns 128 into 229 — which is that same value inverted in linear light and re-encoded to sRGB. Each adapter's test asserts the others' constants, so a change that breaks the agreement fails in three places.

GEGL's offered set is deliberately three of 205. `apply_rgba` takes an operation name and nothing else, so anything needing a radius or an amount cannot be expressed through it; those stay `planned` until the seam grows a parameter mechanism, which is a separate decision rather than an oversight.

Nothing here is `parity`: no golden-output check against GIMP or Krita has run in this repository yet. Stage 2 of the porting plan builds that harness.