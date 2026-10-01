// Krita's CURRENT KisCubicSpline, run verbatim with real Eigen, to produce reference values for the
// Rust translation and to test one claim: that a corner knot makes the spline SEPARABLE, so each run
// between corners is an independent natural cubic spline solvable tridiagonally in O(n) rather than by
// a 4n x 4n sparse solve.
//
// Qt containers and kis_assert are replaced; the equation assembly is untouched.

#include <cstdio>
#include <vector>
#include <Eigen/Sparse>

struct Pt {
    double mx, my;
    bool corner;
    double x() const { return mx; }
    double y() const { return my; }
    bool isSetAsCorner() const { return corner; }
};

struct Coefficients { double a, b, c, d; };

class KisCubicSpline {
public:
    void createSpline(const std::vector<Pt> &a) {
        if (a.empty()) return;
        const int intervals = (int)a.size() - 1;
        m_points = a;
        m_coefficients.clear();

        if (a.size() == 1) { m_coefficients.push_back({0.0, 0.0, 0.0, a.front().y()}); return; }
        if (a.size() == 2) {
            const double c = (a.back().y() - a.front().y()) / (a.back().x() - a.front().x());
            const double d = a.front().y() - c * a.front().x();
            m_coefficients.push_back({0.0, 0.0, c, d});
            return;
        }

        using Triplet = Eigen::Triplet<double>;
        using Matrix = Eigen::SparseMatrix<double>;
        using Vector = Eigen::VectorXd;

        const int numberOfRows = intervals * 4;
        const int numberOfColumns = numberOfRows;
        std::vector<Triplet> triplets;
        Matrix A(numberOfRows, numberOfColumns);
        Vector b(numberOfRows);
        triplets.reserve(numberOfRows * 4);
        int row = 0;

        double pointX = a.front().x(), pointY = a.front().y();
        double pointXSquared = pointX * pointX, pointXCubed = pointXSquared * pointX;
        for (int i = 0; i < intervals; ++i) {
            const int baseColumn = i * 4;
            triplets.push_back(Triplet(row, baseColumn + 0, pointXCubed));
            triplets.push_back(Triplet(row, baseColumn + 1, pointXSquared));
            triplets.push_back(Triplet(row, baseColumn + 2, pointX));
            triplets.push_back(Triplet(row, baseColumn + 3, 1.0));
            b(row) = pointY; ++row;
            pointX = a[i + 1].x(); pointY = a[i + 1].y();
            pointXSquared = pointX * pointX; pointXCubed = pointXSquared * pointX;
            triplets.push_back(Triplet(row, baseColumn + 0, pointXCubed));
            triplets.push_back(Triplet(row, baseColumn + 1, pointXSquared));
            triplets.push_back(Triplet(row, baseColumn + 2, pointX));
            triplets.push_back(Triplet(row, baseColumn + 3, 1.0));
            b(row) = pointY; ++row;
        }
        pointX = a.front().x();
        triplets.push_back(Triplet(row, 0, 6.0 * pointX));
        triplets.push_back(Triplet(row, 1, 2.0));
        b(row) = 0.0; ++row;
        pointX = a.back().x();
        triplets.push_back(Triplet(row, numberOfColumns - 4, 6.0 * pointX));
        triplets.push_back(Triplet(row, numberOfColumns - 3, 2.0));
        b(row) = 0.0; ++row;
        for (int i = 1; i < (int)a.size() - 1; ++i) {
            pointX = a[i].x();
            const int baseColumn = i * 4;
            if (a[i].isSetAsCorner()) {
                triplets.push_back(Triplet(row, baseColumn - 4, 6.0 * pointX));
                triplets.push_back(Triplet(row, baseColumn - 3, 2.0));
                b(row) = 0.0; ++row;
                triplets.push_back(Triplet(row, baseColumn + 0, 6.0 * pointX));
                triplets.push_back(Triplet(row, baseColumn + 1, 2.0));
                b(row) = 0.0; ++row;
            } else {
                pointXSquared = pointX * pointX;
                triplets.push_back(Triplet(row, baseColumn - 4, 3.0 * pointXSquared));
                triplets.push_back(Triplet(row, baseColumn - 3, 2.0 * pointX));
                triplets.push_back(Triplet(row, baseColumn - 2, 1.0));
                triplets.push_back(Triplet(row, baseColumn + 0, -3.0 * pointXSquared));
                triplets.push_back(Triplet(row, baseColumn + 1, -2.0 * pointX));
                triplets.push_back(Triplet(row, baseColumn + 2, -1.0));
                b(row) = 0.0; ++row;
                triplets.push_back(Triplet(row, baseColumn - 4, 6.0 * pointX));
                triplets.push_back(Triplet(row, baseColumn - 3, 2.0));
                triplets.push_back(Triplet(row, baseColumn + 0, -6.0 * pointX));
                triplets.push_back(Triplet(row, baseColumn + 1, -2.0));
                b(row) = 0.0; ++row;
            }
        }
        A.setFromTriplets(triplets.begin(), triplets.end());
        A.makeCompressed();
        Eigen::SparseLU<Matrix> solver;
        solver.analyzePattern(A);
        solver.factorize(A);
        Vector x = solver.solve(b);
        for (int i = 0; i < intervals; ++i) {
            m_coefficients.push_back({x(i * 4 + 0), x(i * 4 + 1), x(i * 4 + 2), x(i * 4 + 3)});
        }
    }

