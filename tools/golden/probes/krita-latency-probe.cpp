// Run Krita's own rolling-max and filtered-rolling-mean logic with real boost, so the Rust translation is
// checked against measured behaviour rather than against my reading of the source.
//
// The Qt and kis_assert dependencies are replaced with equivalents; the ALGORITHM is verbatim.

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <deque>
#include <iostream>
#include <numeric>
#include <queue>
#include <vector>

#include <boost/circular_buffer.hpp>
#include <boost/heap/fibonacci_heap.hpp>
#include <boost/accumulators/accumulators.hpp>
#include <boost/accumulators/statistics/stats.hpp>
#include <boost/accumulators/statistics/rolling_mean.hpp>
#include <boost/accumulators/statistics/rolling_variance.hpp>

// ---- Krita's KisRollingMax, verbatim apart from QQueue -> std::queue.
template<typename T>
class KisRollingMax {
public:
    KisRollingMax(int windowSize) : m_windowSize(windowSize) {}

    void push(T value) {
        while (m_samples.size() > (size_t)m_windowSize) {
            m_values.erase(m_samples.front());
            m_samples.pop();
        }
        m_samples.push(m_values.push(value));
    }

    T max() const { return m_values.top(); }
    size_t heldValues() const { return m_values.size(); }

private:
    const int m_windowSize;
    typedef boost::heap::fibonacci_heap<T> heap_type;
    std::queue<typename heap_type::handle_type> m_samples;
    heap_type m_values;
};

// ---- Krita's KisFilteredRollingMean, verbatim apart from the asserts.
class KisFilteredRollingMean {
public:
    KisFilteredRollingMean(int windowSize, double effectivePortion)
        : m_values(windowSize), m_rollingSum(0.0), m_effectivePortion(effectivePortion),
          m_cutOffBuffer((size_t)std::ceil(0.5 * std::ceil(windowSize * (1.0 - effectivePortion)))) {}

    void addValue(double value) {
        if (m_values.full()) m_rollingSum -= m_values.front();
        m_values.push_back(value);
        m_rollingSum += value;
    }

    double filteredMean(bool *bufferWasTooSmall = nullptr) const {
        if (m_values.empty()) return 0.0;
        const int usefulElements = std::max(1, (int)std::lround(m_effectivePortion * m_values.size()));
        double sum = 0.0;
        int num = 0;
        const int cutOffTotal = (int)m_values.size() - usefulElements;
        if (cutOffTotal > 0) {
            const size_t cutMin = (size_t)std::lround(0.5 * cutOffTotal);
            const size_t cutMax = (size_t)cutOffTotal - cutMin;
            if (bufferWasTooSmall &&
                (cutMin > m_cutOffBuffer.size() || cutMax > m_cutOffBuffer.size())) {
                *bufferWasTooSmall = true;
            }
            if (cutMin > m_cutOffBuffer.size()) m_cutOffBuffer.resize(cutMin);
            if (cutMax > m_cutOffBuffer.size()) m_cutOffBuffer.resize(cutMax);
            sum = m_rollingSum;
            num = usefulElements;
            std::partial_sort_copy(m_values.begin(), m_values.end(),
                                   m_cutOffBuffer.begin(), m_cutOffBuffer.begin() + cutMin);
            sum -= std::accumulate(m_cutOffBuffer.begin(), m_cutOffBuffer.begin() + cutMin, 0.0);
            std::partial_sort_copy(m_values.begin(), m_values.end(),
                                   m_cutOffBuffer.begin(), m_cutOffBuffer.begin() + cutMax,
                                   std::greater<double>());
            sum -= std::accumulate(m_cutOffBuffer.begin(), m_cutOffBuffer.begin() + cutMax, 0.0);
        } else {
            sum = m_rollingSum;
            num = (int)m_values.size();
        }
        if (num <= 0) return 0.0;
        return sum / num;
    }

private:
    boost::circular_buffer<double> m_values;
    double m_rollingSum;
    double m_effectivePortion;
    mutable std::vector<double> m_cutOffBuffer;
};

int main() {
    std::cout.precision(10);

    // --- 1. Does the rolling max hold more than its window?
    std::cout << "=== KisRollingMax: window 4, pushing 1..8\n";
    KisRollingMax<int64_t> rmax(4);
    for (int64_t v = 1; v <= 8; ++v) {
        rmax.push(v);
        std::cout << "  push " << v << "  held=" << rmax.heldValues() << "  max=" << rmax.max() << "\n";
    }
    // A true 4-window over 1..8 ends holding 5,6,7,8. Anything larger means the window is wider than asked.

    std::cout << "\n=== 감소하는 값으로: window 4, pushing 8..1 (max 가 언제 떨어지는가)\n";
    KisRollingMax<int64_t> rmax2(4);
    for (int64_t v = 8; v >= 1; --v) {
        rmax2.push(v);
        std::cout << "  push " << v << "  held=" << rmax2.heldValues() << "  max=" << rmax2.max() << "\n";
    }

    // --- 2. Can the cut-off buffer be undersized? Sweep window and portion.
    std::cout << "\n=== KisFilteredRollingMean: 버퍼가 작아지는 조합이 있는가\n";
    int undersized = 0;
    for (int window = 2; window <= 64; ++window) {
        for (double portion = 0.05; portion <= 0.99; portion += 0.05) {
            KisFilteredRollingMean mean(window, portion);
            bool tooSmall = false;
            for (int i = 0; i < window * 2; ++i) {
                mean.addValue((double)(i % 17));
                mean.filteredMean(&tooSmall);
            }
            if (tooSmall) {
                if (undersized < 6)
                    std::cout << "  window=" << window << " portion=" << portion << "  버퍼 부족\n";
                undersized++;
            }
        }
    }
    std::cout << "  버퍼가 부족한 조합: " << undersized << "개\n";

    // --- 3. Reference values for the Rust tests to match.
    std::cout << "\n=== 러스트가 맞춰야 하는 기준값 (window 10, portion 0.6)\n";
    KisFilteredRollingMean ref(10, 0.6);
    const double samples[] = {5, 5, 5, 5, 5, 5, 5, 5, 5, 1000};
    for (int i = 0; i < 10; ++i) {
        ref.addValue(samples[i]);
        std::cout << "  n=" << (i + 1) << "  filteredMean=" << ref.filteredMean() << "\n";
    }
    std::cout << "  <- 마지막: 1000 이 들어와도 평균이 5 근처면 극단값을 걸러낸 것\n";

    KisFilteredRollingMean ref2(10, 0.6);
    for (int i = 1; i <= 10; ++i) ref2.addValue(i);
    std::cout << "\n  1..10, portion 0.6 -> filteredMean=" << ref2.filteredMean() << "\n";

    // --- 4. Boost's rolling mean and variance, for the tracker's own numbers.
    std::cout << "\n=== boost rolling_mean / rolling_variance (window 5, 1..10)\n";
    namespace ba = boost::accumulators;
    ba::accumulator_set<double, ba::stats<ba::tag::lazy_rolling_mean, ba::tag::rolling_variance>>
        acc(ba::tag::rolling_window::window_size = 5);
    for (int i = 1; i <= 10; ++i) {
        acc(i);
        std::cout << "  n=" << i << "  mean=" << ba::rolling_mean(acc)
                  << "  var=" << ba::rolling_variance(acc) << "\n";
    }
    return 0;
}
