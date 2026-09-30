/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_lcms_adapter.h"

#include <lcms2.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

/* pthread rather than GMutex, matching the babl seam and for the same reason:
 * lcms2 does not link GLib and this adapter has no other use for it. */
static pthread_mutex_t lifecycle_mutex = PTHREAD_MUTEX_INITIALIZER;
static bool initialized;
static unsigned int engine_version;
static cmsHPROFILE builtin_srgb;

/* lcms2's diagnostics go through one global handler that defaults to writing to
 * stderr. A document carrying a malformed profile is a routine, recoverable event
 * -- it must not print to the user's terminal -- so the handler is replaced and
 * the text kept here for whoever asked. */
#define REDROB_LCMS_ERROR_CAPACITY 256
static pthread_mutex_t error_mutex = PTHREAD_MUTEX_INITIALIZER;
static char last_error[REDROB_LCMS_ERROR_CAPACITY];

/* The opaque handles. lcms2's own types never leave this file, so the structs are
 * declared in the header and defined only here. */
struct RedrobLcmsProfile {
    cmsHPROFILE handle;
    /* The built-in sRGB profile is adapter-owned and must survive a caller
     * mistakenly closing it, so closing is a no-op when this is set. */
    bool adapter_owned;
};

struct RedrobLcmsTransform {
    cmsHTRANSFORM handle;
    RedrobLcmsLayout layout;
};

static void capture_error(cmsContext context, cmsUInt32Number code, const char *text)
{
    (void)context;
    (void)code;
    if (text == NULL) {
        return;
    }
    pthread_mutex_lock(&error_mutex);
    strncpy(last_error, text, REDROB_LCMS_ERROR_CAPACITY - 1);
    last_error[REDROB_LCMS_ERROR_CAPACITY - 1] = '\0';
    pthread_mutex_unlock(&error_mutex);
}

/* The three supported layouts, as lcms2 format constants plus their strides.
 * Kept in one table so the bounds check, the stride accessors and the transform
 * builder cannot disagree about what a layout means. */
struct LayoutSpec {
    cmsUInt32Number input_format;
    cmsUInt32Number output_format;
    size_t input_stride;
    size_t output_stride;
};

static const struct LayoutSpec layouts[] = {
    [REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA_F32] = {TYPE_RGBA_8, TYPE_RGBA_FLT, 4, 16},
    [REDROB_LCMS_LAYOUT_RGBA_F32_TO_RGBA8] = {TYPE_RGBA_FLT, TYPE_RGBA_8, 16, 4},
    [REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA8] = {TYPE_RGBA_8, TYPE_RGBA_8, 4, 4},
};
#define REDROB_LCMS_LAYOUT_COUNT (sizeof layouts / sizeof layouts[0])

static bool layout_is_known(RedrobLcmsLayout layout)
{
    return (size_t)layout < REDROB_LCMS_LAYOUT_COUNT;
}

RedrobLcmsAdapterCapabilities redrob_lcms_adapter_capabilities(void)
{
    RedrobLcmsAdapterCapabilities capabilities = {true, false, false, 0, 0};
    pthread_mutex_lock(&lifecycle_mutex);
    capabilities.initialized = initialized;
    capabilities.ready = initialized && builtin_srgb != NULL;
    capabilities.engine_version = engine_version;
    capabilities.layout_count = initialized ? REDROB_LCMS_LAYOUT_COUNT : 0;
    pthread_mutex_unlock(&lifecycle_mutex);
    return capabilities;
}

bool redrob_lcms_adapter_initialize(void)
{
    pthread_mutex_lock(&lifecycle_mutex);
    if (!initialized) {
        /* Before anything that can fail, so the first failure is captured rather
         * than printed. */
        cmsSetLogErrorHandler(capture_error);
        engine_version = (unsigned int)cmsGetEncodedCMMversion();
        builtin_srgb = cmsCreate_sRGBProfile();
        initialized = true;
    }
    const bool ready = builtin_srgb != NULL;
    pthread_mutex_unlock(&lifecycle_mutex);
    return ready;
}

