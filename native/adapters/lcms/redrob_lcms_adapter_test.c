/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_lcms_adapter.h"

/* lcms2 is included here, and only here in the test, to BUILD a profile blob to
 * feed the adapter through its public byte interface. Writing the fixture with
 * the same engine that will read it keeps the test self-contained -- no binary
 * ICC file to check in, nothing to regenerate when a profile convention changes. */
#include <lcms2.h>

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

static void check(bool condition, const char *what)
{
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", what);
        ++failures;
    }
}

static void check_close(float actual, float expected, float tolerance, const char *what)
{
    if (fabsf(actual - expected) > tolerance) {
        fprintf(stderr, "FAIL: %s (got %.6f, expected %.6f +/- %.6f)\n",
                what, (double)actual, (double)expected, (double)tolerance);
        ++failures;
    }
}

/* A linear-gamma profile with sRGB primaries and a D65 white point, serialised to
 * an ICC blob. This is the working space the import path targets: same gamut as
 * sRGB, no transfer curve. */
static uint8_t *build_linear_profile(size_t *length)
{
    cmsCIExyY whitepoint;
    cmsCIExyYTRIPLE primaries = {
        {0.6400, 0.3300, 1.0}, {0.3000, 0.6000, 1.0}, {0.1500, 0.0600, 1.0}};
    cmsWhitePointFromTemp(&whitepoint, 6504);
    cmsToneCurve *curves[3];
    curves[0] = curves[1] = curves[2] = cmsBuildGamma(NULL, 1.0);
    cmsHPROFILE profile = cmsCreateRGBProfile(&whitepoint, &primaries, curves);
    if (profile == NULL) {
        cmsFreeToneCurve(curves[0]);
        return NULL;
    }
    cmsMLU *description = cmsMLUalloc(NULL, 1);
    cmsMLUsetASCII(description, "en", "US", "Redrob linear working space");
    cmsWriteTag(profile, cmsSigProfileDescriptionTag, description);

    cmsUInt32Number needed = 0;
    cmsSaveProfileToMem(profile, NULL, &needed);
    uint8_t *bytes = malloc(needed);
    if (bytes != NULL && !cmsSaveProfileToMem(profile, bytes, &needed)) {
        free(bytes);
        bytes = NULL;
    }
    *length = bytes != NULL ? (size_t)needed : 0;

    cmsMLUfree(description);
    cmsCloseProfile(profile);
    cmsFreeToneCurve(curves[0]);
    return bytes;
}

