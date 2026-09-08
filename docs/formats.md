<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# File-format routes and degradation policy

## Product workflows

`currentFile` is exclusively the editable RRG project path. **Open Project** and **Save Project/Save As** accept `.rrg` only. **Import** accepts `.png`, `.jpg`/`.jpeg`, `.webp`, `.ora`, and `.svg`; successful import creates a fresh editor/history and clears the project path. **Export Current Frame** accepts those same interchange formats, never sets the project path, never navigates, and does not change playback, generation, undo, or redo. Unknown extensions fail. Every read supplies an expected-format hint and the Rust core verifies content, so renaming a file cannot bypass detection. Every write is encoded before a `QSaveFile` atomic commit.

The QML export dialog defaults to strict loss rejection. Allowing loss is an explicit checkbox and successful degradations appear as machine-readable warning codes in the status. JPEG additionally requires quality `1..=100` and an explicitly selected opaque matte; alpha is composited over that matte and is never silently discarded.

## Routed formats

| Format | Extensions | Import | Export | Effective losslessness | Exact limitations |
|---|---|---:|---:|---:|---|
| RRG | `rrg` | yes | yes | lossless | Editable project route. Explicit frame selection is invalid because the complete project/timeline is serialized. |
| PNG | `png` | yes | yes | lossless pixels | Flat interchange. Strict export rejects omitted frames, selection, metadata, hierarchy flattening, or semantic rasterization; allow-loss reports each degradation. Alpha is preserved. |
| JPEG | `jpg`, `jpeg` | yes | yes | no | Flat RGB interchange. Same structural loss rules as PNG, quality is validated without clamping, and nonopaque pixels require explicit opaque-matte compositing. |
| WebP | `webp` | yes | yes | lossless pixels | Only the deterministic lossless encoder route is exposed. Structural degradation rules match PNG. |
| OpenRaster | `ora` | yes | yes | lossless for supported subset | Bounded ZIP/XML; raster layers, isolated groups, offsets, visibility, opacity, and Normal/Multiply/Screen/Overlay/Add. Unsupported composites/security features reject. Semantic source, masks, metadata, selection, and extra frames reject or warn only where the core defines an allow-loss mapping. Enabled group masks always reject. |
| SVG | `svg` | yes | yes | lossless for supported semantic subset | Not general SVG. Supports bounded literal geometry/groups/solid paint and Redrob namespaced font8x8 text. Scripts, CSS, external URLs, transforms, gradients, filters, masks, clipping, animation, and unknown attributes reject. Raster data requires allow-loss data-PNG embedding and emits `embedded_raster_data`. |

## Generic FFI v2

The additive symbols preserve all existing ABI layouts and signatures:

```c
int32_t redrob_editor_import_file(RedrobEditor *, const uint8_t *, size_t,
                                  const uint8_t *options_json, size_t options_len,
                                  RedrobBuffer *out_result_json);
int32_t redrob_editor_export_file(RedrobEditor *,
                                  const uint8_t *options_json, size_t options_len,
                                  RedrobBuffer *out_bytes,
                                  RedrobBuffer *out_result_json);
int32_t redrob_ffi_capabilities_json(RedrobBuffer *out_json);
```

Options use strict schema-versioned JSON, reject unknown fields/values, and are capped at 65,536 bytes. Encoded input is capped at 64 MiB and output at 512 MiB; core dimension, pixel, expanded archive, XML, semantic, and work limits also apply. Import options map `expected_format`, `loss_policy`, and `max_input_bytes`. Export options map `format`, optional stable `frame`, `loss_policy`, `jpeg_quality`, and tagged `jpeg_alpha` (`reject_non_opaque` or `flatten` with opaque RGBA matte).

Import decodes and validates before atomically replacing the locked editor. Failure leaves it unchanged. Export clones a coherent document under lock and performs encoding outside the lock. The two export output pointers must be distinct. On every failure all non-null output buffers are reset to `{NULL,0}`. Success result JSON includes detected/effective format, dimensions, frame, lossless state, JPEG quality, and warning objects with stable `code` values. Returned buffers are Rust-owned and must be released once with `redrob_buffer_free`.

The existing RRG and PNG C functions remain compatibility wrappers. In particular, legacy PNG export retains its historical allow-loss flattening behavior; new UI export uses the generic explicit policy instead.

## Adapter non-capabilities

Built-in format routes do not depend on GEGL or Krita. In the default product build, both adapters report `compiled=false`, `ready=false`, `operations:[]`, and `formats:[]`. An enabled GEGL adapter can report lifecycle initialization but still has no product operation. The current Krita source scaffold reports `scaffold_compiled` and attachment separately while remaining unready. KRA, XCF, PSD, unrestricted SVG, lossy WebP, and general codec/plugin catalogs are unsupported.
