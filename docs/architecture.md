# Architecture

## Ownership

`Editor` owns open documents and services. Each `Document` owns a layer tree, selection mask, color context, render generation, and bounded history. The Rust core is authoritative for state. Qt receives snapshots and change events through a stable C ABI.

## Command pipeline

```text
QML intent ─┐
            ├─> Command Bus -> validate -> preview/execute -> transaction -> history -> ChangeSet
Agent tool ─┘                                                            |
                                                                          v
                                                               immutable render snapshot
```

UI actions and agent tools never mutate layers or pixels directly. A command has a stable name, typed JSON representation, validation, one transaction boundary, and deterministic result. Every command prepares a detached candidate, applies the edit and playback stop there, validates it, then commits exactly one generation and history transition; rejection leaves the document, current frame, playback, generation, and history untouched. Agent responses are queued as generation-and-document-epoch-bound, inert proposals. The user applies each proposal explicitly. Freshness is checked before dispatch, so a current proposal can apply while playback is active while stale or invalid proposals remain inert. The five semantic command proposals are first dry-run against a private authoritative document snapshot through the same complete candidate admission path used by `Editor`; this covers semantic work, render topology, and raster storage without changing direct tool execution. Apply still revalidates every successfully applied command proposal before creating its own history entry; approved undo and redo proposals navigate existing history instead of creating entries.

## Timeline and raster cels

Project v2 keeps raster content sparse: each raster node stores only the `RasterCel` values that exist for stable frame IDs. A missing cel on the current frame renders as transparent. Read-only rendering never allocates a cel; the first raster mutation on a missing current-frame cel preflights the document-wide storage budget and lazily materializes one transparent cel. Frame duplication shares source `RasterBytes` through clone-on-write storage, while blank frame insertion creates no cels.

Timeline FPS is the playback-clock authority (`0 < fps <= 240`). Setting FPS normalizes every frame duration to deterministic `round(1000 / fps)`, clamped to at least 1 ms; newly added and duplicated frames use the same value. Frame CRUD, ordering, FPS, playback range, and looping are typed transactional commands with stable IDs and history entries. Every successful command stops playback inside its detached transaction while preserving the viewed frame unless the command itself removes it; the returned change set reports that timeline/navigation transition.

Current-frame selection, runtime playing state, and playback ticks use `Editor::navigate`, not the history command path. Navigation clones, applies, fully validates, commits, and advances generation so render snapshots and frame-dependent Agent proposals become stale, but it neither records history nor clears redo. Undo and redo preserve the viewed frame ID when it still exists, choose the target snapshot's deterministic valid frame otherwise, and stop playback only after a detached target validates successfully. Qt never stops core playback before dispatch: after a successful command/history commit it stops only its local timer, then lets a coherent refresh publish authoritative state.

## Rendering

The editable layer tree and flattened render projection are separate concepts, following Krita and GIMP. Nodes use canonical bottom-to-top post-order: every subtree is contiguous and a group follows all of its descendants. The CPU renderer visits roots and siblings bottom-to-top. A normal group composites visible children into a transparent intermediate, then applies the enabled group raster mask, group opacity, and group blend mode exactly once when compositing that intermediate into its parent. Enabled raster-node masks multiply source alpha coverage; disabled masks are inert. `RasterMaskFromSelection` requires an active selection, copies its full-canvas 8-bit coverage into an existing or newly attached target mask, and always enables the result; inactive selections and unsupported target kinds are rejected transactionally. Invisible ancestors skip their complete subtree. Pass-through groups are intentionally unsupported in v2—there is no pass-through command or serialized state.

Before allocating full-canvas output, semantic temporaries, or group intermediates, rendering checks peak working storage and checked aggregate work budgets. `MAX_RENDER_PIXEL_VISITS` counts the output clear, each visible raster/semantic composite, the semantic temporary, and both the clear and composite for each visible isolated group; excessive valid wide trees return typed errors. Semantic-specific preflight separately checks flattened segments, text work, and 4x4 sample-edge visits for each enabled fill and stroke paint branch. All limit failures return errors rather than successful blank projections.

## Bounded core format adapters

