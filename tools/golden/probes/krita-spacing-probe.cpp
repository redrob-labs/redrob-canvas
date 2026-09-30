// Krita's elliptical spacing: where along a stroke segment the next dab lands.
//
// Canvas divides the segment length by a scalar spacing, so an elliptical brush spaces the same in every
// direction -- wrong now that dabs can be elliptical. Krita accumulates |dx| and |dy| separately and paints
// where the stroke crosses a spacing ELLIPSE, solved as a quadratic. This runs that solver verbatim to
// produce reference dab positions for the Rust.

#include <cmath>
#include <cstdio>
#include <vector>
#include <algorithm>

static const double MIN_DISTANCE_SPACING = 0.5;
static double pow2(double v) { return v * v; }

struct Point { double x, y; };

// Krita's effectiveSpacing, for the non-auto cases.
static Point effective_spacing(double dab_width, double dab_height, bool isotropic,
                               double spacing_val, double extra_scale) {
    Point spacing;
    if (!isotropic) {
        spacing = {dab_width * spacing_val, dab_height * spacing_val};
    } else {
        const double significant = std::max(dab_width, dab_height) * spacing_val;
        spacing = {significant, significant};
    }
    spacing.x *= extra_scale;
    spacing.y *= extra_scale;
    return spacing;
}

// Krita's setSpacing clamp.
static double clamp_spacing(double s) { return s < 0.02 ? 0.02 : s; }

// Krita's getNextPointPosition, elliptical branch.
class Accumulator {
public:
    Accumulator(Point spacing, double rotation, bool flipped)
        : m_spacing(spacing), m_rotation(rotation), m_flipped(flipped) {}

    // Returns t in [0,1] where the next dab lands on start..end, or -1 for none.
    double next_point_position(Point start, Point end) {
        if (start.x == end.x && start.y == end.y) return -1;

        const double a_rev = 1.0 / std::max(MIN_DISTANCE_SPACING, m_spacing.x);
        const double b_rev = 1.0 / std::max(MIN_DISTANCE_SPACING, m_spacing.y);

        const double x = m_accum.x;
        const double y = m_accum.y;
        const double gamma = pow2(x * a_rev) + pow2(y * b_rev) - 1;

        if (gamma >= 0.0) {
            reset();
            return 0.0;
        }

        static const double eps = 2e-3;
        double rotation = m_rotation;
        if (m_flipped) rotation = 2 * M_PI - rotation;

        Point diff = {end.x - start.x, end.y - start.y};
        if (rotation > eps) {
            const double c = std::cos(rotation), s = std::sin(rotation);
            diff = {diff.x * c - diff.y * s, diff.x * s + diff.y * c};
        }

        const double dx = std::fabs(diff.x);
        const double dy = std::fabs(diff.y);
        const double alpha = pow2(dx * a_rev) + pow2(dy * b_rev);
        const double beta = x * dx * a_rev * a_rev + y * dy * b_rev * b_rev;
        const double d_4 = pow2(beta) - alpha * gamma;

        double t = -1.0;
        if (d_4 >= 0) {
            const double k = (-beta + std::sqrt(d_4)) / alpha;
            if (k >= 0.0 && k <= 1.0) {
                t = k;
                reset();
            } else {
                m_accum.x += dx;
                m_accum.y += dy;
            }
        }
        return t;
    }

    void reset() { m_accum = {0.0, 0.0}; }

private:
    Point m_spacing;
    double m_rotation;
    bool m_flipped;
    Point m_accum{0.0, 0.0};
};

