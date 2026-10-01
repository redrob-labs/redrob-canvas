/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_babl_adapter.h"

#include <babl/babl.h>
#include <pthread.h>
#include <string.h>

/* pthread rather than GMutex, unlike the GEGL seam. That seam gets GLib for free
 * because GEGL links it; babl does not, and pulling GLib in to guard two booleans
 * would add a dependency this adapter has no other use for. */
static pthread_mutex_t lifecycle_mutex = PTHREAD_MUTEX_INITIALIZER;
static bool initialized;
static bool formats_ready;
static size_t resolved_format_count;

static const char *const required_formats[REDROB_BABL_REQUIRED_FORMAT_COUNT] = {
    REDROB_BABL_FORMAT_SRGB_U8,
    REDROB_BABL_FORMAT_LINEAR_F32,
    REDROB_BABL_FORMAT_SRGB_F32,
    REDROB_BABL_FORMAT_LINEAR_U16,
    REDROB_BABL_FORMAT_GRAY_U8,
};

RedrobBablAdapterCapabilities redrob_babl_adapter_capabilities(void)
{
    RedrobBablAdapterCapabilities capabilities = {true, false, false, 0, 0};
    pthread_mutex_lock(&lifecycle_mutex);
    capabilities.initialized = initialized;
    capabilities.ready = formats_ready;
    capabilities.required_format_count = resolved_format_count;
    capabilities.format_count = resolved_format_count;
    pthread_mutex_unlock(&lifecycle_mutex);
    return capabilities;
}

bool redrob_babl_adapter_initialize(void)
{
    pthread_mutex_lock(&lifecycle_mutex);
    if (!initialized) {
        babl_init();
        initialized = true;

        /* Resolve every format we depend on now, not at first use. babl_format
         * ABORTS the process on a name babl does not know -- it calls
         * babl_fatal(), it does not return NULL -- so babl_format_exists() is the
         * only safe way to ask, and asking here means a babl too old to know one
         * of these names produces ready=false rather than a crash mid-render. */
        resolved_format_count = 0;
        for (size_t index = 0; index < REDROB_BABL_REQUIRED_FORMAT_COUNT; ++index) {
            if (babl_format_exists(required_formats[index])) {
                ++resolved_format_count;
            }
        }
        formats_ready = resolved_format_count == REDROB_BABL_REQUIRED_FORMAT_COUNT;
    }
    const bool ready = formats_ready;
    pthread_mutex_unlock(&lifecycle_mutex);
    return ready;
}

void redrob_babl_adapter_shutdown(void)
{
    pthread_mutex_lock(&lifecycle_mutex);
    if (initialized) {
        babl_exit();
        initialized = false;
        formats_ready = false;
        resolved_format_count = 0;
    }
    pthread_mutex_unlock(&lifecycle_mutex);
}

size_t redrob_babl_adapter_bytes_per_pixel(const char *format)
{
    if (format == NULL) {
        return 0;
    }
    pthread_mutex_lock(&lifecycle_mutex);
    const bool usable = initialized;
    pthread_mutex_unlock(&lifecycle_mutex);
    if (!usable) {
        return 0;
    }
    /* babl_format_exists first, always: babl_format on an unknown name aborts. */
    if (!babl_format_exists(format)) {
        return 0;
    }
    const Babl *resolved = babl_format(format);
    return resolved != NULL ? (size_t)babl_format_get_bytes_per_pixel(resolved) : 0;
}

bool redrob_babl_adapter_convert(const char *from_format,
                                 const char *to_format,
                                 const void *input,
                                 size_t input_len,
                                 void *output,
                                 size_t output_len,
                                 size_t pixel_count)
{
    if (from_format == NULL || to_format == NULL || input == NULL || output == NULL) {
        return false;
    }

    pthread_mutex_lock(&lifecycle_mutex);
    const bool usable = initialized && formats_ready;
    pthread_mutex_unlock(&lifecycle_mutex);
    if (!usable) {
        return false;
    }

    /* Zero pixels is a valid no-op, and saying so here keeps callers from having
     * to special-case an empty region. */
    if (pixel_count == 0) {
        return true;
    }

    /* Both names are checked for existence BEFORE either is resolved. babl_format
     * aborts the process on an unknown name, so this is the line that turns a
     * caller's typo into a false return instead of a dead application. */
    if (!babl_format_exists(from_format) || !babl_format_exists(to_format)) {
        return false;
    }

    const Babl *source = babl_format(from_format);
    const Babl *destination = babl_format(to_format);
    if (source == NULL || destination == NULL) {
        return false;
    }

    const size_t source_stride = (size_t)babl_format_get_bytes_per_pixel(source);
    const size_t destination_stride = (size_t)babl_format_get_bytes_per_pixel(destination);
    if (source_stride == 0 || destination_stride == 0) {
        return false;
    }

    /* Overflow before the comparison, not after it. pixel_count arrives from a
     * caller that may have computed width * height, and on a 32-bit size_t a
     * large image multiplied by 16 bytes per pixel wraps -- which would turn this
     * bounds check into a permission slip for the overrun it exists to stop. */
    if (pixel_count > SIZE_MAX / source_stride || pixel_count > SIZE_MAX / destination_stride) {
        return false;
    }
    if (input_len < pixel_count * source_stride || output_len < pixel_count * destination_stride) {
        return false;
    }

    const Babl *fish = babl_fish(source, destination);
    if (fish == NULL) {
        return false;
    }
    babl_process(fish, input, output, (long)pixel_count);
    return true;
}

bool redrob_babl_adapter_srgb_u8_to_linear_f32(const uint8_t *input,
                                               size_t input_len,
                                               float *output,
                                               size_t output_len,
                                               size_t pixel_count)
{
    return redrob_babl_adapter_convert(REDROB_BABL_FORMAT_SRGB_U8,
                                       REDROB_BABL_FORMAT_LINEAR_F32,
                                       input,
                                       input_len,
                                       output,
                                       output_len,
                                       pixel_count);
}

bool redrob_babl_adapter_linear_f32_to_srgb_u8(const float *input,
                                               size_t input_len,
                                               uint8_t *output,
                                               size_t output_len,
                                               size_t pixel_count)
{
    return redrob_babl_adapter_convert(REDROB_BABL_FORMAT_LINEAR_F32,
                                       REDROB_BABL_FORMAT_SRGB_U8,
                                       input,
                                       input_len,
                                       output,
                                       output_len,
                                       pixel_count);
}