The Rust core owns a generic typed boundary for RRG, PNG, JPEG, lossless WebP, ORA, and a deliberately limited deterministic SVG subset. Detection is based on strict content signatures and callers may require an expected format; unknown and mismatched content is rejected. `ImportOptions` and `ExportOptions` default to `RejectLoss`, outcomes carry structured warnings and effective metadata, JPEG validates quality without clamping, and non-opaque JPEG output requires either `RejectNonOpaque` success or an explicit opaque matte. Existing RRG and PNG functions are compatibility wrappers and retain their prior signatures and behavior.

FFI v2 exposes this boundary additively through `redrob_editor_import_file`, `redrob_editor_export_file`, and `redrob_ffi_capabilities_json`; no existing struct layout or signature changes. Options are strict deny-unknown schema-v1 JSON bounded to 64 KiB. Import fully decodes into a new `Editor` before one locked replacement. Export clones a coherent document under the lock, encodes after releasing it, and cannot navigate or alter playback, generation, or history. Both output buffers are reset before every failure. Result JSON reports detected/effective format, dimensions, selected frame, lossless state, JPEG quality, and stable warning codes. Qt supplies extension-derived expected-format hints so renamed or mismatched content fails, and publishes bytes only through `QSaveFile`.

Raster decoding uses guarded `image` readers with the core dimension, pixel, allocation, and encoded-input limits. PNG and the pinned `image` WebP encoder preserve straight alpha; only the encoder's deterministic lossless WebP mode is exposed. Explicit-frame rendering validates the stable `FrameId` and renders directly from immutable document state, without navigation, generation changes, or history entries. Current-frame rendering delegates to that path.

`DocumentImportBuilder` is detached and atomic. It accepts raster/text/vector/group payloads, hierarchy and node properties, timeline frames, masks, metadata, and selection state without exposing invariant-bearing fields. Build derives frame durations, canonicalizes bottom-to-top postorder, and runs the unchanged complete document validator before returning any document; no placeholder layer is inserted.

ORA is an in-memory bounded ZIP/XML adapter. It requires a first, stored `mimetype` entry with exact `image/openraster` bytes and `stack.xml`; rejects duplicate, absolute, backslash, empty-component, dot, and traversal paths; never extracts files; and bounds entries, individual/expanded bytes, XML, depth, nodes, decoded pixels, incremental archive output, and final document work. DTD/entity content and unknown composite operations reject. The native mapping is limited to raster layers, isolated groups, x/y placement, visibility, opacity, and exact Normal/Multiply/Screen/Overlay/Add operations. Export writes deterministic top-first ORA XML over the core's bottom-first tree, deterministic layer paths, source PNGs, and `mergedimage.png`. Strict mode rejects semantic source loss, masks, extra frames, active selection, and unrepresentable document metadata; `AllowLoss` may rasterize semantic source nodes without node opacity/blend, bake raster masks once, omit metadata, and reports each loss. Enabled group masks always reject because ORA output could otherwise render incorrectly.

The SVG adapter is not a general SVG implementation. It accepts numeric pixel canvas dimensions with an exactly matching simple viewBox; bounded groups; literal solid colors; exact visibility, opacity, and Redrob namespaced blend properties; M/L/H/V/C/S/Z path commands; and exact lowering for rect, line, polyline, and polygon. Redrob namespaced top-left `font8x8` text is the only text representation. DTDs, entities, scripts, CSS, event handlers, external URLs, use, filters, clipping, masks, animation, transforms, gradients, patterns, dashes, unsupported path commands, and unknown attributes reject. Raster nodes require explicit `AllowLoss` data-PNG mode with warnings. Every import still passes semantic budgets and final document validation.

## Limited native text and vector semantics

Project v2 supports a deliberately narrow deterministic semantic path. Text stores a top-left origin, color, size, metadata-only `font_family`, and the fixed embedded ID `font8x8-basic-0.3.1`. Only printable ASCII, newline, left-to-right advance, and hard line breaks are accepted. The core never asks Qt, the OS, a platform painter, a shaping service, a GPU, or font fallback to resolve text. Vector nodes contain bounded move/line/cubic/close paths with non-zero or even-odd fill, optional solid RGBA fill, and optional bounded solid stroke; there are no transforms, gradients, dashes, variable caps/joins, general SVG semantics, or boolean path operations.

