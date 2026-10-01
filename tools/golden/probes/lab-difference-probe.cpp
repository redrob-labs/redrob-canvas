// Does a closed-form sRGB -> Lab conversion reproduce what Krita gets through lcms2?
//
// Krita's fill tolerance is `differenceA`, which transforms both pixels to Lab16 with the document's ICC
// profile and returns sqrt(dL^2 + da^2 + db^2 + dAlpha^2), truncated to a byte. This product's Pixel is
// fixed sRGB 8-bit with no profile, and redrob-core cannot depend on lcms2 -- the lcms adapter is an
// optional C shim for the Qt layer. So the conversion has to be closed-form, and closed-form is only
// honest if it matches.
//
// This runs Krita's path through real lcms2 and the closed form beside it, over a sweep.

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <algorithm>
#include <lcms2.h>

// ---- Krita's differenceA, with the sRGB -> Lab16 transform lcms2 builds.
static cmsHTRANSFORM to_lab16 = nullptr;

static void krita_to_lab(const uint8_t *rgba, uint16_t out[4]) {
    // Krita's LabA16 buffer is four u16: three Lab channels plus alpha. lcms2's TYPE_Lab_16 carries only
    // the three, so alpha is scaled through separately -- which is what Krita's own colour space does with
    // its appended alpha channel. Reading a fourth channel lcms2 never wrote is how the first version of
    // this probe reported a non-zero difference between a colour and ITSELF.
    cmsDoTransform(to_lab16, rgba, out, 1);
    out[3] = (uint16_t)(rgba[3] * 257);
}

static uint8_t krita_difference_a(const uint8_t *p1, const uint8_t *p2) {
    if (p1[3] == 0 || p2[3] == 0) {
        const double alphaScale = 100.0 / 255.0;
        return (uint8_t)lround(alphaScale * std::abs((int)p1[3] - (int)p2[3]));
    }
    uint16_t lab1[4], lab2[4];
    krita_to_lab(p1, lab1);
    krita_to_lab(p2, lab2);
    cmsCIELab f1, f2;
    cmsLabEncoded2Float(&f1, lab1);
    cmsLabEncoded2Float(&f2, lab2);
    const double dL = std::fabs(f1.L - f2.L);
    const double da = std::fabs(f1.a - f2.a);
    const double db = std::fabs(f1.b - f2.b);
    const double alphaScale = 100.0 / 65535.0;
    const double dAlpha = std::fabs((double)lab1[3] - (double)lab2[3]) * alphaScale;
    const double diff = std::pow(dL * dL + da * da + db * db + dAlpha * dAlpha, 0.5);
    return diff > 255.0 ? 255 : (uint8_t)diff;
}

// ---- The closed form, which is what the Rust will do.
static double srgb_to_linear(double c) {
    return c <= 0.04045 ? c / 12.92 : std::pow((c + 0.055) / 1.055, 2.4);
}

static double lab_f(double t) {
    const double epsilon = 216.0 / 24389.0;
    const double kappa = 24389.0 / 27.0;
    return t > epsilon ? std::cbrt(t) : (kappa * t + 16.0) / 116.0;
}

static void closed_form_lab(const uint8_t *rgba, double out[3], double white[3]) {
    const double r = srgb_to_linear(rgba[0] / 255.0);
    const double g = srgb_to_linear(rgba[1] / 255.0);
    const double b = srgb_to_linear(rgba[2] / 255.0);
    // sRGB -> XYZ, then adapted to D50, which is what an ICC Lab PCS uses.
    // Bradford-adapted sRGB D65 -> D50 matrix, the one ICC profiles carry.
    const double X = 0.4360747 * r + 0.3850649 * g + 0.1430804 * b;
    const double Y = 0.2225045 * r + 0.7168786 * g + 0.0606169 * b;
    const double Z = 0.0139322 * r + 0.0971045 * g + 0.7141733 * b;
    const double fx = lab_f(X / white[0]);
    const double fy = lab_f(Y / white[1]);
    const double fz = lab_f(Z / white[2]);
    out[0] = 116.0 * fy - 16.0;
    out[1] = 500.0 * (fx - fy);
    out[2] = 200.0 * (fy - fz);
}

