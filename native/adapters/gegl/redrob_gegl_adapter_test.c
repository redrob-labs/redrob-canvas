/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_gegl_adapter.h"

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

static void check_byte(uint8_t actual, uint8_t expected, int tolerance, const char *what)
{
    int delta = (int)actual - (int)expected;
    if (delta < 0) {
        delta = -delta;
    }
    if (delta > tolerance) {
        fprintf(stderr, "FAIL: %s (got %u, expected %u +/- %d)\n",
                what, (unsigned)actual, (unsigned)expected, tolerance);
        ++failures;
    }
}

int main(void)
{
    /* --- before initialize --------------------------------------------------- */
    RedrobGeglAdapterCapabilities before = redrob_gegl_adapter_capabilities();
    check(before.compiled, "compiled is true when the adapter is built");
    check(!before.initialized, "initialized is false before initialize");
    check(!before.ready, "ready is false before initialize");
    check(before.operation_count == 0, "no operations before initialize");
    check(before.format_count == 0, "no formats before initialize");
    check(before.catalog_operation_count == 0, "no catalogue before initialize");

    /* The supported list is static, so it answers before initialize. That is
     * deliberate: a caller can ask what this adapter would offer without paying
     * for GEGL's startup. */
    check(redrob_gegl_adapter_supports_operation(REDROB_GEGL_OPERATION_INVERT_LINEAR),
          "the supported list answers before initialize");
    check(!redrob_gegl_adapter_supports_operation("gegl:gaussian-blur"),
          "a parameterised operation is not offered");
    check(!redrob_gegl_adapter_supports_operation("gegl:not-a-real-op"),
          "an invented operation is not offered");
    check(!redrob_gegl_adapter_supports_operation(NULL), "NULL is not an operation");

    const uint8_t sample[8] = {255, 128, 0, 255, 0, 64, 255, 128};
    uint8_t untouched[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                          sizeof sample, 2, 1, untouched, sizeof untouched),
          "apply refuses before initialize");
    check(untouched[0] == 1 && untouched[7] == 8, "a refused apply writes nothing");

    /* Shutdown before initialize must be harmless. */
    redrob_gegl_adapter_shutdown();

    /* --- initialize, twice -------------------------------------------------- */
    check(redrob_gegl_adapter_initialize(), "initialize reports ready");
    check(redrob_gegl_adapter_initialize(), "initialize is idempotent");

    RedrobGeglAdapterCapabilities after = redrob_gegl_adapter_capabilities();
    check(after.initialized, "initialized is true after initialize");
    check(after.ready, "ready is true after initialize");
    check(after.operation_count == REDROB_GEGL_SUPPORTED_OPERATION_COUNT,
          "every offered operation is present in GEGL");
    check(after.format_count == 1, "one format crosses this boundary");

    /* The catalogue is much larger than what we offer, and the report must say so
     * rather than conflating the two. 205 on 0.4.70 with the Debian plug-in set;
     * asserted as a floor because a plug-in that fails to load lowers it and that
     * is not this adapter's fault. */
    check(after.catalog_operation_count > after.operation_count,
          "GEGL's catalogue is larger than what this adapter offers");
    check(after.catalog_operation_count >= 100,
          "GEGL's catalogue has at least 100 operations");
    printf("GEGL catalogue: %zu operations, of which this adapter offers %zu\n",
           after.catalog_operation_count, after.operation_count);

    /* --- enumeration -------------------------------------------------------- */
    size_t enumerated = 0;
    for (size_t index = 0; index < 16; ++index) {
        const char *name = redrob_gegl_adapter_operation_name(index);
        if (name == NULL) {
            break;
        }
        check(redrob_gegl_adapter_supports_operation(name),
              "an enumerated operation is a supported one");
        ++enumerated;
    }
    check(enumerated == REDROB_GEGL_SUPPORTED_OPERATION_COUNT,
          "enumeration yields exactly the supported count");
    check(redrob_gegl_adapter_operation_name(REDROB_GEGL_SUPPORTED_OPERATION_COUNT) == NULL,
          "enumeration stops at the end");

    /* --- the numbers, and they are not the obvious ones --------------------- */
    /* Input pixel 0 is (255, 128, 0) opaque; pixel 1 is (0, 64, 255) at half alpha.
     *
     * A naive 255-x inversion would give 0, 127, 255. GEGL gives 0, 229, 255,
     * measured, and that number is the whole reason this test is worth having:
     * 128 sRGB is 0.2159 in linear light (which is exactly what the babl adapter
     * next door asserts), inverting in linear gives 0.7841, and re-encoding that
     * to sRGB is 229. The three adapters are therefore arithmetically consistent
     * with each other -- babl states the transfer function, lcms2 reproduces it
     * through ICC, and GEGL's linear invert is that function applied, inverted,
     * and applied back. */
    uint8_t inverted[8] = {0};
    check(redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                         sizeof sample, 2, 1, inverted, sizeof inverted),
          "invert-linear applies");
    check_byte(inverted[0], 0, 1, "255 inverts to 0");
    check_byte(inverted[1], 229, 2, "128 inverts to 229 in linear light, not 127");
    check_byte(inverted[2], 255, 1, "0 inverts to 255");
    check_byte(inverted[3], 255, 0, "opaque alpha is preserved exactly");
    check_byte(inverted[5], 249, 2, "64 inverts to 249");
    check_byte(inverted[7], 128, 0, "half alpha is preserved exactly");

    /* Alpha preservation is asserted with tolerance ZERO. Unlike lcms2, GEGL
     * preserves it without being asked -- measured -- so any drift here is a real
     * regression rather than rounding. */

    /* gegl:invert produced byte-identical output to gegl:invert-linear on 0.4.70.
     * Asserted so that a future GEGL making them differ is a visible failure
     * rather than a silent change in what a caller gets. */
    uint8_t plain[8] = {0};
    check(redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT, sample, sizeof sample,
                                         2, 1, plain, sizeof plain),
          "invert applies");
    check(memcmp(plain, inverted, sizeof plain) == 0,
          "gegl:invert and gegl:invert-linear agree on this input");

    /* Grey: a luminance-weighted desaturation, so (255,128,0) becomes a single
     * grey level near 165 rather than the arithmetic mean of 128. */
    uint8_t grey[8] = {0};
    check(redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_GREY, sample, sizeof sample,
                                         2, 1, grey, sizeof grey),
          "grey applies");
    check_byte(grey[0], 165, 3, "grey of (255,128,0) is near 165");
    check(grey[0] == grey[1] && grey[1] == grey[2], "grey makes the channels equal");
    check_byte(grey[3], 255, 0, "grey preserves opaque alpha");
    check(grey[4] == grey[5] && grey[5] == grey[6], "grey makes the channels equal (pixel 1)");
    check_byte(grey[7], 128, 0, "grey preserves half alpha");

    /* --- repeatability ------------------------------------------------------ */
    uint8_t again[8] = {0};
    check(redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                         sizeof sample, 2, 1, again, sizeof again),
          "invert-linear applies a second time");
    check(memcmp(again, inverted, sizeof again) == 0,
          "a second call gives the same bytes -- no state leaks between graphs");

    /* The input must be unchanged: a filter preview may not destroy its source. */
    const uint8_t pristine[8] = {255, 128, 0, 255, 0, 64, 255, 128};
    check(memcmp(sample, pristine, sizeof sample) == 0, "the input buffer is untouched");

    /* --- a larger block, to exercise real strides -------------------------- */
    enum { WIDTH = 17, HEIGHT = 13, PIXELS = WIDTH * HEIGHT };
    uint8_t *big_in = malloc(PIXELS * 4);
    uint8_t *big_out = malloc(PIXELS * 4);
    check(big_in != NULL && big_out != NULL, "the large fixture allocated");
    if (big_in != NULL && big_out != NULL) {
        for (size_t index = 0; index < PIXELS; ++index) {
            big_in[index * 4 + 0] = (uint8_t)(index % 256);
            big_in[index * 4 + 1] = 128;
            big_in[index * 4 + 2] = (uint8_t)(255 - index % 256);
            big_in[index * 4 + 3] = (uint8_t)(index % 200 + 55);
        }
        memset(big_out, 0, PIXELS * 4);
        check(redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, big_in,
                                             PIXELS * 4, WIDTH, HEIGHT, big_out, PIXELS * 4),
              "a 17x13 block applies (odd dimensions, so no lucky tiling)");
        /* Every green channel was 128, so every one must now be 229 -- a per-pixel
         * assertion over 221 pixels rather than a spot check on the first. */
        size_t wrong_green = 0;
        size_t wrong_alpha = 0;
        for (size_t index = 0; index < PIXELS; ++index) {
            int delta = (int)big_out[index * 4 + 1] - 229;
            if (delta < 0) {
                delta = -delta;
            }
            if (delta > 2) {
                ++wrong_green;
            }
            if (big_out[index * 4 + 3] != big_in[index * 4 + 3]) {
                ++wrong_alpha;
            }
        }
        if (wrong_green != 0) {
            fprintf(stderr, "FAIL: %zu of %d pixels did not invert 128 to 229\n",
                    wrong_green, PIXELS);
            ++failures;
        }
        if (wrong_alpha != 0) {
            fprintf(stderr, "FAIL: %zu of %d pixels lost their alpha\n", wrong_alpha, PIXELS);
            ++failures;
        }
    }
    free(big_in);
    free(big_out);

    /* --- refusals, and each must write nothing ------------------------------ */
    uint8_t guard[8] = {11, 22, 33, 44, 55, 66, 77, 88};
    const uint8_t guard_copy[8] = {11, 22, 33, 44, 55, 66, 77, 88};

    check(!redrob_gegl_adapter_apply_rgba("gegl:gaussian-blur", sample, sizeof sample, 2, 1,
                                          guard, sizeof guard),
          "an operation GEGL has but we do not offer is refused");
    check(!redrob_gegl_adapter_apply_rgba("gegl:not-a-real-op", sample, sizeof sample, 2, 1,
                                          guard, sizeof guard),
          "an invented operation is refused");
    check(!redrob_gegl_adapter_apply_rgba(NULL, sample, sizeof sample, 2, 1, guard,
                                          sizeof guard),
          "a NULL operation is refused");
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, NULL,
                                          sizeof sample, 2, 1, guard, sizeof guard),
          "a NULL input is refused");
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                          sizeof sample, 0, 1, guard, sizeof guard),
          "zero width is refused");
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                          sizeof sample, 2, 0, guard, sizeof guard),
          "zero height is refused");
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                          sizeof sample, 3, 1, guard, sizeof guard),
          "a width past the input buffer is refused");
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                          sizeof sample, 2, 1, guard, 4),
          "an output buffer too small is refused");
    check(memcmp(guard, guard_copy, sizeof guard) == 0,
          "not one refusal wrote to the output buffer");

    /* --- shutdown, twice ---------------------------------------------------- */
    redrob_gegl_adapter_shutdown();
    redrob_gegl_adapter_shutdown();
    RedrobGeglAdapterCapabilities closed = redrob_gegl_adapter_capabilities();
    check(!closed.initialized, "initialized is false after shutdown");
    check(!closed.ready, "ready is false after shutdown");
    check(closed.operation_count == 0, "no operations are claimed after shutdown");
    check(closed.catalog_operation_count == 0, "no catalogue after shutdown");
    check(!redrob_gegl_adapter_apply_rgba(REDROB_GEGL_OPERATION_INVERT_LINEAR, sample,
                                          sizeof sample, 2, 1, guard, sizeof guard),
          "apply refuses after shutdown");

    if (failures != 0) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return EXIT_FAILURE;
    }
    printf("redrob_gegl_adapter: all checks passed\n");
    return EXIT_SUCCESS;
}
