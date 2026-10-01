/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_babl_adapter.h"

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

int main(void)
{
    /* --- before initialize: compiled, but nothing is ready ------------------ */
    RedrobBablAdapterCapabilities before = redrob_babl_adapter_capabilities();
    check(before.compiled, "compiled is true when the adapter is built");
    check(!before.initialized, "initialized is false before initialize");
    check(!before.ready, "ready is false before initialize");
    check(before.required_format_count == 0, "no formats resolved before initialize");

    /* Conversion must refuse before initialize rather than reach into an
     * uninitialised babl. */
    const uint8_t probe_in[4] = {255, 128, 0, 255};
    float probe_out[4] = {0};
    check(!redrob_babl_adapter_srgb_u8_to_linear_f32(probe_in, sizeof probe_in, probe_out,
                                                    sizeof probe_out, 1),
          "convert refuses before initialize");
    check(redrob_babl_adapter_bytes_per_pixel(REDROB_BABL_FORMAT_SRGB_U8) == 0,
          "bytes_per_pixel refuses before initialize");

    /* --- initialize --------------------------------------------------------- */
    check(redrob_babl_adapter_initialize(), "initialize reports ready");

    RedrobBablAdapterCapabilities after = redrob_babl_adapter_capabilities();
    check(after.initialized, "initialized is true after initialize");
    check(after.ready, "ready is true after initialize");
    check(after.required_format_count == REDROB_BABL_REQUIRED_FORMAT_COUNT,
          "every required format resolved");

    /* --- strides, because a format name and its stride are one fact -------- */
    check(redrob_babl_adapter_bytes_per_pixel(REDROB_BABL_FORMAT_SRGB_U8) == 4, "sRGB u8 is 4 bytes");
    check(redrob_babl_adapter_bytes_per_pixel(REDROB_BABL_FORMAT_LINEAR_F32) == 16,
          "linear f32 is 16 bytes");
    check(redrob_babl_adapter_bytes_per_pixel(REDROB_BABL_FORMAT_SRGB_F32) == 16,
          "sRGB f32 is 16 bytes");
    check(redrob_babl_adapter_bytes_per_pixel(REDROB_BABL_FORMAT_LINEAR_U16) == 8,
          "linear u16 is 8 bytes");
    check(redrob_babl_adapter_bytes_per_pixel(REDROB_BABL_FORMAT_GRAY_U8) == 1, "gray u8 is 1 byte");
    check(redrob_babl_adapter_bytes_per_pixel("not a real format") == 0,
          "an unknown format has no stride");

    /* --- the numbers, which are the whole point ---------------------------- */
    /* Two pixels: opaque orange, and half-alpha blue.
     *
     * The sRGB electro-optical transfer function gives, for 128/255 = 0.501961:
     *   ((0.501961 + 0.055) / 1.055) ^ 2.4 = 0.21586
     * That constant is what makes this a real check rather than a smoke test: a
     * linear divide by 255 would put 0.50196 here, and blending on that value is
     * the bug this adapter exists to prevent -- 50% between black and white
     * reads as 188, not 128.
     *
     * Alpha is NOT transfer-encoded, so 128/255 stays 0.501961. Getting that
     * wrong is the other half of the same mistake, and it is why alpha is
     * asserted separately below. */
    const uint8_t srgb[8] = {255, 128, 0, 255, 0, 64, 255, 128};
    float linear[8] = {0};
    check(redrob_babl_adapter_srgb_u8_to_linear_f32(srgb, sizeof srgb, linear, sizeof linear, 2),
          "sRGB u8 to linear f32 succeeds");

    check_close(linear[0], 1.0f, 1e-4f, "255 sRGB is 1.0 linear");
    check_close(linear[1], 0.21586f, 1e-3f, "128 sRGB is 0.2159 linear, not 0.5020");
    check_close(linear[2], 0.0f, 1e-4f, "0 sRGB is 0.0 linear");
    check_close(linear[3], 1.0f, 1e-4f, "opaque alpha is 1.0");
    check_close(linear[5], 0.05126f, 1e-3f, "64 sRGB is 0.0513 linear");
    check_close(linear[6], 1.0f, 1e-4f, "255 sRGB is 1.0 linear (second pixel)");
    check_close(linear[7], 0.50196f, 1e-4f, "alpha is linear: 128/255 stays 0.5020");

    /* --- round trip, which catches a wrong transfer in either direction ----- */
    uint8_t back[8] = {0};
    check(redrob_babl_adapter_linear_f32_to_srgb_u8(linear, sizeof linear, back, sizeof back, 2),
          "linear f32 back to sRGB u8 succeeds");
    for (size_t index = 0; index < 8; ++index) {
        if (back[index] != srgb[index]) {
            fprintf(stderr, "FAIL: round trip byte %zu is %u, expected %u\n",
                    index, (unsigned)back[index], (unsigned)srgb[index]);
            ++failures;
        }
    }

    /* A whole run of every 8-bit value, because a transfer function can be right
     * at the three endpoints a hand-picked sample happens to use and wrong in the
     * middle. */
    uint8_t ramp[256 * 4];
    for (size_t value = 0; value < 256; ++value) {
        ramp[value * 4 + 0] = (uint8_t)value;
        ramp[value * 4 + 1] = (uint8_t)value;
        ramp[value * 4 + 2] = (uint8_t)value;
        ramp[value * 4 + 3] = 255;
    }
    float ramp_linear[256 * 4];
    uint8_t ramp_back[256 * 4];
    check(redrob_babl_adapter_srgb_u8_to_linear_f32(ramp, sizeof ramp, ramp_linear,
                                                    sizeof ramp_linear, 256),
          "full 8-bit ramp converts to linear");
    check(redrob_babl_adapter_linear_f32_to_srgb_u8(ramp_linear, sizeof ramp_linear, ramp_back,
                                                    sizeof ramp_back, 256),
          "full 8-bit ramp converts back");
    size_t ramp_mismatches = 0;
    for (size_t value = 0; value < 256 * 4; ++value) {
        if (ramp_back[value] != ramp[value]) {
            ++ramp_mismatches;
        }
    }
    if (ramp_mismatches != 0) {
        fprintf(stderr, "FAIL: %zu of %d ramp bytes did not survive the round trip\n",
                ramp_mismatches, 256 * 4);
        ++failures;
    }
    /* And the ramp must be monotonic: a transfer function that is not increasing
     * everywhere would make a gradient band or reverse. */
    for (size_t value = 1; value < 256; ++value) {
        if (ramp_linear[value * 4] < ramp_linear[(value - 1) * 4]) {
            fprintf(stderr, "FAIL: linear ramp decreases at %zu\n", value);
            ++failures;
            break;
        }
    }

    /* --- grayscale, which masks use ---------------------------------------- */
    const uint8_t gray_in[2] = {128, 255};
    float gray_out[2] = {0};
    check(redrob_babl_adapter_convert(REDROB_BABL_FORMAT_GRAY_U8, "Y float", gray_in,
                                      sizeof gray_in, gray_out, sizeof gray_out, 2),
          "gray u8 to linear gray float succeeds");
    check_close(gray_out[0], 0.21586f, 1e-3f, "gray 128 is 0.2159 linear");

    /* --- refusals ----------------------------------------------------------- */
    check(!redrob_babl_adapter_convert(NULL, REDROB_BABL_FORMAT_LINEAR_F32, srgb, sizeof srgb,
                                       linear, sizeof linear, 2),
          "a NULL source format is refused");
    check(!redrob_babl_adapter_convert(REDROB_BABL_FORMAT_SRGB_U8, "utter nonsense", srgb,
                                       sizeof srgb, linear, sizeof linear, 2),
          "an unknown destination format is refused");
    check(!redrob_babl_adapter_convert(REDROB_BABL_FORMAT_SRGB_U8, REDROB_BABL_FORMAT_LINEAR_F32,
                                       NULL, sizeof srgb, linear, sizeof linear, 2),
          "a NULL input buffer is refused");
    check(!redrob_babl_adapter_srgb_u8_to_linear_f32(srgb, sizeof srgb, linear, sizeof linear, 3),
          "a pixel count past the input buffer is refused");
    check(!redrob_babl_adapter_srgb_u8_to_linear_f32(srgb, sizeof srgb, linear, 4, 2),
          "an output buffer too small for the destination stride is refused");
    check(redrob_babl_adapter_srgb_u8_to_linear_f32(srgb, sizeof srgb, linear, sizeof linear, 0),
          "zero pixels is a successful no-op");

    /* The short-output case matters more than it looks: 8 input bytes are two
     * sRGB pixels but only half a linear pixel of output, so a check that sized
     * both sides by the SOURCE stride would pass it and write 32 bytes into a
     * 16-byte buffer. */
    uint8_t narrow[4] = {0};
    check(!redrob_babl_adapter_linear_f32_to_srgb_u8(linear, sizeof linear, narrow, sizeof narrow, 2),
          "destination bounds are checked against the destination stride");

    /* --- shutdown ----------------------------------------------------------- */
    redrob_babl_adapter_shutdown();
    RedrobBablAdapterCapabilities closed = redrob_babl_adapter_capabilities();
    check(!closed.initialized, "initialized is false after shutdown");
    check(!closed.ready, "ready is false after shutdown");
    check(closed.required_format_count == 0, "no formats are claimed after shutdown");
    check(!redrob_babl_adapter_srgb_u8_to_linear_f32(probe_in, sizeof probe_in, probe_out,
                                                    sizeof probe_out, 1),
          "convert refuses after shutdown");

    if (failures != 0) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return EXIT_FAILURE;
    }
    printf("redrob_babl_adapter: all checks passed\n");
    return EXIT_SUCCESS;
}