// Walk a polyline the way the stroke path does, collecting dab positions.
static std::vector<Point> walk(const std::vector<Point> &path, Point spacing,
                               double rotation, bool flipped, int cap) {
    std::vector<Point> dabs;
    Accumulator accumulator(spacing, rotation, flipped);
    if (path.empty()) return dabs;
    dabs.push_back(path[0]);
    for (size_t i = 0; i + 1 < path.size(); ++i) {
        Point start = path[i];
        const Point end = path[i + 1];
        // Krita re-solves from the last dab along the remainder of the segment.
        while ((int)dabs.size() < cap) {
            const double t = accumulator.next_point_position(start, end);
            if (t < 0.0) break;
            Point dab = {start.x + (end.x - start.x) * t, start.y + (end.y - start.y) * t};
            dabs.push_back(dab);
            start = dab;
            if (std::fabs(end.x - start.x) < 1e-12 && std::fabs(end.y - start.y) < 1e-12) break;
        }
    }
    return dabs;
}

static void report(const char *label, const std::vector<Point> &path, Point spacing,
                   double rotation, bool flipped) {
    const auto dabs = walk(path, spacing, rotation, flipped, 40);
    printf("=== %s  (간격 %.3f x %.3f)\n", label, spacing.x, spacing.y);
    printf("    답 %zu개:", dabs.size());
    for (size_t i = 0; i < dabs.size() && i < 12; ++i)
        printf(" (%.3f,%.3f)", dabs[i].x, dabs[i].y);
    printf("%s\n", dabs.size() > 12 ? " ..." : "");
}

int main() {
    printf("Krita: setSpacing 은 0.02 미만을 0.02 로, 거리 축은 0.5 로 하한을 둔다\n\n");

    // A round dab of diameter 20, spacing 0.25 -> a 5x5 spacing ellipse (a circle).
    Point round_spacing = effective_spacing(20, 20, false, clamp_spacing(0.25), 1.0);
    report("원형 답, 수평 스트로크", {{0, 0}, {40, 0}}, round_spacing, 0.0, false);
    report("원형 답, 수직 스트로크", {{0, 0}, {0, 40}}, round_spacing, 0.0, false);
    report("원형 답, 대각 스트로크", {{0, 0}, {30, 30}}, round_spacing, 0.0, false);

    // An elliptical dab, 20 wide and 5 tall. THIS is what canvas gets wrong today.
    Point ellipse_spacing = effective_spacing(20, 5, false, clamp_spacing(0.25), 1.0);
    report("타원 답(20x5), 수평", {{0, 0}, {40, 0}}, ellipse_spacing, 0.0, false);
    report("타원 답(20x5), 수직", {{0, 0}, {0, 40}}, ellipse_spacing, 0.0, false);
    printf("    ^ 수평은 넓게, 수직은 촘촘하게 — 스칼라 간격은 이 구분을 못 한다\n\n");

    // Isotropic: the same dab, spaced by its larger axis in both directions.
    Point isotropic = effective_spacing(20, 5, true, clamp_spacing(0.25), 1.0);
    report("타원 답, isotropic 수평", {{0, 0}, {40, 0}}, isotropic, 0.0, false);
    report("타원 답, isotropic 수직", {{0, 0}, {0, 40}}, isotropic, 0.0, false);

    // The 0.02 clamp and the 0.5 floor.
    Point tiny = effective_spacing(20, 20, false, clamp_spacing(0.001), 1.0);
    report("간격 0.001 -> 0.02 로 고정", {{0, 0}, {5, 0}}, tiny, 0.0, false);
    Point floored = effective_spacing(1, 1, false, clamp_spacing(0.02), 1.0);
    report("작은 답: 축이 0.5 로 하한", {{0, 0}, {3, 0}}, floored, 0.0, false);

    printf("\n=== 러스트가 맞춰야 하는 정확한 위치 (원형 20, 간격 0.25, 수평 0->40)\n");
    for (const auto &p : walk({{0, 0}, {40, 0}}, round_spacing, 0.0, false, 40))
        printf("  %.6f\n", p.x);
    printf("\n=== 타원 20x5, 간격 0.25, 수직 0->20\n");
    for (const auto &p : walk({{0, 0}, {0, 20}}, ellipse_spacing, 0.0, false, 40))
        printf("  %.6f\n", p.y);
    return 0;
}
