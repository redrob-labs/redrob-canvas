/* SPDX-License-Identifier: GPL-3.0-or-later */
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Optional babl C seam. ABI floor: babl 0.1.108.
 *
 * Unlike the GEGL seam next to this one, this adapter is OPERATIONAL rather than
 * reserved, and the difference is a property of babl rather than a choice. GEGL
 * owns a buffer and a graph, so a GEGL operation cannot be exercised without
 * deciding how GeglBuffer relates to our raster surface -- which is why that
 * seam's apply function returns false until a pinned fixture defines the
 * behaviour. babl owns neither: it converts a block of pixels from one named
 * format to another and holds no state beyond its format registry. There is
 * nothing to decide, so there is no reason to ship a stub.
 *
 * No babl type crosses this boundary. Callers pass raw bytes and format NAMES;
 * the Babl* handles stay inside the translation unit. That keeps redrob-ffi free
 * of the dependency, the same rule the GEGL seam follows for GObject.
 *
 * What this buys the product: correct linear compositing. Our raster surface is
 * 8-bit sRGB, and blending gamma-encoded values is wrong -- 50% between black
 * and white is 188, not 128. babl is the shortest correct path from the bytes we
 * have to the linear floats compositing needs, and back. */

/* The formats this product actually asks for, checked at initialize time.
 *
 * Checked up front for a blunter reason than tidiness: babl_format() with a name
 * babl does not know does NOT return NULL -- it calls babl_fatal() and ABORTS THE
 * PROCESS. (Measured, not assumed: the first version of this adapter's test
 * called it with "not a real format" and the test binary died with exit 255
 * instead of reporting a refusal.) So every lookup in this file is guarded by
 * babl_format_exists(), which is the non-fatal query, and no caller-supplied
 * string ever reaches babl_format() unguarded. A format name arriving from
 * outside is untrusted input whose worst case is process death, which makes this
 * a robustness boundary rather than a style preference. */
#define REDROB_BABL_FORMAT_SRGB_U8 "R'G'B'A u8"
#define REDROB_BABL_FORMAT_LINEAR_F32 "RGBA float"
#define REDROB_BABL_FORMAT_SRGB_F32 "R'G'B'A float"
#define REDROB_BABL_FORMAT_LINEAR_U16 "RGBA u16"
#define REDROB_BABL_FORMAT_GRAY_U8 "Y' u8"
#define REDROB_BABL_REQUIRED_FORMAT_COUNT 5

typedef struct RedrobBablAdapterCapabilities {
    bool compiled;
    bool initialized;
    /* True only when every required format resolved. A partially available babl
     * is not a usable one: the conversion that silently did not happen is worse
     * than the one that refused. */
    bool ready;
    size_t format_count;
    /* babl publishes no count of its whole registry, so this is the number of
     * REQUIRED formats that resolved, out of REDROB_BABL_REQUIRED_FORMAT_COUNT.
     * A measured number of the formats we depend on is more useful than an
     * unmeasurable number of the ones we do not. */
    size_t required_format_count;
} RedrobBablAdapterCapabilities;

/* Querying capabilities has no lifecycle side effects. */
RedrobBablAdapterCapabilities redrob_babl_adapter_capabilities(void);
bool redrob_babl_adapter_initialize(void);
void redrob_babl_adapter_shutdown(void);

/* Bytes per pixel for a named format, or 0 when babl does not know the name.
 * Callers size their own buffers with this rather than assuming, because a
 * format name and its stride are not independent facts. */
size_t redrob_babl_adapter_bytes_per_pixel(const char *format);

/* Convert pixel_count pixels from one named format to another.
 *
 * Returns false, converting nothing, when: the adapter is not ready, either
 * name is unknown, either buffer is NULL, or a buffer is too small for
 * pixel_count pixels of its format. The length arguments exist so that check is
 * possible -- a conversion that overruns the destination by one pixel is the
 * failure this seam is shaped to make impossible. */
bool redrob_babl_adapter_convert(const char *from_format,
                                 const char *to_format,
                                 const void *input,
                                 size_t input_len,
                                 void *output,
                                 size_t output_len,
                                 size_t pixel_count);

/* The two conversions the product needs by name, so callers do not repeat the
 * format strings and cannot disagree about which pair is canonical. */
bool redrob_babl_adapter_srgb_u8_to_linear_f32(const uint8_t *input,
                                               size_t input_len,
                                               float *output,
                                               size_t output_len,
                                               size_t pixel_count);
bool redrob_babl_adapter_linear_f32_to_srgb_u8(const float *input,
                                               size_t input_len,
                                               uint8_t *output,
                                               size_t output_len,
                                               size_t pixel_count);

#ifdef __cplusplus
}
#endif
