/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef REDROB_FFI_H
#define REDROB_FFI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version for feature detection by native hosts. Adding a new function or
 * standalone struct is backward-compatible and does not increment this value;
 * only a changed existing layout/signature requires a new ABI version. */
#define REDROB_FFI_ABI_VERSION 2u
#define REDROB_OK 0
#define REDROB_ERROR 1
#define REDROB_PANIC 2
#define REDROB_FORMAT_OPTIONS_SCHEMA_VERSION 1u
#define REDROB_MAX_FORMAT_OPTIONS_JSON_BYTES 65536u

/* Opaque. Created by redrob_editor_create and owned by the caller until
 * redrob_editor_destroy. Never copy, dereference, or free it directly. */
typedef struct RedrobEditor RedrobEditor;

/* Rust-owned immutable bytes. Every non-empty value returned by this API MUST
 * be released exactly once with redrob_buffer_free. C/C++ must never call free,
 * delete, realloc, or an allocator-specific function on data. */
typedef struct RedrobBuffer {
    uint8_t *data;
    size_t len;
} RedrobBuffer;

typedef struct RedrobRenderSnapshot {
    RedrobBuffer rgba; /* tightly packed straight-alpha RGBA8, row-major */
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint64_t generation;
} RedrobRenderSnapshot;

/* One byte of selection coverage per pixel, row-major. active == 0 means no
 * explicit selection and edits affect the whole canvas. mask remains an owned
 * raw mask snapshot and must be released with redrob_buffer_free. */
typedef struct RedrobSelectionMaskSnapshot {
    RedrobBuffer mask;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint64_t generation;
    uint8_t active;
} RedrobSelectionMaskSnapshot;

uint32_t redrob_ffi_abi_version(void);

/* Last errors are thread-local: concurrent callers cannot overwrite another
 * thread's message. Returns required bytes excluding NUL. If destination is
 * non-null and capacity > 0, copies a NUL-terminated, possibly truncated value.
 * The destination remains caller-owned. */
size_t redrob_last_error_copy(char *destination, size_t capacity);

void redrob_buffer_free(RedrobBuffer buffer);

int32_t redrob_editor_create(uint32_t width, uint32_t height, RedrobEditor **out_editor);
void redrob_editor_destroy(RedrobEditor *editor);

/* Synchronous hosted-agent request; native callers should invoke it on a
 * worker thread. editor and both UTF-8 spans are borrowed for the entire call,
 * so the editor must remain alive and the spans must remain readable until the
 * function returns. The editor is snapshotted but never mutated. api_key and
 * prompt must be non-empty; prompts are limited to 16384 bytes. On success,
 * out_json contains owned UTF-8 JSON with assistant_text, usage, and proposals.
 * Each proposal contains title, summary, base_generation, and an action tagged
 * command (with exact redrob-core Command JSON), undo, or redo. The caller owns
 * no returned data directly: every non-empty out_json must be released exactly
 * once with redrob_buffer_free. On error, out_json is reset to {NULL, 0}. */
int32_t redrob_agent_propose(RedrobEditor *editor,
                             const uint8_t *api_key, size_t api_key_len,
                             const uint8_t *prompt, size_t prompt_len,
                             RedrobBuffer *out_json);

/* Input JSON is a borrowed UTF-8 byte span valid only for the call and is
 * limited to 1048576 bytes before deserialization. Output JSON and binary
 * values are Rust-owned RedrobBuffer values and require redrob_buffer_free.
 * Output pointers must be non-null. A successful command, undo, or redo stops
 * playback in the same detached transaction, preserves the current frame when
 * valid, advances generation once, and returns timeline/navigation invalidation
 * when playback changed. Rejected commands and unavailable history actions
 * leave document, frame, playback, generation, and history unchanged. */
int32_t redrob_editor_execute_json(RedrobEditor *editor, const uint8_t *json, size_t json_len,
                                   RedrobBuffer *out_changes_json);
int32_t redrob_editor_undo(RedrobEditor *editor, RedrobBuffer *out_changes_json);
int32_t redrob_editor_redo(RedrobEditor *editor, RedrobBuffer *out_changes_json);
int32_t redrob_editor_document_json(RedrobEditor *editor, RedrobBuffer *out_json);
int32_t redrob_editor_layers_json(RedrobEditor *editor, RedrobBuffer *out_json);
/* ABI-v2-compatible owned JSON projections. timeline_json contains ordered
 * frame id/index/duration/current/in_range plus fps/range/loop/playing and
 * generation. state_json captures document/layers/timeline under one editor
 * lock. On every failure each output is reset to {NULL, 0}. */
