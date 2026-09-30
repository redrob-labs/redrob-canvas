# Compatibility matrix

Status: `planned`, `native`, `adapter`, `parity`.

| Domain | Feature | Authority | Initial route | Status |
|---|---|---|---|---|
| Document | layered editable project | Krita `KisDocument`/GIMP `GimpImage` | Rust document + `.rrg` | native |
| Layers | raster layers, ordering, opacity, visibility | both | Rust | native |
| Layers | isolated groups and raster masks | Krita | Rust | native |
| Text | printable ASCII + newline, LTR hard breaks, embedded font8x8 bitmap | redrob-canvas bounded native scope | deterministic Rust semantic rasterizer | native |
| Text | shaping, Unicode, bidi, rich text, platform/system fonts | Krita text stack | excluded from v2 native scope | planned |
| Vector | bounded paths, optional solid fill/stroke, non-zero/even-odd fill | redrob-canvas bounded native scope | deterministic Rust semantic rasterizer | native |
| Vector | deterministic SVG subset: paths/rect/line/polyline/polygon, solid literal paint, groups, and Redrob namespaced font8x8 text | redrob-canvas bounded native scope | bounded Rust XML/path adapter | native |
| Vector | constructed shapes: rectangle, rounded rectangle, ellipse, regular polygon, star, line | Graphite `vector-types` | ported geometry behind `Command::AddShapeNode`; the SHAPE is recorded in history and the project file, not its expanded coordinates | native |
| Vector | curve intersection, path offsetting, spline solving, Poisson-disk sampling | Graphite `vector-types` | ported into `crates/redrob-core/src/geometry`; reachable from Rust, not yet exposed as commands | native |
| Vector | general SVG, transforms, gradients, dashes, CSS, scripts, external data, clipping, filters, animation, variable joins/caps, path booleans | Krita Flake/SVG | explicitly rejected by native subset | planned |
| Layers | pass-through group projection | Krita | unsupported in v2; future Rust projection | planned |
| History | grouped undo/redo with bounded memory | both | Rust command snapshots/deltas | native |
| Paint | pressure-aware round brush stroke | Krita paint-op | Rust baseline | native |
| Paint | full preset/sensor engine | Krita paint-op registry | Krita adapter then Rust ports | planned |
| Selection | grayscale mask; replace/add/subtract/intersect | GIMP channels | Rust | native |
| Filters | invert, grayscale, blur | GIMP/GEGL | Rust baseline | native |
| Filters | parameterless GEGL operations over an RGBA8 block: `gegl:invert-linear`, `gegl:invert`, `gegl:grey` | GIMP/GEGL | optional native C adapter behind `REDROB_ENABLE_GEGL` | native |
| Filters | complete GEGL operation catalog — 205 operations, and any taking a radius, amount or curve | GIMP/GEGL | GEGL adapter, once the seam grows a parameter mechanism | planned |
| Composite | normal, multiply, screen, overlay | both | Rust baseline | native |
| Color | tagged embedded ICC profiles, and non-destructive transforms through them | lcms2 | optional native C adapter behind `REDROB_ENABLE_LCMS` | native |
| Color | full GEGL colour-managed operation pipeline | GIMP/GEGL | GEGL adapter | planned |
| Color | sRGB u8 to linear float conversion, and back, for correct compositing | babl | optional native C adapter behind `REDROB_ENABLE_BABL` | native |
| Files | RRG v1 import/v2 import-export and exact RRG/PNG compatibility wrappers | redrob-canvas | bounded Rust core + additive generic FFI v2 | native |
| Files | strict generic PNG/JPEG/lossless-WebP raster codecs, explicit-frame export, JPEG matte/quality policy | codec specifications | bounded Rust `image` adapter routed through FFI and Qt/QML | native |
| Files | bounded ORA raster/group hierarchy, five exact blend modes, offsets, deterministic ZIP/XML export | OpenRaster | bounded Rust ZIP/XML adapter routed through FFI and Qt/QML | native |
| Files | limited deterministic SVG subset with explicit embedded-raster loss warnings | redrob-canvas bounded native scope | bounded Rust SVG adapter routed through FFI and Qt/QML | native |
| Files | KRA/XCF/PSD and unrestricted codec catalog | both | isolated import/export adapters | planned |
| Animation | sparse raster frame timeline, frame CRUD, playback range/loop | Krita | Rust timeline + native Qt playback | native |
| Animation | onion skin rendering and controls | Krita | bounded adjacent-frame render projections | planned |
| Automation | typed command/procedure registry | GIMP PDB | Rust command registry | native |
| Agent | streaming Redrob tool calls | Redrob Code | native Rust HTTP/SSE | native |
| Agent | generation-and-document-epoch-bound preview and explicit per-proposal approval; current proposals apply during playback in one core commit, stale/invalid proposals remain inert, command proposals create separate entries, undo/redo navigate history | redrob-canvas | command bus | native |

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