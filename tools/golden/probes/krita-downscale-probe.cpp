// Krita's scale-aware downscale weights, run with its own fixed-point arithmetic, to produce reference values
// for the Rust translation in `sample_rgba_filtered`.
//
// From libs/image/kis_filter_weights_buffer.h, kis_filter_strategy.cc and kis_fixed_point_maths.h at the commit
// pinned in docs/upstream-sources.toml, GPL-2.0-or-later. boost::ordered_field_operators and QDebug are
// dropped; every line of the weight construction, the support widening and the normalisation is Krita's.
//
// WHAT THIS PINS. A plain bilinear sample reads four texels whatever the scale factor, so shrinking reads one
// phase of the source and discards the rest. Krita widens the filter's support in SOURCE space by 1/scale while
// evaluating its weights in DESTINATION space, then normalises the table to sum to 255. This probe prints those
// tables so the translation is checked against them rather than against a description of them.

#include <cstdio>
#include <cstdint>
#include <cmath>
#include <vector>

// ---------------------------------------------------------------- KisFixedPoint, 8-bit fraction
class KisFixedPoint {
public:
    KisFixedPoint() : d(0) {}
    KisFixedPoint(int iValue) : d(iValue * 256) {}
    KisFixedPoint(double fValue) : d(static_cast<int>(fValue * 256)) {}

    int32_t toInt() const { return d >= 0 ? d >> 8 : -((-d) >> 8); }
    int32_t toIntCeil() const { return d >= 0 ? (d + ((1 << 8) - 1)) >> 8 : -((-d) >> 8); }
    double toFloat() const { return double(d) / double(1 << 8); }
    KisFixedPoint &from256Frac(int32_t v) { d = v; return *this; }
    int32_t to256Frac() const { return d; }
    bool isInteger() const { return !(d & ((1 << 8) - 1)); }

    KisFixedPoint &operator+=(const KisFixedPoint &x) { d += x.d; return *this; }
    KisFixedPoint &operator-=(const KisFixedPoint &x) { d -= x.d; return *this; }
    KisFixedPoint &operator*=(const KisFixedPoint &x) {
        int64_t t = (int64_t)d * (int64_t)x.d;
        d = (int32_t)(t >> 8);
        return *this;
    }
    KisFixedPoint &operator/=(const KisFixedPoint &x) {
        int64_t t = ((int64_t)d << 8);
        d = (int32_t)(t / (int64_t)x.d);
        return *this;
    }
    friend KisFixedPoint operator+(KisFixedPoint a, const KisFixedPoint &b) { a += b; return a; }
    friend KisFixedPoint operator-(KisFixedPoint a, const KisFixedPoint &b) { a -= b; return a; }
    friend KisFixedPoint operator*(KisFixedPoint a, const KisFixedPoint &b) { a *= b; return a; }
    friend KisFixedPoint operator/(KisFixedPoint a, const KisFixedPoint &b) { a /= b; return a; }
    friend KisFixedPoint operator-(KisFixedPoint x) { x.d = -x.d; return x; }

    int32_t d;
};

// ---------------------------------------------------------------- KisBilinearFilterStrategy
//
// The triangle 1 - |t|, support 1.0. Its `weightsPositionScale` argument is DELIBERATELY UNUSED: the scaling is
// applied to the POSITION before the weight is evaluated, which the weights buffer below does.
struct KisBilinearFilterStrategy {
    static constexpr double supportVal = 1.0;
    static constexpr int32_t intSupportVal = 256;

    int32_t intValueAt(int32_t t, double /*weightsPositionScale*/) const {
        if (t < 0) t = -t;
        if (t < 256) {
            // Krita's own comment: calc 256-1 but also go from .8 fixed point to 8bit scale.
            if (t >= 128) return 256 - t;
            return 255 - t;
        }
        return 0;
    }
    int32_t intSupport(double /*weightsPositionScale*/) const {
        // The BASE class's implementation, which bilinear does not override: `Q_UNUSED(weightsPositionScale)`
        // and return intSupportVal flat. Only KisBoxFilterStrategy overrides it to scale the support.
        //
        // My first version of this probe used the BOX override -- ceil(intSupportVal * scale) -- and the
        // difference is not cosmetic: it made the support at quarter scale come out 1.0 instead of 4.0, the
        // span 3 instead of 9, and a checkerboard 102 instead of 128. That would have recorded Krita as
        // aliasing where it does not, and would have made this product's correct translation look like a
        // departure. Checked in kis_filter_strategy.h: bilinear declares supportVal 1.0 / intSupportVal 256
        // and overrides only valueAt and intValueAt.
        return intSupportVal;
    }
};

// ---------------------------------------------------------------- KisFilterWeightsBuffer
struct FilterWeights {
    std::vector<int16_t> weight;
    int centerIndex = 0;
    int span = 0;
};