void redrob_lcms_adapter_shutdown(void)
{
    pthread_mutex_lock(&lifecycle_mutex);
    if (initialized) {
        if (builtin_srgb != NULL) {
            cmsCloseProfile(builtin_srgb);
            builtin_srgb = NULL;
        }
        cmsSetLogErrorHandler(NULL);
        engine_version = 0;
        initialized = false;
    }
    pthread_mutex_unlock(&lifecycle_mutex);
    pthread_mutex_lock(&error_mutex);
    last_error[0] = '\0';
    pthread_mutex_unlock(&error_mutex);
}

RedrobLcmsProfile *redrob_lcms_adapter_srgb_profile(void)
{
    /* One static wrapper, so the returned pointer is stable across calls and a
     * caller can compare identity. */
    static struct RedrobLcmsProfile wrapper;
    pthread_mutex_lock(&lifecycle_mutex);
    const bool usable = initialized && builtin_srgb != NULL;
    wrapper.handle = builtin_srgb;
    wrapper.adapter_owned = true;
    pthread_mutex_unlock(&lifecycle_mutex);
    return usable ? &wrapper : NULL;
}

RedrobLcmsProfile *redrob_lcms_adapter_open_profile(const uint8_t *icc, size_t icc_len)
{
    if (icc == NULL || icc_len == 0) {
        return NULL;
    }
    pthread_mutex_lock(&lifecycle_mutex);
    const bool usable = initialized;
    pthread_mutex_unlock(&lifecycle_mutex);
    if (!usable) {
        return NULL;
    }

    /* lcms2 takes a 32-bit length. A blob larger than that is not a profile
     * anyone embedded; refuse rather than truncate into a wrong parse. */
    if (icc_len > 0xFFFFFFFFu) {
        return NULL;
    }

    cmsHPROFILE handle = cmsOpenProfileFromMem(icc, (cmsUInt32Number)icc_len);
    if (handle == NULL) {
        return NULL;
    }
    struct RedrobLcmsProfile *profile = calloc(1, sizeof *profile);
    if (profile == NULL) {
        cmsCloseProfile(handle);
        return NULL;
    }
    profile->handle = handle;
    profile->adapter_owned = false;
    return profile;
}

void redrob_lcms_adapter_close_profile(RedrobLcmsProfile *profile)
{
    if (profile == NULL || profile->adapter_owned) {
        return;
    }
    if (profile->handle != NULL) {
        cmsCloseProfile(profile->handle);
    }
    free(profile);
}

size_t redrob_lcms_adapter_profile_description(const RedrobLcmsProfile *profile,
                                               char *destination,
                                               size_t capacity)
{
    if (destination != NULL && capacity > 0) {
        destination[0] = '\0';
    }
    if (profile == NULL || profile->handle == NULL) {
        return 0;
    }

    /* Ask for the length first, then read into a local buffer. cmsGetProfileInfoASCII
     * needs somewhere to write; it does not report a length without one. */
    char text[256];
    const cmsUInt32Number written = cmsGetProfileInfoASCII(
        profile->handle, cmsInfoDescription, "en", "US", text, (cmsUInt32Number)sizeof text);
    if (written == 0) {
        return 0;
    }
    text[sizeof text - 1] = '\0';
    const size_t required = strlen(text);
    if (destination != NULL && capacity > 0) {
        const size_t copied = required < capacity - 1 ? required : capacity - 1;
        memcpy(destination, text, copied);
        destination[copied] = '\0';
    }
    return required;
}

