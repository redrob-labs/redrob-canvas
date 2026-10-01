// Krita's circle mask generator, run with its own arithmetic, to produce reference values for the Rust
// translation and to pin down the edge cases the formula hides.
//
// The class hierarchy, Qt types and the fade-maker template are replaced by the equivalent scalars; every
// line of the value computation and the coefficient setup is Krita's.
//
// Note Krita's mask convention: 0 is FULLY OPAQUE and 255 is fully transparent. That is the opposite of
// what a coverage value normally means and is the first thing a translation can get backwards.

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <algorithm>

static inline double norme(double a, double b) { return a * a + b * b; }

class CircleMask {
public:
    // diameter, ratio, fade horizontal/vertical, spikes are Krita's constructor arguments.
    CircleMask(double diameter, double ratio, double fh, double fv, bool antialiasEdges)
        : m_diameter(diameter), m_ratio(ratio), m_fh(fh), m_fv(fv), m_antialias(antialiasEdges) {
        setSoftness(1.0);
    }

    void setSoftness(double softness) {
        m_safeSoftnessCoeff = 1.0 / std::max(0.01, softness);
        recompute();
    }

    void recompute() {
        const double width = effectiveSrcWidth();
        const double height = effectiveSrcHeight();
        m_xcoef = 2.0 / width;
        m_ycoef = 2.0 / height;
        m_xfadecoef = (m_fh == 0.0) ? 1.0 : (2.0 / (m_fh * width));
        m_yfadecoef = (m_fv == 0.0) ? 1.0 : (2.0 / (m_fv * height));
        m_transformedFadeX = m_xfadecoef * m_safeSoftnessCoeff;
        m_transformedFadeY = m_yfadecoef * m_safeSoftnessCoeff;
    }

    double effectiveSrcWidth() const { return m_diameter; }
    double effectiveSrcHeight() const { return m_diameter * m_ratio; }
    bool isEmpty() const { return m_diameter <= 0.0; }

    uint8_t valueAt(double x, double y) const {
        if (isEmpty()) return 255;
        double xr = x;
        double yr = std::fabs(y);
        // fixRotation is identity at zero rotation, which is the only case translated.
        double n = norme(xr * m_xcoef, yr * m_ycoef);
        if (n > 1.0) return 255;

        if (m_antialias) {
            xr = std::fabs(xr) + 1.0;
            yr = std::fabs(yr) + 1.0;
        }

        double nf = norme(xr * m_transformedFadeX, yr * m_transformedFadeY);
        if (nf < 1.0) return 0;
        return (uint8_t)(255 * n * (nf - 1.0) / (nf - n));
    }

private:
    double m_diameter, m_ratio, m_fh, m_fv;
    bool m_antialias;
    double m_xcoef = 0, m_ycoef = 0, m_xfadecoef = 0, m_yfadecoef = 0;
    double m_safeSoftnessCoeff = 1, m_transformedFadeX = 0, m_transformedFadeY = 0;
};

static void row(const char *label, const CircleMask &m, double diameter) {
    printf("  %-34s", label);
    // Sample along the x axis from centre to past the edge.
    for (double frac : {0.0, 0.25, 0.5, 0.75, 0.9, 1.0, 1.1}) {
        printf(" %3d", (int)m.valueAt(frac * diameter * 0.5, 0.0));
    }
    printf("\n");
}

int main() {
    printf("샘플 위치: 중심에서 반지름의 0, 0.25, 0.5, 0.75, 0.9, 1.0, 1.1 배 (x축)\n");
    printf("Krita 규약: 0 = 완전 불투명, 255 = 완전 투명\n\n");

    const double d = 40.0;

    printf("=== fade 를 바꾼다 (softness 1.0, antialias off)\n");
    for (double fade : {0.0, 0.25, 0.5, 0.75, 1.0}) {
        char label[64];
        snprintf(label, sizeof label, "fade=%.2f", fade);
        CircleMask m(d, 1.0, fade, fade, false);
        row(label, m, d);
    }

    printf("\n=== softness 를 바꾼다 (fade 0.5, antialias off)\n");
    for (double softness : {0.1, 0.5, 1.0, 2.0}) {
        char label[64];
        snprintf(label, sizeof label, "softness=%.2f", softness);
        CircleMask m(d, 1.0, 0.5, 0.5, false);
        m.setSoftness(softness);
        row(label, m, d);
    }

    printf("\n=== antialias 가 켜지면\n");
    { CircleMask m(d, 1.0, 0.5, 0.5, true);  row("fade=0.5 antialias=on",  m, d); }
    { CircleMask m(d, 1.0, 0.5, 0.5, false); row("fade=0.5 antialias=off", m, d); }

    printf("\n=== 비율(타원)\n");
    for (double ratio : {0.5, 1.0, 2.0}) {
        char label[64];
        snprintf(label, sizeof label, "ratio=%.2f (x축)", ratio);
        CircleMask m(d, ratio, 0.5, 0.5, false);
        row(label, m, d);
    }
    printf("  y축 값 (ratio 0.5, fade 0.5):");
    { CircleMask m(d, 0.5, 0.5, 0.5, false);
      for (double frac : {0.0, 0.25, 0.5, 0.75, 0.9, 1.0, 1.1})
          printf(" %3d", (int)m.valueAt(0.0, frac * d * 0.5 * 0.5));
      printf("\n"); }

    printf("\n=== 경계 사례\n");
    { CircleMask m(0.0, 1.0, 0.5, 0.5, false);
      printf("  지름 0 -> valueAt(0,0) = %d (빈 마스크는 255)\n", (int)m.valueAt(0, 0)); }
    { CircleMask m(d, 1.0, 0.0, 0.0, false);
      printf("  fade 0 -> 중심 %d, 0.9R %d, 1.0R %d\n",
             (int)m.valueAt(0,0), (int)m.valueAt(0.9*d*0.5,0), (int)m.valueAt(d*0.5,0)); }
    { CircleMask m(d, 1.0, 1.0, 1.0, false);
      printf("  fade 1 -> 중심 %d, 0.5R %d, 0.99R %d  (nf==n 이면 0 을 돌려준다)\n",
             (int)m.valueAt(0,0), (int)m.valueAt(0.5*d*0.5,0), (int)m.valueAt(0.99*d*0.5,0)); }

    printf("\n=== 정확한 값 (fade=0.5, softness=1, d=40, 소수 없음)\n");
    { CircleMask m(d, 1.0, 0.5, 0.5, false);
      for (int px = 0; px <= 21; ++px)
          printf("  x=%2d  value=%3d\n", px, (int)m.valueAt((double)px, 0.0)); }
    printf("\n=== hardness(fade) 1.0 에서 antialias 가 켜지면 정말 딱딱한가\n");
    { CircleMask on(d, 1.0, 1.0, 1.0, true), off(d, 1.0, 1.0, 1.0, false);
      for (double x : {0.0, 10.0, 15.0, 18.0, 19.0, 19.5, 20.0})
          printf("  x=%5.1f  antialias_on=%3d  antialias_off=%3d\n",
                 x, (int)on.valueAt(x,0.0), (int)off.valueAt(x,0.0)); }
    return 0;
}
