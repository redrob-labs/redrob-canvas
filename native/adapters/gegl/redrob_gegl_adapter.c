/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_gegl_adapter.h"

#include <gegl.h>

static GMutex lifecycle_mutex;
static bool initialized;

RedrobGeglAdapterCapabilities redrob_gegl_adapter_capabilities(void)
{
    RedrobGeglAdapterCapabilities capabilities = {true, false, false, 0, 0};
    g_mutex_lock(&lifecycle_mutex);
    capabilities.initialized = initialized;
    g_mutex_unlock(&lifecycle_mutex);
    return capabilities;
}

bool redrob_gegl_adapter_initialize(void)
{
    g_mutex_lock(&lifecycle_mutex);
    if (!initialized) {
        gegl_init(NULL, NULL);
        initialized = true;
    }
    g_mutex_unlock(&lifecycle_mutex);
    return true;
}

void redrob_gegl_adapter_shutdown(void)
{
    g_mutex_lock(&lifecycle_mutex);
    if (initialized) {
        gegl_exit();
        initialized = false;
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
    (void)operation;
    (void)input_rgba;
    (void)input_len;
    (void)width;
    (void)height;
    (void)output_rgba;
    (void)output_len;
    /* Deliberately unavailable until operation-specific pinned fixtures exist. */
    return false;
}