RedrobLcmsTransform *redrob_lcms_adapter_create_transform(const RedrobLcmsProfile *source,
                                                          const RedrobLcmsProfile *destination,
                                                          RedrobLcmsLayout layout)
{
    if (source == NULL || destination == NULL || !layout_is_known(layout)) {
        return NULL;
    }
    if (source->handle == NULL || destination->handle == NULL) {
        return NULL;
    }
    pthread_mutex_lock(&lifecycle_mutex);
    const bool usable = initialized;
    pthread_mutex_unlock(&lifecycle_mutex);
    if (!usable) {
        return NULL;
    }

    const struct LayoutSpec spec = layouts[layout];

    /* cmsFLAGS_COPY_ALPHA is NOT optional and NOT the default. Without it, lcms2
     * treats the fourth channel as an extra channel it has no instructions about
     * and leaves it at zero: measured on 2.17, a 256-pixel round trip kept the
     * colour channels byte-exact and destroyed 255 of 256 alpha bytes, which
     * renders every pixel fully transparent. There is no configuration of this
     * product that wants that, so the flag is set here rather than exposed. */
    cmsHTRANSFORM handle = cmsCreateTransform(source->handle,
                                              spec.input_format,
                                              destination->handle,
                                              spec.output_format,
                                              INTENT_RELATIVE_COLORIMETRIC,
                                              cmsFLAGS_COPY_ALPHA);
    if (handle == NULL) {
        return NULL;
    }
    struct RedrobLcmsTransform *transform = calloc(1, sizeof *transform);
    if (transform == NULL) {
        cmsDeleteTransform(handle);
        return NULL;
    }
    transform->handle = handle;
    transform->layout = layout;
    return transform;
}

void redrob_lcms_adapter_close_transform(RedrobLcmsTransform *transform)
{
    if (transform == NULL) {
        return;
    }
    if (transform->handle != NULL) {
        cmsDeleteTransform(transform->handle);
    }
    free(transform);
}

size_t redrob_lcms_adapter_layout_input_stride(RedrobLcmsLayout layout)
{
    return layout_is_known(layout) ? layouts[layout].input_stride : 0;
}

size_t redrob_lcms_adapter_layout_output_stride(RedrobLcmsLayout layout)
{
    return layout_is_known(layout) ? layouts[layout].output_stride : 0;
}

bool redrob_lcms_adapter_apply(const RedrobLcmsTransform *transform,
                               const void *input,
                               size_t input_len,
                               void *output,
                               size_t output_len,
                               size_t pixel_count)
{
    if (transform == NULL || transform->handle == NULL || input == NULL || output == NULL) {
        return false;
    }
    if (!layout_is_known(transform->layout)) {
        return false;
    }
    if (pixel_count == 0) {
        return true;
    }

    const struct LayoutSpec spec = layouts[transform->layout];

    /* Overflow before the comparison. pixel_count may be a caller's width *
     * height, and multiplying it by 16 can wrap -- which would turn this bounds
     * check into permission for the overrun it exists to stop. */
    if (pixel_count > SIZE_MAX / spec.input_stride ||
        pixel_count > SIZE_MAX / spec.output_stride) {
        return false;
    }
    /* BOTH sides. Two of the three layouts differ by a factor of four between
     * input and output, so a check sized by one side alone authorises an overrun
     * of the other. */
    if (input_len < pixel_count * spec.input_stride ||
        output_len < pixel_count * spec.output_stride) {
        return false;
    }

    /* lcms2 takes a 32-bit pixel count. Refuse rather than silently convert a
     * prefix of a very large image. */
    if (pixel_count > 0xFFFFFFFFu) {
        return false;
    }

    cmsDoTransform(transform->handle, input, output, (cmsUInt32Number)pixel_count);
    return true;
}

size_t redrob_lcms_adapter_last_error(char *destination, size_t capacity)
{
    pthread_mutex_lock(&error_mutex);
    const size_t required = strlen(last_error);
    if (destination != NULL && capacity > 0) {
        const size_t copied = required < capacity - 1 ? required : capacity - 1;
        memcpy(destination, last_error, copied);
        destination[copied] = '\0';
    }
    pthread_mutex_unlock(&error_mutex);
    return required;
}
