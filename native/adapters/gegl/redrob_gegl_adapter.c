/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_gegl_adapter.h"

#include <gegl.h>
#include <string.h>

/* GMutex here, unlike the babl and lcms seams which use pthread. GEGL links GLib
 * unconditionally, so this costs nothing extra; those two do not, and pulling GLib
 * in to guard two booleans would have been a dependency for nothing. */
static GMutex lifecycle_mutex;
static bool initialized;
static bool operations_ready;
static size_t resolved_operation_count;
static size_t catalog_count;

static const char *const supported_operations[REDROB_GEGL_SUPPORTED_OPERATION_COUNT] = {
    REDROB_GEGL_OPERATION_INVERT_LINEAR,
    REDROB_GEGL_OPERATION_INVERT,
    REDROB_GEGL_OPERATION_GREY,
};

const char *redrob_gegl_adapter_operation_name(size_t index)
{
    return index < REDROB_GEGL_SUPPORTED_OPERATION_COUNT ? supported_operations[index] : NULL;
}

bool redrob_gegl_adapter_supports_operation(const char *operation)
{
    if (operation == NULL) {
        return false;
    }
    for (size_t index = 0; index < REDROB_GEGL_SUPPORTED_OPERATION_COUNT; ++index) {
        if (strcmp(operation, supported_operations[index]) == 0) {
            return true;
        }
    }
    return false;
}

RedrobGeglAdapterCapabilities redrob_gegl_adapter_capabilities(void)
{
    RedrobGeglAdapterCapabilities capabilities = {true, false, false, 0, 0, 0};
    g_mutex_lock(&lifecycle_mutex);
    capabilities.initialized = initialized;
    capabilities.ready = operations_ready;
    capabilities.operation_count = resolved_operation_count;
    capabilities.format_count = operations_ready ? 1 : 0;
    capabilities.catalog_operation_count = catalog_count;
    g_mutex_unlock(&lifecycle_mutex);
    return capabilities;
}

bool redrob_gegl_adapter_initialize(void)
{
    g_mutex_lock(&lifecycle_mutex);
    if (!initialized) {
        gegl_init(NULL, NULL);
        initialized = true;

        /* gegl_has_operation is the SAFE query -- verified on 0.4.70 that an
         * unknown name returns 0 rather than aborting, unlike babl_format next
         * door. So the catalogue can be interrogated without a guard around the
         * guard. */
        resolved_operation_count = 0;
        for (size_t index = 0; index < REDROB_GEGL_SUPPORTED_OPERATION_COUNT; ++index) {
            if (gegl_has_operation(supported_operations[index])) {
                ++resolved_operation_count;
            }
        }
        operations_ready = resolved_operation_count == REDROB_GEGL_SUPPORTED_OPERATION_COUNT;

        /* The graph nodes the adapter itself builds must exist too, or apply would
         * fail at run time with the capability report still claiming ready. */
        if (!gegl_has_operation("gegl:buffer-source") || !gegl_has_operation("gegl:write-buffer")) {
            operations_ready = false;
        }

        guint listed = 0;
        gchar **all = gegl_list_operations(&listed);
        catalog_count = listed;
        g_free(all);
    }
    const bool ready = operations_ready;
    g_mutex_unlock(&lifecycle_mutex);
    return ready;
}

void redrob_gegl_adapter_shutdown(void)
{
    g_mutex_lock(&lifecycle_mutex);
    if (initialized) {
        gegl_exit();
        initialized = false;
        operations_ready = false;
        resolved_operation_count = 0;
        catalog_count = 0;
    }
    g_mutex_unlock(&lifecycle_mutex);
}

bool redrob_gegl_adapter_apply_rgba(const char *operation,
                                    const uint8_t *input_rgba,
                                    size_t input_len,
                                    uint32_t width,
                                    uint32_t height,
                                    uint8_t *output_rgba,
                                    size_t output_len)
{
    if (input_rgba == NULL || output_rgba == NULL) {
        return false;
    }
    if (!redrob_gegl_adapter_supports_operation(operation)) {
        return false;
    }
    if (width == 0 || height == 0) {
        return false;
    }

    g_mutex_lock(&lifecycle_mutex);
    const bool usable = initialized && operations_ready;
    g_mutex_unlock(&lifecycle_mutex);
    if (!usable) {
        return false;
    }

    /* Overflow before the comparison, in two steps, because width * height can
     * wrap on its own before the bytes-per-pixel multiply is reached. Both
     * dimensions are 32-bit and size_t may be 32-bit too. */
    const size_t pixels_wide = (size_t)width;
    const size_t pixels_high = (size_t)height;
    if (pixels_high != 0 && pixels_wide > SIZE_MAX / pixels_high) {
        return false;
    }
    const size_t pixel_count = pixels_wide * pixels_high;
    if (pixel_count > SIZE_MAX / REDROB_GEGL_BYTES_PER_PIXEL) {
        return false;
    }
    const size_t required = pixel_count * REDROB_GEGL_BYTES_PER_PIXEL;
    if (input_len < required || output_len < required) {
        return false;
    }

    /* Everything that can fail has failed by here, so from this point the output
     * is only written on the success path -- a refused call leaves the caller's
     * buffer exactly as it was. */
    const Babl *format = babl_format(REDROB_GEGL_FORMAT);
    if (format == NULL) {
        return false;
    }

    GeglRectangle extent = {0, 0, (gint)width, (gint)height};
    GeglBuffer *source = gegl_buffer_new(&extent, format);
    if (source == NULL) {
        return false;
    }
    GeglBuffer *sink = gegl_buffer_new(&extent, format);
    if (sink == NULL) {
        g_object_unref(source);
        return false;
    }

    /* A COPY of the caller's bytes goes into the buffer. GEGL must not hold a
     * pointer into memory whose lifetime it does not control, and the caller must
     * be able to keep using its input afterwards. */
    gegl_buffer_set(source, &extent, 0, format, input_rgba, GEGL_AUTO_ROWSTRIDE);

    GeglNode *graph = gegl_node_new();
    bool succeeded = false;
    if (graph != NULL) {
        GeglNode *load = gegl_node_new_child(graph, "operation", "gegl:buffer-source",
                                             "buffer", source, NULL);
        GeglNode *filter = gegl_node_new_child(graph, "operation", operation, NULL);
        GeglNode *store = gegl_node_new_child(graph, "operation", "gegl:write-buffer",
                                              "buffer", sink, NULL);
        if (load != NULL && filter != NULL && store != NULL) {
            gegl_node_link_many(load, filter, store, NULL);
            gegl_node_process(store);
            /* GEGL_ABYSS_NONE: the region asked for is exactly the buffer's own
             * extent, so there is no outside to sample and no edge policy to
             * choose. A parameterised operation with a radius would make this a
             * real decision, which is another reason those are not offered yet. */
            gegl_buffer_get(sink, &extent, 1.0, format, output_rgba, GEGL_AUTO_ROWSTRIDE,
                            GEGL_ABYSS_NONE);
            succeeded = true;
        }
        g_object_unref(graph);
    }

    g_object_unref(source);
    g_object_unref(sink);
    return succeeded;
}