int32_t redrob_editor_timeline_json(RedrobEditor *editor, RedrobBuffer *out_json);
int32_t redrob_editor_state_json(RedrobEditor *editor, RedrobBuffer *out_json);
/* Non-history navigation. Successful calls increment generation without
 * recording undo or clearing redo and return an owned ChangeSet JSON buffer. */
int32_t redrob_editor_set_current_frame(RedrobEditor *editor, uint32_t frame_id,
                                        RedrobBuffer *out_changes_json);
int32_t redrob_editor_set_playing(RedrobEditor *editor, bool playing,
                                  RedrobBuffer *out_changes_json);
int32_t redrob_editor_advance_playback(RedrobEditor *editor,
                                       RedrobBuffer *out_changes_json);
int32_t redrob_editor_render_rgba(RedrobEditor *editor, RedrobRenderSnapshot *out_snapshot);
/* New ABI v2-compatible symbol: no existing struct or signature changed. */
int32_t redrob_editor_selection_mask(RedrobEditor *editor,
                                     RedrobSelectionMaskSnapshot *out_snapshot);

/* ABI-v2-compatible generic format additions. Options are borrowed UTF-8 JSON
 * objects limited to REDROB_MAX_FORMAT_OPTIONS_JSON_BYTES. Every object must
 * contain schema_version:1 and unknown fields/values are rejected.
 *
 * Import options:
 *   {"schema_version":1,"expected_format":"rrg|png|jpeg|webp|ora|svg"|null,
 *    "loss_policy":"reject_loss|allow_loss","max_input_bytes":N}
 * expected_format is optional and verifies detected content; extensions are
 * never guessed. Import fully decodes and validates before atomically replacing
 * the editor, then resets generation/history. Failure leaves the editor intact.
 *
 * Export options:
 *   {"schema_version":1,"format":"rrg|png|jpeg|webp|ora|svg",
 *    "frame":u32|null,"loss_policy":"reject_loss|allow_loss",
 *    "jpeg_quality":1..100,
 *    "jpeg_alpha":{"policy":"reject_non_opaque"} |
 *                 {"policy":"flatten","matte":{"r":u8,"g":u8,"b":u8,"a":255}}}
 * Export snapshots the editor and never changes frame, playback, generation,
 * or history. RRG rejects an explicit frame. JPEG never silently drops alpha.
 *
 * Successful result JSON contains schema_version, operation,
 * detected_format, effective_format, dimensions, frame, lossless,
 * jpeg_quality, and machine-readable warning objects. out_bytes and
 * out_result_json must point to distinct RedrobBuffer objects. All output
 * buffers are Rust-owned and require redrob_buffer_free. Every non-null output
 * is reset to {NULL,0} before any failure. */
int32_t redrob_editor_import_file(RedrobEditor *editor,
                                  const uint8_t *bytes, size_t len,
                                  const uint8_t *options_json, size_t options_len,
                                  RedrobBuffer *out_result_json);
int32_t redrob_editor_export_file(RedrobEditor *editor,
                                  const uint8_t *options_json, size_t options_len,
                                  RedrobBuffer *out_bytes,
                                  RedrobBuffer *out_result_json);

/* Returns owned JSON describing exact built-in format routes and product-routed
 * adapter readiness. This is the feature-discovery API; ABI version alone must
 * never be interpreted as adapter or format support. Default builds report the
 * GEGL and Krita routes compiled=false, ready=false, operations=[], formats=[]. */
int32_t redrob_ffi_capabilities_json(RedrobBuffer *out_json);

/* Compatibility wrappers retained unchanged. Project/image input bytes are
 * borrowed for the duration of the call. Loading or importing replaces the
 * current document and resets generation/history. The PNG export wrapper keeps
 * its historical allow-loss flattening behavior. */
int32_t redrob_editor_load_rrg(RedrobEditor *editor, const uint8_t *bytes, size_t len);
int32_t redrob_editor_save_rrg(RedrobEditor *editor, RedrobBuffer *out_bytes);
int32_t redrob_editor_import_png(RedrobEditor *editor, const uint8_t *bytes, size_t len);
int32_t redrob_editor_export_png(RedrobEditor *editor, RedrobBuffer *out_bytes);

#ifdef __cplusplus
} /* extern "C" */
#endif
#endif /* REDROB_FFI_H */