static uint8_t closed_form_difference(const uint8_t *p1, const uint8_t *p2) {
    if (p1[3] == 0 || p2[3] == 0) {
        const double alphaScale = 100.0 / 255.0;
        return (uint8_t)lround(alphaScale * std::abs((int)p1[3] - (int)p2[3]));
    }
    double white[3] = {0.964212, 1.0, 0.825188};  // D50
    double lab1[3], lab2[3];
    closed_form_lab(p1, lab1, white);
    closed_form_lab(p2, lab2, white);
    const double dL = std::fabs(lab1[0] - lab2[0]);
    const double da = std::fabs(lab1[1] - lab2[1]);
    const double db = std::fabs(lab1[2] - lab2[2]);
    // Alpha in Lab16 is the 8-bit alpha scaled to 16 bits, so the term reduces to
    // |a1 - a2| * 257 * 100 / 65535, which is |a1 - a2| * 100/255 to within rounding.
    const double alphaScale = 100.0 / 65535.0;
    const double dAlpha =
        std::fabs((double)(p1[3] * 257) - (double)(p2[3] * 257)) * alphaScale;
    const double diff = std::sqrt(dL * dL + da * da + db * db + dAlpha * dAlpha);
    return diff > 255.0 ? 255 : (uint8_t)diff;
}

int main() {
    cmsHPROFILE srgb = cmsCreate_sRGBProfile();
    cmsHPROFILE lab = cmsCreateLab4Profile(nullptr);
    to_lab16 = cmsCreateTransform(srgb, TYPE_RGBA_8, lab, TYPE_Lab_16, INTENT_PERCEPTUAL, 0);
    if (!to_lab16) { printf("변환을 만들 수 없다\n"); return 1; }

    printf("=== 대표 색쌍: lcms2 대 닫힌 형식\n");
    struct Pair { const char *label; uint8_t a[4], b[4]; };
    Pair pairs[] = {
        {"검정 대 검정",      {0,0,0,255},       {0,0,0,255}},
        {"검정 대 흰색",      {0,0,0,255},       {255,255,255,255}},
        {"빨강 대 초록",      {255,0,0,255},     {0,255,0,255}},
        {"회색 128 대 129",   {128,128,128,255}, {129,129,129,255}},
        {"회색 128 대 138",   {128,128,128,255}, {138,138,138,255}},
        {"흰색 대 거의 흰색",  {255,255,255,255}, {250,250,250,255}},
        {"투명 대 불투명",     {0,0,0,0},         {0,0,0,255}},
        {"반투명 대 불투명",   {10,20,30,128},    {10,20,30,255}},
    };
    int worst = 0;
    for (auto &p : pairs) {
        const int k = krita_difference_a(p.a, p.b);
        const int c = closed_form_difference(p.a, p.b);
        printf("  %-20s lcms2=%3d  닫힌형식=%3d  차이=%d\n", p.label, k, c, std::abs(k - c));
        worst = std::max(worst, std::abs(k - c));
    }
    printf("  최대 차이: %d\n", worst);

    printf("\n=== 넓은 스윕 (회색 램프와 채도 램프)\n");
    int sweep_worst = 0, over_one = 0, total = 0;
    for (int v1 = 0; v1 <= 255; v1 += 5) {
        for (int v2 = 0; v2 <= 255; v2 += 5) {
            uint8_t a[4] = {(uint8_t)v1, (uint8_t)v1, (uint8_t)v1, 255};
            uint8_t b[4] = {(uint8_t)v2, (uint8_t)v2, (uint8_t)v2, 255};
            const int k = krita_difference_a(a, b);
            const int c = closed_form_difference(a, b);
            const int d = std::abs(k - c);
            sweep_worst = std::max(sweep_worst, d);
            if (d > 1) over_one++;
            total++;
        }
    }
    printf("  회색 램프 %d 쌍: 최대 차이 %d, 1 초과 %d 쌍\n", total, sweep_worst, over_one);

    sweep_worst = over_one = total = 0;
    for (int r = 0; r <= 255; r += 51) {
      for (int g = 0; g <= 255; g += 51) {
        for (int b2 = 0; b2 <= 255; b2 += 51) {
          uint8_t a[4] = {(uint8_t)r, (uint8_t)g, (uint8_t)b2, 255};
          uint8_t b[4] = {128, 128, 128, 255};
          const int k = krita_difference_a(a, b);
          const int c = closed_form_difference(a, b);
          const int d = std::abs(k - c);
          sweep_worst = std::max(sweep_worst, d);
          if (d > 1) over_one++;
          total++;
        }
      }
    }
    printf("  색 큐브 %d 쌍 대 회색128: 최대 차이 %d, 1 초과 %d 쌍\n", total, sweep_worst, over_one);

    printf("\n=== 러스트가 맞춰야 하는 기준값 (lcms2)\n");
    for (auto &p : pairs) {
        printf("  (%3d,%3d,%3d,%3d) 대 (%3d,%3d,%3d,%3d) -> %d\n",
               p.a[0],p.a[1],p.a[2],p.a[3], p.b[0],p.b[1],p.b[2],p.b[3],
               krita_difference_a(p.a, p.b));
    }
    return 0;
}
