/* SPDX-License-Identifier: GPL-3.0-or-later */
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Optional GEGL C seam. ABI floor: GEGL 0.4.66, matching the pinned GIMP source.
 * No GeglBuffer/GObject crosses into redrob-ffi; callers retain their RGBA bytes.
 *
 * This seam was reserved -- capabilities zero, apply returning false -- because
 * GEGL, unlike babl and lcms2 beside it, OWNS a buffer and a graph, and nothing
 * could be exercised before deciding how GeglBuffer relates to our raster
 * surface. That decision is now made, and it is the conservative one: it does
 * not. A caller hands over RGBA bytes, the adapter builds a GeglBuffer around a
 * copy of them, runs a graph, and reads the result back into the caller's output.
 * The buffer is created and destroyed inside one call.
 *
 * The cost of that choice is real and is worth stating rather than discovering:
 * a graph is constructed per call, so this is not the path for a per-frame
 * pipeline. It is the path for applying a filter to a layer once. A cached-graph
 * variant would need GeglBuffer to become part of our document model, which is a
 * much larger decision than this seam should make on its own.
 *
 * WHAT IS OFFERED IS NARROWER THAN WHAT GEGL HAS, deliberately. apply_rgba takes
 * an operation NAME and nothing else, so an operation needing a radius, an amount
 * or a curve cannot be expressed through it at all. GEGL 0.4.70 was measured to
 * carry 205 operations; three of them are parameterless, verifiable by hand, and
 * preserve alpha through an R'G'B'A u8 round trip, so three is what this claims.
 * The rest stay `planned` in docs/compatibility.md until the contract grows a
 * parameter mechanism -- which is a separate decision, not an oversight. */

/* The operations this adapter offers, checked against GEGL at initialize time.
 *
 * gegl:invert-linear and gegl:invert both invert in linear light, measured to
 * produce identical bytes on 0.4.70. Both are listed because they are distinct
 * names in GEGL's catalogue and a caller may reasonably ask for either. */
#define REDROB_GEGL_OPERATION_INVERT_LINEAR "gegl:invert-linear"
#define REDROB_GEGL_OPERATION_INVERT "gegl:invert"
#define REDROB_GEGL_OPERATION_GREY "gegl:grey"
#define REDROB_GEGL_SUPPORTED_OPERATION_COUNT 3

/* The one pixel format that crosses this boundary. Kept to one on purpose: every
 * additional format is another conversion to verify, and babl already exists next
 * door for moving between formats. */
#define REDROB_GEGL_FORMAT "R'G'B'A u8"
#define REDROB_GEGL_BYTES_PER_PIXEL 4

typedef struct RedrobGeglAdapterCapabilities {
    bool compiled;
    bool initialized;
    /* True once initialized and every operation in the supported list is present
     * in GEGL's catalogue. A partially available GEGL is not usable: the filter
     * that silently did not run is worse than the one that refused. */
    bool ready;
    /* How many of OUR supported operations GEGL actually has, out of
     * REDROB_GEGL_SUPPORTED_OPERATION_COUNT. */
    size_t operation_count;
    /* Formats this seam accepts. One. */
    size_t format_count;
    /* How many operations GEGL's catalogue holds in total -- 205 on 0.4.70 with
     * the Debian plug-in set. Reported separately from operation_count because
     * the difference between "GEGL can do 205 things" and "this product offers
     * three of them" is exactly the distinction a capability report exists to
     * make, and collapsing the two is how a roadmap starts overclaiming.
     *
     * It is a RUNTIME measurement, not a constant: a plug-in that fails to load
     * lowers it. On the machine this was developed on, matting-levin.so does not
     * load because libumfpack is absent, and the count is 205 rather than 206. */
    size_t catalog_operation_count;
} RedrobGeglAdapterCapabilities;

/* Querying capabilities has no lifecycle side effects. */
RedrobGeglAdapterCapabilities redrob_gegl_adapter_capabilities(void);
bool redrob_gegl_adapter_initialize(void);
void redrob_gegl_adapter_shutdown(void);

/* The supported operation at `index`, or NULL past the end. Lets a caller
 * enumerate what is on offer without hardcoding the list a second time. */
const char *redrob_gegl_adapter_operation_name(size_t index);

/* Whether this adapter offers `operation`. Distinct from GEGL merely having it:
 * this answers "will apply_rgba accept this", which is the question a caller has.
 * Safe on any string, including NULL. */
bool redrob_gegl_adapter_supports_operation(const char *operation);

/* Apply one operation over a width x height block of R'G'B'A u8 pixels.
 *
 * Returns false and writes NOTHING when: the adapter is not ready, the operation
 * is not one this adapter offers, either pointer is NULL, width or height is
 * zero, or either buffer is smaller than width * height * 4. The output is left
 * exactly as the caller had it on every failure, so a refused call cannot be
 * mistaken for a successful no-op that happened to change nothing.
 *
 * Input and output may not overlap. There is no in-place variant, because a
 * filter preview must not destroy the layer it previews. */
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
