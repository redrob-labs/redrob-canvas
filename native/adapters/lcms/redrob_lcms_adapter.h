/* SPDX-License-Identifier: GPL-3.0-or-later */
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Optional Little-CMS 2 seam. ABI floor: lcms2 2.16.
 *
 * This is the third adapter beside GEGL and babl, and it exists because babl
 * cannot do this job. babl converts between formats it KNOWS -- named encodings
 * and their spaces. An image that arrives carrying an embedded ICC profile is
 * tagged with a space nobody enumerated in advance, and turning those bytes into
 * our working space needs an ICC engine. That is what lcms2 is.
 *
 * So the division is: lcms2 gets the pixels INTO and OUT OF our working space
 * when a profile is involved; babl moves them around inside it. They agree where
 * they overlap -- an sRGB-to-linear transform through lcms2 produces 0.21587 for
 * 128 where babl produces 0.21586, measured, which is how we know both are wired
 * correctly rather than merely compiling.
 *
 * No lcms2 type crosses this boundary, the same rule the GEGL seam follows for
 * GObject. Profiles and transforms are opaque handles whose definitions stay
 * inside the translation unit; callers hold pointers and raw bytes.
 *
 * NON-DESTRUCTIVE is a property of this API, not a promise in a comment: every
 * apply function takes a const input and a separate output, and there is no
 * in-place variant. A display transform must never touch the document. */

/* ALPHA IS THE TRAP HERE, and it is worth stating in the header because the
 * correct behaviour is not lcms2's default.
 *
 * lcms2 treats the fourth channel of TYPE_RGBA_* as an EXTRA channel and does not
 * copy extra channels unless cmsFLAGS_COPY_ALPHA is set. Measured on lcms2 2.17:
 * without that flag a 256-pixel round trip left the colour channels byte-exact
 * (0 of 768 wrong) and destroyed 255 of 256 alpha bytes -- every pixel arrives
 * fully transparent. With the flag, 0 of 256 alpha bytes differ.
 *
 * Every transform this adapter builds therefore sets it, unconditionally. There
 * is no option to turn it off, because there is no version of this product that
 * wants a display transform to erase its own alpha. */

typedef struct RedrobLcmsProfile RedrobLcmsProfile;
typedef struct RedrobLcmsTransform RedrobLcmsTransform;

/* Which pixel layouts a transform moves between.
 *
 * A transform is built for one layout pair and validates against it, rather than
 * taking the layout at apply time. Building an ICC transform is expensive --
 * lcms2 constructs lookup tables -- so a caller caches the handle and reuses it
 * per frame; carrying the layout in the handle is what lets apply check the
 * buffer sizes it was actually given. */
typedef enum RedrobLcmsLayout {
    /* Import: tagged 8-bit source into our linear float working space. */
    REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA_F32 = 0,
    /* Export: linear float working space back out to 8-bit. */
    REDROB_LCMS_LAYOUT_RGBA_F32_TO_RGBA8 = 1,
    /* Display: 8-bit straight to 8-bit, for showing a tagged image as-is. */
    REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA8 = 2,
} RedrobLcmsLayout;

typedef struct RedrobLcmsAdapterCapabilities {
    bool compiled;
    bool initialized;
    /* True once initialized and the built-in sRGB profile resolved. */
    bool ready;
    /* lcms2's own encoded version, e.g. 2170 for 2.17. Zero before initialize.
     * Reported rather than the pkg-config string because this is the version
     * actually loaded at runtime, which is the one that matters. */
    unsigned int engine_version;
    size_t layout_count;
} RedrobLcmsAdapterCapabilities;

RedrobLcmsAdapterCapabilities redrob_lcms_adapter_capabilities(void);
bool redrob_lcms_adapter_initialize(void);
void redrob_lcms_adapter_shutdown(void);

/* The built-in sRGB profile. Owned by the adapter; do NOT close it. Returned as
 * the same pointer every time so a caller can compare identity. */
RedrobLcmsProfile *redrob_lcms_adapter_srgb_profile(void);

/* Open a profile from an embedded ICC blob -- a PNG iCCP chunk, a JPEG APP2
 * segment, the bytes a document carried with it.
 *
 * Returns NULL on anything malformed. Verified on lcms2 2.17 that this is a
 * refusal and not a crash: a 128-byte run of 0xAB returns NULL with "not an ICC
 * profile, invalid signature", and a zero length returns NULL too. Close it with
 * redrob_lcms_adapter_close_profile. */
RedrobLcmsProfile *redrob_lcms_adapter_open_profile(const uint8_t *icc, size_t icc_len);
void redrob_lcms_adapter_close_profile(RedrobLcmsProfile *profile);

/* The profile's human description, as ICC recorded it. Returns the length
 * excluding NUL, and writes a NUL-terminated possibly-truncated copy when
 * destination is non-NULL and capacity > 0 -- the same contract as
 * redrob_adapter_capabilities_json. */
size_t redrob_lcms_adapter_profile_description(const RedrobLcmsProfile *profile,
                                               char *destination,
                                               size_t capacity);

/* Build a cached transform. NULL when either profile is NULL, the layout is
 * unknown, or lcms2 declines to build it. */
RedrobLcmsTransform *redrob_lcms_adapter_create_transform(const RedrobLcmsProfile *source,
                                                          const RedrobLcmsProfile *destination,
                                                          RedrobLcmsLayout layout);
void redrob_lcms_adapter_close_transform(RedrobLcmsTransform *transform);

/* Apply a transform. Bounds are checked against BOTH layouts' strides, because
 * the source and destination sizes differ by a factor of four in two of the three
 * layouts and a check sized by one side would authorise an overrun of the other.
 *
 * Returns false and converts nothing on a size or NULL failure. Zero pixels is a
 * successful no-op. */
bool redrob_lcms_adapter_apply(const RedrobLcmsTransform *transform,
                               const void *input,
                               size_t input_len,
                               void *output,
                               size_t output_len,
                               size_t pixel_count);

/* Bytes per pixel for each side of a layout, so callers size buffers from the
 * same source of truth the bounds check uses. Zero for an unknown layout. */
size_t redrob_lcms_adapter_layout_input_stride(RedrobLcmsLayout layout);
size_t redrob_lcms_adapter_layout_output_stride(RedrobLcmsLayout layout);

/* The last error lcms2 reported, or an empty string. lcms2 writes diagnostics
 * through a global handler that defaults to stderr; this adapter installs its own
 * so a malformed profile in a document does not print to a user's terminal, and
 * so the reason is available to whoever asked. */
size_t redrob_lcms_adapter_last_error(char *destination, size_t capacity);

#ifdef __cplusplus
}
#endif