int main(void)
{
    /* --- before initialize --------------------------------------------------- */
    RedrobLcmsAdapterCapabilities before = redrob_lcms_adapter_capabilities();
    check(before.compiled, "compiled is true when the adapter is built");
    check(!before.initialized, "initialized is false before initialize");
    check(!before.ready, "ready is false before initialize");
    check(before.engine_version == 0, "no engine version before initialize");
    check(before.layout_count == 0, "no layouts before initialize");
    check(redrob_lcms_adapter_srgb_profile() == NULL, "no sRGB profile before initialize");

    uint8_t stub[4] = {0, 0, 0, 0};
    check(redrob_lcms_adapter_open_profile(stub, sizeof stub) == NULL,
          "opening a profile refuses before initialize");

    /* --- initialize ---------------------------------------------------------- */
    check(redrob_lcms_adapter_initialize(), "initialize reports ready");
    RedrobLcmsAdapterCapabilities after = redrob_lcms_adapter_capabilities();
    check(after.initialized, "initialized is true after initialize");
    check(after.ready, "ready is true after initialize");
    check(after.engine_version >= 2160, "engine version is at least 2.16");
    check(after.layout_count == 3, "three layouts are supported");
    printf("lcms2 engine version: %u\n", after.engine_version);

    RedrobLcmsProfile *srgb = redrob_lcms_adapter_srgb_profile();
    check(srgb != NULL, "the built-in sRGB profile resolves");
    check(srgb == redrob_lcms_adapter_srgb_profile(),
          "the sRGB profile is the same pointer every call");

    /* Closing the adapter-owned profile must be harmless, because a caller that
     * closes everything it was handed is behaving reasonably. */
    redrob_lcms_adapter_close_profile(srgb);
    check(redrob_lcms_adapter_srgb_profile() != NULL,
          "closing the built-in profile does not destroy it");

    /* --- a profile from bytes, the real embedded-ICC path ------------------- */
    size_t blob_len = 0;
    uint8_t *blob = build_linear_profile(&blob_len);
    check(blob != NULL && blob_len > 0, "the test built an ICC blob to open");

    RedrobLcmsProfile *working = redrob_lcms_adapter_open_profile(blob, blob_len);
    check(working != NULL, "a valid ICC blob opens");

    char description[128];
    const size_t description_len =
        redrob_lcms_adapter_profile_description(working, description, sizeof description);
    check(description_len > 0, "the profile reports a description");
    check(strcmp(description, "Redrob linear working space") == 0,
          "the description is the one written into the blob");
    printf("profile description: %s\n", description);

    /* --- strides ------------------------------------------------------------- */
    check(redrob_lcms_adapter_layout_input_stride(REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA_F32) == 4,
          "RGBA8 input is 4 bytes");
    check(redrob_lcms_adapter_layout_output_stride(REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA_F32) == 16,
          "RGBA float output is 16 bytes");
    check(redrob_lcms_adapter_layout_input_stride(REDROB_LCMS_LAYOUT_RGBA_F32_TO_RGBA8) == 16,
          "RGBA float input is 16 bytes");
    check(redrob_lcms_adapter_layout_output_stride(REDROB_LCMS_LAYOUT_RGBA_F32_TO_RGBA8) == 4,
          "RGBA8 output is 4 bytes");
    check(redrob_lcms_adapter_layout_input_stride((RedrobLcmsLayout)99) == 0,
          "an unknown layout has no stride");

    /* --- the numbers, cross-checked against babl --------------------------- */
    RedrobLcmsTransform *import = redrob_lcms_adapter_create_transform(
        srgb, working, REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA_F32);
    check(import != NULL, "an sRGB to linear transform builds");

    const uint8_t tagged[8] = {255, 128, 0, 255, 0, 64, 255, 128};
    float linear[8] = {0};
    check(redrob_lcms_adapter_apply(import, tagged, sizeof tagged, linear, sizeof linear, 2),
          "the import transform applies");

    /* babl produces 0.21586 for 128 and 0.05126 for 64. lcms2 arriving at the same
     * numbers through an ICC matrix-shaper pipeline is the evidence that both
     * adapters are wired correctly: two independent engines, one answer. The
     * tolerance is 1e-3 because ICC round-off is real -- lcms2 returns 1.00031 for
     * 255, not exactly 1.0 -- and tightening it would make the test fail on
     * arithmetic that is not wrong. */
    check_close(linear[1], 0.21586f, 1e-3f, "128 sRGB is 0.2159 linear (babl agrees)");
    check_close(linear[5], 0.05126f, 1e-3f, "64 sRGB is 0.0513 linear (babl agrees)");
    check_close(linear[0], 1.0f, 1e-3f, "255 sRGB is 1.0 linear");
    check_close(linear[2], 0.0f, 1e-3f, "0 sRGB is 0.0 linear");

    /* ALPHA. This is the regression this adapter exists to prevent: without
     * cmsFLAGS_COPY_ALPHA these are 0.0 and every pixel is invisible. */
    check_close(linear[3], 1.0f, 1e-4f, "opaque alpha survives the transform");
    check_close(linear[7], 0.50196f, 1e-4f, "half alpha survives as 128/255, not zero");

    /* --- round trip, colour AND alpha, across the whole 8-bit range -------- */
    RedrobLcmsTransform *export_back = redrob_lcms_adapter_create_transform(
        working, srgb, REDROB_LCMS_LAYOUT_RGBA_F32_TO_RGBA8);
    check(export_back != NULL, "a linear to sRGB transform builds");

    uint8_t ramp[256 * 4];
    float ramp_linear[256 * 4];
    uint8_t ramp_back[256 * 4];
    for (size_t value = 0; value < 256; ++value) {
        ramp[value * 4 + 0] = (uint8_t)value;
        ramp[value * 4 + 1] = (uint8_t)(255 - value);
        ramp[value * 4 + 2] = (uint8_t)((value * 7) % 256);
        /* Alpha VARIES. A ramp with constant alpha cannot tell "copied" from
         * "coincidentally right". */
        ramp[value * 4 + 3] = (uint8_t)value;
    }
    check(redrob_lcms_adapter_apply(import, ramp, sizeof ramp, ramp_linear, sizeof ramp_linear, 256),
          "the full ramp imports");
    check(redrob_lcms_adapter_apply(export_back, ramp_linear, sizeof ramp_linear, ramp_back,
                                    sizeof ramp_back, 256),
          "the full ramp exports back");

    size_t colour_bad = 0;
    size_t alpha_bad = 0;
    int worst_colour = 0;
    for (size_t value = 0; value < 256; ++value) {
        for (size_t channel = 0; channel < 3; ++channel) {
            const int delta =
                (int)ramp_back[value * 4 + channel] - (int)ramp[value * 4 + channel];
            if (delta != 0) {
                ++colour_bad;
                const int magnitude = delta < 0 ? -delta : delta;
                if (magnitude > worst_colour) {
                    worst_colour = magnitude;
                }
            }
        }
        if (ramp_back[value * 4 + 3] != ramp[value * 4 + 3]) {
            ++alpha_bad;
        }
    }
    if (colour_bad != 0) {
        fprintf(stderr, "FAIL: %zu of 768 colour bytes changed, worst delta %d\n",
                colour_bad, worst_colour);
        ++failures;
    }
    if (alpha_bad != 0) {
        fprintf(stderr,
                "FAIL: %zu of 256 alpha bytes changed -- cmsFLAGS_COPY_ALPHA is not in effect\n",
                alpha_bad);
        ++failures;
    }

    /* --- refusals ------------------------------------------------------------ */
    check(redrob_lcms_adapter_open_profile(NULL, 16) == NULL, "a NULL blob is refused");
    check(redrob_lcms_adapter_open_profile(blob, 0) == NULL, "a zero-length blob is refused");

    uint8_t garbage[128];
    memset(garbage, 0xAB, sizeof garbage);
    check(redrob_lcms_adapter_open_profile(garbage, sizeof garbage) == NULL,
          "a malformed blob is refused rather than crashing");
    /* The adapter's internal buffer is 256 bytes; anything at least that large
     * exercises the same path without the test needing a private macro. */
    char error[256];
    const size_t error_len = redrob_lcms_adapter_last_error(error, sizeof error);
    check(error_len > 0, "the malformed blob left a reason behind");
    printf("last error: %s\n", error);

    check(redrob_lcms_adapter_create_transform(NULL, working,
                                               REDROB_LCMS_LAYOUT_RGBA8_TO_RGBA8) == NULL,
          "a transform from a NULL profile is refused");
    check(redrob_lcms_adapter_create_transform(srgb, working, (RedrobLcmsLayout)99) == NULL,
          "a transform with an unknown layout is refused");
    check(!redrob_lcms_adapter_apply(NULL, tagged, sizeof tagged, linear, sizeof linear, 2),
          "applying a NULL transform is refused");
    check(!redrob_lcms_adapter_apply(import, tagged, sizeof tagged, linear, sizeof linear, 3),
          "a pixel count past the input buffer is refused");

    /* The asymmetric case: 8 input bytes are two RGBA8 pixels, but two linear
     * pixels need 32 output bytes. A bounds check sized by the input alone would
     * pass this and write 32 bytes into 16. */
    float narrow[4] = {0};
    check(!redrob_lcms_adapter_apply(import, tagged, sizeof tagged, narrow, sizeof narrow, 2),
          "output bounds are checked against the output stride");

    check(redrob_lcms_adapter_apply(import, tagged, sizeof tagged, linear, sizeof linear, 0),
          "zero pixels is a successful no-op");

    /* --- shutdown ------------------------------------------------------------ */
    redrob_lcms_adapter_close_transform(import);
    redrob_lcms_adapter_close_transform(export_back);
    redrob_lcms_adapter_close_profile(working);
    free(blob);

    redrob_lcms_adapter_shutdown();
    RedrobLcmsAdapterCapabilities closed = redrob_lcms_adapter_capabilities();
    check(!closed.initialized, "initialized is false after shutdown");
    check(!closed.ready, "ready is false after shutdown");
    check(closed.engine_version == 0, "no engine version after shutdown");
    check(redrob_lcms_adapter_srgb_profile() == NULL, "no sRGB profile after shutdown");

    if (failures != 0) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return EXIT_FAILURE;
    }
    printf("redrob_lcms_adapter: all checks passed\n");
    return EXIT_SUCCESS;
}