The core quantizes semantic geometry to 1/256 pixel, flattens every cubic in a fixed bounded order, clips work to the canvas, and evaluates coverage at a fixed 4x4 sample grid with integer rounding. Every drawable subpath starts with `MoveTo`. `Close` ends that subpath and requires a new `MoveTo` before any further line, cubic, or close command. A filled subpath receives an implicit closing edge before the next `MoveTo` and at path end; stroke-only subpaths remain open. An explicit return to the start followed by `Close` is valid and adds no duplicate edge. Quantized zero-length lines and fully stationary cubics are deterministic no-ops. Every path must have fill or stroke; an empty `VectorContent` is the only paintless transparent vector state. Validation and flattening share this exact state machine.

It creates one straight-alpha temporary and sends it through the existing node compositing boundary, so node opacity and blend are applied exactly once. `RasterizeSemanticNode` replaces only node content with one current-frame raster cel: ID, name, parent/order, visibility, opacity, and blend remain unchanged; selection is neither consumed nor baked; other frames are transparent. Undo and redo restore exact semantic/raster snapshots. This is a native implementation, not a claim of Krita/GIMP text or vector parity.

Project admission caps encoded JSON at 512 MiB. Semantic strings and arrays are bounded while decoding: text content 256 KiB, font family 256 bytes, font ID 128 bytes, 4,096 vector paths, and 65,536 commands per path (with a 1,000,000-command document cap). The v2 node and vector-path visitors ignore untrusted size hints, enforce the 4,096-node bound (as does the v1 layer visitor), and incrementally charge document-wide text, path, command, and logical semantic memory usage before retaining each node or path. Admission stops at the first over-limit element. Document-wide semantic admission limits text bytes to 1 MiB, paths to 4,096, commands to 1,000,000, and logical semantic memory (string bytes plus text/path/command/style structures) to 64 MiB; `Document::validate` retains the same checks for programmatically constructed state. Add/Set commands use the shared checked replacement arithmetic, and semantic Agent proposals additionally dry-run the full candidate so render-work and raster-storage limits match Apply on the captured snapshot.

The Qt Quick canvas uploads immutable generation-numbered snapshots without traversing the document. Qt obtains document, layer, and timeline metadata from one coherent owned-JSON FFI projection and publishes them only when their generation also matches render and selection snapshots. `FrameModel` exposes ordered stable frame IDs and boundary roles; a native timer drives non-history playback navigation at the authoritative FPS. Qt projects the core `can_edit_raster` capability as one active-node property used by every raster-only QML tool and action; hierarchy rows similarly expose sibling count and top/bottom boundaries. A future `wgpu` backend keeps the same interface.

## Source-of-truth policy

- Krita: paint engines, stroke behavior, layer projection, animation.
- GIMP/GEGL: filters, blend semantics, raster selections, color conversion.
- redrob-graphics: Rust API, QML UX, command model, agent interaction.

An implementation is compatible only when its matrix entry names an upstream reference, fixture, and measurable acceptance criterion. Architecture similarity alone is not parity.

## Native integration seams

- `redrob-core`: safe Rust domain model and renderer.
- `redrob-ffi`: opaque editor handle, fixed-width values, caller-owned paths, explicit buffers.
- `EditorBridge` (C++): QObject/QAbstractListModel façade for QML.
- `CanvasItem`: render snapshot consumer; no model mutation.
- GEGL adapter: optional native C library linked to system GEGL 0.4.66+; no GEGL/GObject handle crosses the Redrob ABI, and no product operation or format is currently implemented.
- Krita adapter: optional C++ source-boundary scaffold with explicit detached/attached lifecycle; it reports `ready=false` and implements no KRA or paint operation.

No Qt container, C++ template, GObject pointer, or Rust allocation crosses the public ABI without an explicit ownership function. Capability reporting separates compilation/lifecycle facts from product readiness: the default product reports both adapters `compiled=false`, `ready=false`, with empty operation/format arrays; an enabled GEGL library may report initialized while remaining unready, and the Krita scaffold reports `scaffold_compiled` and `attached` separately from readiness.

## Agent safety

Redrob receives a dedicated network-safe semantic summary, never the local LayerModel projection: text contributes byte/character/line counts, font/style/origin metadata, and capabilities, but no raw text or preview. Tools map one-to-one to commands. Read-only tools may execute immediately; mutating tools produce a proposal and preview; destructive operations and export require commit. Every tool call records model, arguments, result, cost metadata when available, and resulting history entry.

## Plugins

Plugins run out of process and register versioned procedures with JSON Schema arguments/results. Bulk pixels move through bounded shared memory or streams. Filesystem, network, and UI capabilities are declared and granted separately.