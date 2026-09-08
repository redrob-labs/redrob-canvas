# Compatibility matrix

Status: `planned`, `native`, `adapter`, `parity`.

| Domain | Feature | Authority | Initial route | Status |
|---|---|---|---|---|
| Document | layered editable project | Krita `KisDocument`/GIMP `GimpImage` | Rust document + `.rrg` | native |
| Layers | raster layers, ordering, opacity, visibility | both | Rust | native |
| Layers | isolated groups and raster masks | Krita | Rust | native |
| Text | printable ASCII + newline, LTR hard breaks, embedded font8x8 bitmap | redrob-graphics bounded native scope | deterministic Rust semantic rasterizer | native |
| Text | shaping, Unicode, bidi, rich text, platform/system fonts | Krita text stack | excluded from v2 native scope | planned |
| Vector | bounded paths, optional solid fill/stroke, non-zero/even-odd fill | redrob-graphics bounded native scope | deterministic Rust semantic rasterizer | native |
| Vector | deterministic SVG subset: paths/rect/line/polyline/polygon, solid literal paint, groups, and Redrob namespaced font8x8 text | redrob-graphics bounded native scope | bounded Rust XML/path adapter | native |
| Vector | general SVG, transforms, gradients, dashes, CSS, scripts, external data, clipping, filters, animation, variable joins/caps, path booleans | Krita Flake/SVG | explicitly rejected by native subset | planned |
| Layers | pass-through group projection | Krita | unsupported in v2; future Rust projection | planned |
| History | grouped undo/redo with bounded memory | both | Rust command snapshots/deltas | native |
| Paint | pressure-aware round brush stroke | Krita paint-op | Rust baseline | native |
| Paint | full preset/sensor engine | Krita paint-op registry | Krita adapter then Rust ports | planned |
| Selection | grayscale mask; replace/add/subtract/intersect | GIMP channels | Rust | native |
| Filters | invert, grayscale, blur | GIMP/GEGL | Rust baseline | native |
| Filters | complete GEGL operation catalog | GIMP/GEGL | GEGL adapter | planned |
| Composite | normal, multiply, screen, overlay | both | Rust baseline | native |
| Color | tagged profiles and non-destructive display conversion | both | LCMS/GEGL adapter | planned |
| Files | RRG v1 import/v2 import-export and exact RRG/PNG compatibility wrappers | redrob-graphics | bounded Rust core + additive generic FFI v2 | native |
| Files | strict generic PNG/JPEG/lossless-WebP raster codecs, explicit-frame export, JPEG matte/quality policy | codec specifications | bounded Rust `image` adapter routed through FFI and Qt/QML | native |
| Files | bounded ORA raster/group hierarchy, five exact blend modes, offsets, deterministic ZIP/XML export | OpenRaster | bounded Rust ZIP/XML adapter routed through FFI and Qt/QML | native |
| Files | limited deterministic SVG subset with explicit embedded-raster loss warnings | redrob-graphics bounded native scope | bounded Rust SVG adapter routed through FFI and Qt/QML | native |
| Files | KRA/XCF/PSD and unrestricted codec catalog | both | isolated import/export adapters | planned |
| Animation | sparse raster frame timeline, frame CRUD, playback range/loop | Krita | Rust timeline + native Qt playback | native |
| Animation | onion skin rendering and controls | Krita | bounded adjacent-frame render projections | planned |
| Automation | typed command/procedure registry | GIMP PDB | Rust command registry | native |
| Agent | streaming Redrob tool calls | Redrob Code | native Rust HTTP/SSE | native |
| Agent | generation-and-document-epoch-bound preview and explicit per-proposal approval; current proposals apply during playback in one core commit, stale/invalid proposals remain inert, command proposals create separate entries, undo/redo navigate history | redrob-graphics | command bus | native |

`native` means the architecture has an owned implementation, not full upstream parity. A feature may move to `parity` only after golden-output and interaction checks against the pinned authority. Adapter compilation or dependency discovery is not behavioral support: current GEGL and Krita capability operation/format arrays are empty, `ready=false`, and KRA/Krita paint remain `planned`.