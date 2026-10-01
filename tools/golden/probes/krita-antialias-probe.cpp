// Does Krita's antialiasEdges soften the border even at full hardness?
//
// My translation says yes, because the +1.0 is applied to BOTH coordinates, so y becomes 1.0 even when
// the sample sits exactly on the x axis. That is a claim about Krita's arithmetic, so it gets checked
// against Krita's arithmetic rather than argued.

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <algorithm>

static inline double norme(double a, double b) { return a * a + b * b; }

class CircleMask {
public:
    CircleMask(double diameter, double ratio, double fh, double fv, bool antialiasEdges)
        : m_diameter(diameter), m_ratio(ratio), m_fh(fh), m_fv(fv), m_antialias(antialiasEdges) {
        setSoftness(1.0);
    }
    void setSoftness(double softness) {
        m_safeSoftnessCoeff = 1.0 / std::max(0.01, softness);
        const double width = m_diameter, height = m_diameter * m_ratio;
        m_xcoef = 2.0 / width;
        m_ycoef = 2.0 / height;
        m_xfadecoef = (m_fh == 0.0) ? 1.0 : (2.0 / (m_fh * width));
        m_yfadecoef = (m_fv == 0.0) ? 1.0 : (2.0 / (m_fv * height));
        m_transformedFadeX = m_xfadecoef * m_safeSoftnessCoeff;
        m_transformedFadeY = m_yfadecoef * m_safeSoftnessCoeff;
    }
    uint8_t valueAt(double x, double y) const {
        if (m_diameter <= 0.0) return 255;
        double xr = x, yr = std::fabs(y);
        double n = norme(xr * m_xcoef, yr * m_ycoef);
        if (n > 1.0) return 255;
        if (m_antialias) { xr = std::fabs(xr) + 1.0; yr = std::fabs(yr) + 1.0; }
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

int main() {
    const double d = 40.0;
    printf("=== fade 1.0 (최대 경도), antialias on 대 off\n");
    CircleMask on(d, 1.0, 1.0, 1.0, true), off(d, 1.0, 1.0, 1.0, false);
    for (double x : {0.0, 10.0, 15.0, 18.0, 19.0, 19.5, 20.0}) {
        printf("  x=%5.1f  on=%3d  off=%3d\n", x, (int)on.valueAt(x, 0.0), (int)off.valueAt(x, 0.0));
    }
    printf("\n=== 같은 것을 커버리지로 (1 - v/255)\n");
    for (double x : {18.0, 19.0, 19.5}) {
        printf("  x=%5.1f  on=%.6f  off=%.6f\n", x,
               1.0 - on.valueAt(x, 0.0) / 255.0, 1.0 - off.valueAt(x, 0.0) / 255.0);
    }
    printf("\n=== y 에도 +1 이 붙는지: y=0 샘플의 nf 기여\n");
    printf("  antialias 가 y=0 을 y=1 로 만들면 fade 1.0 에서도 마지막 픽셀이 부드러워진다.\n");
    printf("  x=19 on=%d -> %s\n", (int)on.valueAt(19.0, 0.0),
           on.valueAt(19.0, 0.0) > 0 ? "그렇다, 부드러워진다" : "아니다, 딱딱하다");
    return 0;
}
