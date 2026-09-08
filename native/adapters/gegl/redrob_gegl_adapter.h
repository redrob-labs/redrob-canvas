/* SPDX-License-Identifier: GPL-3.0-or-later */
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Optional GEGL C seam. ABI floor: GEGL 0.4.66, matching the pinned GIMP source.
 * No GeglBuffer/GObject crosses into redrob-ffi; callers retain their RGBA bytes. */
typedef struct RedrobGeglAdapterCapabilities {
    bool compiled;
    bool initialized;
    bool ready;
    size_t operation_count;
    size_t format_count;
} RedrobGeglAdapterCapabilities;

/* Querying capabilities has no lifecycle side effects. compiled may be true
 * when the optional library is built, while ready and both counts remain zero
 * until a measured product operation is implemented. */
RedrobGeglAdapterCapabilities redrob_gegl_adapter_capabilities(void);
bool redrob_gegl_adapter_initialize(void);
void redrob_gegl_adapter_shutdown(void);

/* Reserved typed raster operation seam. Returns false until a pinned fixture
 * defines the requested operation's measurable behavior. */
bool redrob_gegl_adapter_apply_rgba(const char *operation,
                                    const uint8_t *input_rgba,
                                    size_t input_len,
                                    uint32_t width,
                                    uint32_t height,
                                    uint8_t *output_rgba,
                                    size_t output_len);

#ifdef __cplusplus
}
#endif