    double getValue(double x) const {
        if (m_coefficients.empty()) return 0.0;
        int interval;
        for (interval = 0; interval < (int)m_coefficients.size() - 1; ++interval) {
            if (x < m_points[interval + 1].x()) break;
        }
        const double xs = x * x, xc = xs * x;
        const Coefficients &c = m_coefficients[interval];
        return c.a * xc + c.b * xs + c.c * x + c.d;
    }

private:
    std::vector<Pt> m_points;
    std::vector<Coefficients> m_coefficients;
};

// Krita's KisCubicCurve::Data::value -- clamp x into range, then clamp y to [0,1].
static double curveValue(const KisCubicSpline &s, const std::vector<Pt> &pts, double x) {
    if (x < pts.front().x()) x = pts.front().x();
    if (x > pts.back().x()) x = pts.back().x();
    double y = s.getValue(x);
    return y < 0.0 ? 0.0 : (y > 1.0 ? 1.0 : y);
}

static void dump(const char *name, std::vector<Pt> pts) {
    KisCubicSpline s;
    s.createSpline(pts);
    printf("=== %s\n", name);
    printf("    points:");
    for (auto &p : pts) printf(" (%.3f,%.3f%s)", p.mx, p.my, p.corner ? ",C" : "");
    printf("\n");
    for (double x : {0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0}) {
        printf("    x=%.2f  spline=%.12f  curve=%.12f\n", x, s.getValue(x), curveValue(s, pts, x));
    }
    printf("\n");
}

int main() {
    // The identity a tone curve starts from.
    dump("identity (2 points)", {{0.0, 0.0, false}, {1.0, 1.0, false}});

    // A single interior point: an S-curve, the most common adjustment.
    dump("3 points, smooth", {{0.0, 0.0, false}, {0.5, 0.7, false}, {1.0, 1.0, false}});

    // Two interior points.
    dump("4 points, smooth",
         {{0.0, 0.0, false}, {0.25, 0.1, false}, {0.75, 0.9, false}, {1.0, 1.0, false}});

    // The same, with the middle point marked as a corner.
    dump("3 points, middle is a CORNER", {{0.0, 0.0, false}, {0.5, 0.7, true}, {1.0, 1.0, false}});

    dump("5 points, middle is a CORNER",
         {{0.0, 0.0, false}, {0.25, 0.3, false}, {0.5, 0.7, true},
          {0.75, 0.8, false}, {1.0, 1.0, false}});

    // A curve that overshoots, to prove the y clamp matters.
    dump("overshooting curve", {{0.0, 0.0, false}, {0.4, 0.95, false},
                                {0.6, 0.98, false}, {1.0, 1.0, false}});

    // One point: a constant.
    dump("1 point", {{0.3, 0.42, false}});

    // === The separability claim: a corner run solved alone must equal the same run inside a
    // larger spline that has a corner at its boundary.
    printf("=== 분리 가능성 주장: 코너가 스플라인을 독립 구간으로 나누는가\n");
    std::vector<Pt> whole = {{0.0, 0.0, false}, {0.25, 0.3, false}, {0.5, 0.7, true},
                             {0.75, 0.8, false}, {1.0, 1.0, false}};
    std::vector<Pt> leftRun = {{0.0, 0.0, false}, {0.25, 0.3, false}, {0.5, 0.7, false}};
    std::vector<Pt> rightRun = {{0.5, 0.7, false}, {0.75, 0.8, false}, {1.0, 1.0, false}};
    KisCubicSpline sWhole, sLeft, sRight;
    sWhole.createSpline(whole);
    sLeft.createSpline(leftRun);
    sRight.createSpline(rightRun);
    double worst = 0.0;
    for (int i = 0; i <= 100; ++i) {
        double x = i / 100.0;
        double a = sWhole.getValue(x);
        double b = (x <= 0.5) ? sLeft.getValue(x) : sRight.getValue(x);
        double diff = a - b;
        if (diff < 0) diff = -diff;
        if (diff > worst) worst = diff;
    }
    printf("    코너로 나눈 독립 스플라인과 전체의 최대 차이: %.3e\n", worst);
    printf("    %s\n", worst < 1e-9 ? "-> 분리 가능하다. 삼중대각으로 구간마다 풀면 된다."
                                    : "-> 분리 불가능하다. 전체 계를 풀어야 한다.");
    return 0;
}