struct KisFilterWeightsBuffer {
    KisFilterWeightsBuffer(const KisBilinearFilterStrategy &filterStrategy, double realScale)
    {
        weights.resize(256);
        maxSpan = 0;
        weightsPositionScale = KisFixedPoint(1);

        KisFixedPoint supportSrc;
        KisFixedPoint supportDst;

        // The widening, and the bound on it: past a 1/256 scale the support would cover the whole image for
        // every destination pixel.
        if (realScale < 1.0 && realScale > (1.0 / (1 << 8))) {
            weightsPositionScale = KisFixedPoint(realScale);
            supportSrc.from256Frac(
                (int32_t)(filterStrategy.intSupport(weightsPositionScale.toFloat()) / realScale));
            supportDst.from256Frac(filterStrategy.intSupport(weightsPositionScale.toFloat()));
        } else {
            supportSrc.from256Frac(filterStrategy.intSupport(weightsPositionScale.toFloat()));
            supportDst.from256Frac(filterStrategy.intSupport(weightsPositionScale.toFloat()));
        }

        for (int i = 0; i < 256; i++) {
            KisFixedPoint centerSrc;
            centerSrc.from256Frac(i);

            KisFixedPoint beginSrc = -supportSrc - centerSrc / weightsPositionScale;
            KisFixedPoint endSrc = supportSrc - centerSrc / weightsPositionScale;

            int span = (KisFixedPoint(2) * supportSrc).toInt() +
                       (beginSrc.isInteger() && endSrc.isInteger());
            int centerIndex = -beginSrc.toInt();

            weights[i].centerIndex = centerIndex;
            weights[i].span = span;
            weights[i].weight.assign((size_t)(span > 0 ? span : 0), 0);
            if (span > maxSpan) maxSpan = span;

            // In destination coordinates.
            KisFixedPoint scaledIter = centerSrc + KisFixedPoint(beginSrc.toInt()) * weightsPositionScale;
            KisFixedPoint scaledInc = weightsPositionScale;

            int sum = 0;
            for (int j = 0; j < span; j++) {
                int t = filterStrategy.intValueAt(scaledIter.to256Frac(), weightsPositionScale.toFloat());
                weights[i].weight[(size_t)j] = (int16_t)t;
                sum += t;
                scaledIter += scaledInc;
            }

            // Krita normalises the table to sum to 255. A triangle over a widened support does not.
            if (sum != 255 && sum > 0) {
                double fixFactor = 255.0 / sum;
                sum = 0;
                for (int j = 0; j < span; j++) {
                    int t = (int)std::lround(weights[i].weight[(size_t)j] * fixFactor);
                    weights[i].weight[(size_t)j] = (int16_t)t;
                    sum += t;
                }
            }
            sums[i] = sum;
        }
    }

    std::vector<FilterWeights> weights;
    KisFixedPoint weightsPositionScale;
    int maxSpan = 0;
    int sums[256] = {0};
};

static void report(double scale)
{
    KisBilinearFilterStrategy strategy;
    KisFilterWeightsBuffer buffer(strategy, scale);
    printf("=== scale %.4f\n", scale);
    printf("    weightsPositionScale %.6f   maxSpan %d\n",
           buffer.weightsPositionScale.toFloat(), buffer.maxSpan);
    // Subpixel offset 0 is the aligned case and the one a translation is most likely to get right by
    // accident; 128 is the half-pixel case that separates a real filter from a point sample.
    for (int offset : {0, 64, 128}) {
        const FilterWeights &w = buffer.weights[(size_t)offset];
        printf("    offset %3d: span %2d centerIndex %2d sum %3d weights",
               offset, w.span, w.centerIndex, buffer.sums[offset]);
        for (int j = 0; j < w.span; j++) printf(" %d", (int)w.weight[(size_t)j]);
        printf("\n");
    }
    printf("\n");
}

// What a one-pixel checkerboard row becomes, which is the measurement the Rust asserts.
static void checkerboard(double scale, int destination_pixels)
{
    KisBilinearFilterStrategy strategy;
    KisFilterWeightsBuffer buffer(strategy, scale);
    printf("=== a 1px checkerboard row at scale %.4f, %d destination pixels\n", scale, destination_pixels);
    printf("    source is 255,0,255,0,... so the area average is 128\n");
    printf("    destination:");
    for (int d = 0; d < destination_pixels; d++) {
        // Krita's applicator steps the source position by 1/scale per destination pixel and uses the weight
        // table indexed by the subpixel part.
        double source_centre = d / scale;
        int subpixel = (int)std::lround((source_centre - std::floor(source_centre)) * 256.0) & 0xff;
        const FilterWeights &w = buffer.weights[(size_t)subpixel];
        int first = (int)std::floor(source_centre) - w.centerIndex;
        int accumulated = 0;
        for (int j = 0; j < w.span; j++) {
            int source_index = first + j;
            int value = (source_index >= 0 && (source_index % 2) == 0) ? 255 : 0;
            accumulated += value * w.weight[(size_t)j];
        }
        printf(" %d", (accumulated + 127) / 255);
    }
    printf("\n\n");
}

int main()
{
    printf("Krita's scale-aware downscale weights, its own fixed-point arithmetic\n");
    printf("from libs/image/kis_filter_weights_buffer.h and kis_filter_strategy.cc\n");
    printf("Krita's bilinear strategy is the triangle 1-|t| and IGNORES its scale argument:\n");
    printf("the scaling is applied to the POSITION before the weight is evaluated.\n\n");

    // Upscaling and unit scale must NOT widen: the support stays 1.0 and the span stays small.
    report(2.0);
    report(1.0);
    // Downscaling widens by 1/scale.
    report(0.5);
    report(0.25);
    // Krita's own bound: at or below 1/256 the widening stops.
    report(1.0 / 256.0);

    checkerboard(0.25, 8);
    checkerboard(0.5, 8);

    return 0;
}
