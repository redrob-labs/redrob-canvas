//! Rolling statistics and a latency tracker, for measuring the running product.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later,
//! taken into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/global/kis_latency_tracker.h` — `KisRollingMax`, `KisScalarTracker`, `KisLatencyTracker`
//! - `libs/global/KisFilteredRollingMean.{h,cpp}` — `KisFilteredRollingMean`
//!
//! Licence change notice: the originals are GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why this is translated first
//!
//! It measures what nothing else can see. Cycle 13 of the porting programme established that neither
//! golden-output comparison nor a screenshot can detect a missing stroke scheduler: a correct canvas
//! renders correctly, just too slowly, and both instruments say it is fine. The scheduler is the LAST
//! block to be translated and this is the FIRST, because this is what will judge it.
//!
//! # What was deliberately not translated
//!
//! `KisRollingMeanAccumulatorWrapper` and `KisRollingSumAccumulatorWrapper` (105 and 120 lines) exist,
//! by their own doc comment, to "hide boost includes from QtCreator preventing it from crashing". They
//! are a build-tooling workaround with no algorithmic content, and there is no boost here.
//!
//! The print path is not translated either. Krita's tracker calls `qInfo()` on a timer; this crate has
//! no logging dependency by design, so [`ScalarTracker`] exposes a [`ScalarStats`] snapshot and the
//! caller decides whether anything is emitted.

use std::collections::VecDeque;

/// The maximum value in a sliding window.
///
/// Krita implements this with a boost fibonacci heap plus a queue of handles, erasing the departing
/// sample's handle on each push. This uses a **monotonic deque** instead: O(1) amortised per push
/// against the heap's O(log n) erase, and no handle bookkeeping. That makes it a replacement rather
/// than a transliteration, which is the honest description.
///
/// It also does not reproduce the upstream's window arithmetic. MEASURED by compiling
/// `KisRollingMax` against real boost 1.83: `KisRollingMax(4)` holds **five** values, because `push`
/// evicts only once the queue is already longer than the window. The consequence is visible with a
/// decreasing stream — window 4, pushing 8,7,6,5,4 reports `max = 8` for five pushes where a true
/// 4-window drops to 7 on the fourth. The reported maximum is one sample stale.
///
/// A stale maximum is precisely the wrong error for the thing this measures. The maximum frame time is
/// the number that says whether a stroke ever stuttered, and holding a departed spike one sample too
/// long reports a stutter that has already passed.
#[derive(Debug, Clone)]
pub struct RollingMax<T> {
    window: usize,
    /// Values in decreasing order; the front is the window maximum. A value is dropped as soon as a
    /// larger one arrives after it, because it can never be the maximum again.
    candidates: VecDeque<(u64, T)>,
    pushed: u64,
}

impl<T: PartialOrd + Copy> RollingMax<T> {
    /// A tracker over the last `window` values. A window of 0 is treated as 1: a window that holds
    /// nothing has no maximum to report, and refusing here would push the check onto every caller.
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            candidates: VecDeque::new(),
            pushed: 0,
        }
    }

    pub fn push(&mut self, value: T) {
        // Drop every candidate this value dominates. They are behind it in the window and smaller, so
        // no future window can have them as its maximum.
        while self
            .candidates
            .back()
            .is_some_and(|(_, held)| *held <= value)
        {
            self.candidates.pop_back();
        }
        self.candidates.push_back((self.pushed, value));
        self.pushed += 1;

        // Evict the front if it has left the window. Exactly `window` samples are in scope, which is
        // the arithmetic the upstream gets wrong.
        if let Some((index, _)) = self.candidates.front()
            && self.pushed.saturating_sub(*index) > self.window as u64
        {
            self.candidates.pop_front();
        }
    }

    /// The largest value in the window, or `None` when nothing has been pushed.
    ///
    /// Krita's `max()` **throws** on an empty tracker. An `Option` says the same thing without a panic
    /// path, and the caller that wants the old behaviour can unwrap.
    pub fn max(&self) -> Option<T> {
        self.candidates.front().map(|(_, value)| *value)
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

/// A rolling mean that discards extreme deviations before averaging.
///
/// Translated from `KisFilteredRollingMean`. On each read it drops the lowest and highest portion of
/// the window and averages the rest — "basically, it takes the median and a few surrounding values",
/// as the upstream comment puts it. `add_value` is O(1); [`Self::filtered_mean`] is O(n) plus
/// O(n log m) in the number dropped, the same complexity as the original.
///
/// This is the right average for latency. One compositor hitch or one allocator pause moves a plain
/// mean and tells you the product is slow when it is not.
#[derive(Debug, Clone)]
pub struct FilteredRollingMean {
    values: VecDeque<f64>,
    window: usize,
    rolling_sum: f64,
    effective_portion: f64,
    /// Reused across reads so a steady-state call allocates nothing.
    cut_off_buffer: Vec<f64>,
}

impl FilteredRollingMean {
    /// `effective_portion` is the fraction of the window that is actually averaged; the rest is split
    /// between the two tails and dropped. It is clamped to `0.0..=1.0` because a portion outside that
    /// range has no meaning and the upstream does not check it.
    pub fn new(window: usize, effective_portion: f64) -> Self {
        let window = window.max(1);
        let effective_portion = effective_portion.clamp(0.0, 1.0);
        let cut_total = (window as f64 * (1.0 - effective_portion)).ceil();
        Self {
            values: VecDeque::with_capacity(window),
            window,
            rolling_sum: 0.0,
            effective_portion,
            cut_off_buffer: Vec::with_capacity((0.5 * cut_total).ceil() as usize),
        }
    }

    pub fn add_value(&mut self, value: f64) {
        if self.values.len() == self.window
            && let Some(oldest) = self.values.pop_front()
        {
            self.rolling_sum -= oldest;
        }
        self.values.push_back(value);
        self.rolling_sum += value;
    }

    /// The mean of the window with its tails removed, or `None` when the window is empty.
    ///
    /// Krita returns 0.0 for an empty window behind a soft assert. Zero is indistinguishable from a
    /// genuine zero-latency reading, so this returns `None` — the same reasoning that made an
    /// unanswerable plugin request a refusal rather than an empty list.
    pub fn filtered_mean(&mut self) -> Option<f64> {
        if self.values.is_empty() {
            return None;
        }
        let size = self.values.len();
        let useful = ((self.effective_portion * size as f64).round() as usize).max(1);
        let cut_total = size.saturating_sub(useful);
        if cut_total == 0 {
            return Some(self.rolling_sum / size as f64);
        }

        let cut_min = (0.5 * cut_total as f64).round() as usize;
        let cut_max = cut_total - cut_min;

        // The upstream guards both cuts with a resize in case its buffer was sized too small. MEASURED
        // across windows 2..64 and portions 0.05..0.95: not one combination undersizes it, so both
        // branches are dead code upstream and are not reproduced here. Capacity is still requested
        // rather than assumed, so a future portion change cannot silently start allocating per read.
        let needed = cut_min.max(cut_max);
        if self.cut_off_buffer.capacity() < needed {
            self.cut_off_buffer
                .reserve(needed - self.cut_off_buffer.capacity());
        }

        let mut sum = self.rolling_sum;

        // The lowest `cut_min`, then the highest `cut_max`. A partial sort, not a full one: that is the
        // optimisation the upstream comment mentions and the reason the complexity is O(n log m).
        self.cut_off_buffer.clear();
        self.cut_off_buffer.extend(self.values.iter().copied());
        let buffer = &mut self.cut_off_buffer;
        if cut_min > 0 {
            buffer.select_nth_unstable_by(cut_min - 1, |a, b| a.total_cmp(b));
            sum -= buffer[..cut_min].iter().sum::<f64>();
        }
        if cut_max > 0 {
            let at = size - cut_max;
            buffer.select_nth_unstable_by(at, |a, b| a.total_cmp(b));
            sum -= buffer[at..].iter().sum::<f64>();
        }

        Some(sum / useful as f64)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// A reading from a [`ScalarTracker`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalarStats {
    pub mean: f64,
    /// The **sample** variance, with an `n - 1` denominator.
    ///
    /// Matched to boost deliberately rather than assumed: `rolling_variance` over a 5-window of 1..10
    /// settles at 2.5, and the five values 1..5 have a population variance of 2.0 against a sample
    /// variance of 2.5. Reporting the population figure would make every latency spread read 20%
    /// tighter than Krita's for a 5-window, and the gap only closes as the window grows.
    pub variance: f64,
    pub max: f64,
    /// How many samples are in the window — without it, a mean from two samples reads like a mean
    /// from five hundred.
    pub samples: usize,
}

impl ScalarStats {
    /// The square root of the variance, which is the figure comparable to the mean.
    pub fn standard_deviation(&self) -> f64 {
        self.variance.max(0.0).sqrt()
    }
}

/// Rolling mean, variance and maximum over a window of scalars.
///
/// Translated from `KisScalarTracker`. Krita's version formats a string and calls `qInfo()` every
/// window-full or every second; this exposes [`Self::stats`] and emits nothing, because this crate has
/// no logging dependency by design and a measurement tool that writes to a log the product does not
/// have cannot be tested.
#[derive(Debug, Clone)]
pub struct ScalarTracker {
    window: usize,
    values: VecDeque<f64>,
    sum: f64,
    max: RollingMax<f64>,
}

impl ScalarTracker {
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            window,
            values: VecDeque::with_capacity(window),
            sum: 0.0,
            max: RollingMax::new(window),
        }
    }

    /// The window size Krita defaults to.
    pub const DEFAULT_WINDOW: usize = 500;

    pub fn push(&mut self, value: f64) {
        if self.values.len() == self.window
            && let Some(oldest) = self.values.pop_front()
        {
            self.sum -= oldest;
        }
        self.values.push_back(value);
        self.sum += value;
        self.max.push(value);
    }

    /// The current reading, or `None` when nothing has been pushed.
    pub fn stats(&self) -> Option<ScalarStats> {
        let samples = self.values.len();
        if samples == 0 {
            return None;
        }
        let mean = self.sum / samples as f64;
        // n - 1, matching boost. A single sample has no spread rather than a spread of zero, but zero
        // is the conventional report and the sample count distinguishes the two.
        let variance = if samples > 1 {
            self.values
                .iter()
                .map(|value| {
                    let deviation = value - mean;
                    deviation * deviation
                })
                .sum::<f64>()
                / (samples - 1) as f64
        } else {
            0.0
        };
        Some(ScalarStats {
            mean,
            variance,
            max: self.max.max().unwrap_or(mean),
            samples,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// How long events take to reach a point in the program.
///
/// Translated from `KisLatencyTracker`. The upstream is abstract over a `currentTimestamp()` the
/// subclass supplies; here the caller passes both timestamps, which keeps the type free of a clock and
/// therefore testable without one. Units are whatever the caller uses consistently — the tests and the
/// bench use microseconds.
#[derive(Debug, Clone)]
pub struct LatencyTracker {
    tracker: ScalarTracker,
}

impl LatencyTracker {
    pub fn new(window: usize) -> Self {
        Self {
            tracker: ScalarTracker::new(window),
        }
    }

    /// Record that an event stamped `event_timestamp` arrived at `now`.
    ///
    /// A negative difference is recorded as zero. It means the clocks disagree or the stamp is from the
    /// future, and a negative latency would drag the mean below anything achievable — a corrupt reading
    /// that looks like a good one, which is worse than a clamped reading that looks suspicious.
    pub fn push_event(&mut self, event_timestamp: i64, now: i64) {
        self.tracker
            .push(now.saturating_sub(event_timestamp).max(0) as f64);
    }

    pub fn stats(&self) -> Option<ScalarStats> {
        self.tracker.stats()
    }

    pub fn is_empty(&self) -> bool {
        self.tracker.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window holds exactly what was asked for, which the upstream does not.
    ///
    /// MEASURED against real boost: `KisRollingMax(4)` holds five values, and with a decreasing stream
    /// 8,7,6,5,4 it reports `max = 8` for five pushes. A true 4-window drops to 7 on the fourth.
    #[test]
    fn the_rolling_max_window_is_the_size_requested() {
        let mut max = RollingMax::new(4);
        for value in [8, 7, 6, 5] {
            max.push(value);
            assert_eq!(max.max(), Some(8), "8 is still in a 4-window");
        }
        // The fifth push pushes 8 out. Upstream still reports 8 here.
        max.push(4);
        assert_eq!(max.max(), Some(7), "8 has left the window");
        max.push(3);
        assert_eq!(max.max(), Some(6));
        max.push(2);
        assert_eq!(max.max(), Some(5));
        max.push(1);
        assert_eq!(max.max(), Some(4));
    }

    #[test]
    fn the_rolling_max_rises_immediately() {
        let mut max = RollingMax::new(4);
        assert_eq!(max.max(), None, "nothing pushed, nothing to report");
        for value in 1..=8 {
            max.push(value);
            assert_eq!(
                max.max(),
                Some(value),
                "an increasing stream maxes at its newest"
            );
        }
    }

    #[test]
    fn a_rolling_max_window_of_zero_behaves_as_one() {
        let mut max = RollingMax::new(0);
        max.push(5);
        assert_eq!(max.max(), Some(5));
        max.push(3);
        assert_eq!(max.max(), Some(3), "a window of one holds only the newest");
    }

    /// Equal values must not vanish.
    ///
    /// The deque drops a candidate the new value is greater than **or equal to**. That is required for
    /// correctness — keeping equal duplicates would leave stale entries the eviction check cannot
    /// reach — but it means a run of equal values must still report that value.
    #[test]
    fn a_run_of_equal_values_keeps_reporting_that_value() {
        let mut max = RollingMax::new(3);
        for _ in 0..6 {
            max.push(7);
            assert_eq!(max.max(), Some(7));
        }
        max.push(1);
        assert_eq!(max.max(), Some(7), "a 3-window still holds two 7s");
        max.push(1);
        max.push(1);
        assert_eq!(max.max(), Some(1), "now the window is all 1s");
    }

    /// The reference values measured from the upstream with real boost.
    #[test]
    fn the_filtered_mean_matches_the_measured_upstream() {
        // Nine 5s and one 1000. Upstream reports exactly 5.0: the extreme is dropped.
        let mut mean = FilteredRollingMean::new(10, 0.6);
        for _ in 0..9 {
            mean.add_value(5.0);
        }
        mean.add_value(1000.0);
        assert_eq!(
            mean.filtered_mean(),
            Some(5.0),
            "one 1000 must not move the mean"
        );

        // 1..10 with the same window and portion. Upstream reports 5.5.
        let mut ramp = FilteredRollingMean::new(10, 0.6);
        for value in 1..=10 {
            ramp.add_value(value as f64);
        }
        let got = ramp.filtered_mean().unwrap();
        assert!(
            (got - 5.5).abs() < 1e-9,
            "expected the upstream's 5.5, got {got}"
        );
    }

    /// A plain mean would be dragged by the spike this one drops. Without this the filtering is
    /// untested: a mean of 5.0 is also what a broken implementation returns if it ignores input.
    #[test]
    fn the_filtering_is_what_rejects_the_spike() {
        let mut filtered = FilteredRollingMean::new(10, 0.6);
        let mut plain = 0.0f64;
        for _ in 0..9 {
            filtered.add_value(5.0);
            plain += 5.0;
        }
        filtered.add_value(1000.0);
        plain += 1000.0;
        assert_eq!(filtered.filtered_mean(), Some(5.0));
        assert!(
            (plain / 10.0 - 104.5).abs() < 1e-9,
            "the unfiltered mean is 104.5, twenty times higher"
        );
    }

    /// An empty window has no mean, and does not claim zero.
    #[test]
    fn an_empty_filtered_mean_is_none_not_zero() {
        let mut mean = FilteredRollingMean::new(10, 0.6);
        assert_eq!(mean.filtered_mean(), None);
        mean.add_value(0.0);
        assert_eq!(
            mean.filtered_mean(),
            Some(0.0),
            "a genuine zero reading is distinguishable from no reading"
        );
    }

    /// A portion of 1.0 drops nothing; a portion of 0.0 still averages one value.
    #[test]
    fn the_portion_bounds_behave() {
        let mut all = FilteredRollingMean::new(5, 1.0);
        for value in [1.0, 2.0, 3.0, 4.0, 100.0] {
            all.add_value(value);
        }
        assert_eq!(
            all.filtered_mean(),
            Some(22.0),
            "nothing dropped: a plain mean"
        );

        let mut none = FilteredRollingMean::new(5, 0.0);
        for value in [1.0, 2.0, 3.0, 4.0, 100.0] {
            none.add_value(value);
        }
        // useful is clamped to 1, so this is the median-ish single survivor rather than a division by zero.
        let got = none.filtered_mean().unwrap();
        assert!(
            got.is_finite(),
            "a portion of 0 must not divide by zero, got {got}"
        );

        // Out of range is clamped rather than trusted.
        let mut absurd = FilteredRollingMean::new(5, 9.0);
        absurd.add_value(4.0);
        assert_eq!(absurd.filtered_mean(), Some(4.0));
    }

    /// Reading twice must give the same answer -- `filtered_mean` mutates a scratch buffer.
    #[test]
    fn reading_the_filtered_mean_twice_is_stable() {
        let mut mean = FilteredRollingMean::new(10, 0.6);
        for value in [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 100.0] {
            mean.add_value(value);
        }
        let first = mean.filtered_mean();
        let second = mean.filtered_mean();
        assert_eq!(
            first, second,
            "the scratch buffer must not corrupt the window"
        );
        mean.add_value(3.0);
        assert!(
            mean.filtered_mean().is_some(),
            "and the window still accepts values"
        );
    }

    /// The variance is boost's: an n-1 denominator.
    ///
    /// MEASURED: boost's `rolling_variance` over a 5-window of 1..10 settles at 2.5. The population
    /// variance of 1..5 is 2.0, so a population denominator would report every spread 20% tighter.
    #[test]
    fn the_variance_matches_boost() {
        let mut tracker = ScalarTracker::new(5);
        for value in 1..=5 {
            tracker.push(value as f64);
        }
        let stats = tracker.stats().unwrap();
        assert!((stats.mean - 3.0).abs() < 1e-9, "mean {}", stats.mean);
        assert!(
            (stats.variance - 2.5).abs() < 1e-9,
            "expected boost's 2.5, got {}",
            stats.variance
        );
        assert_eq!(stats.max, 5.0);
        assert_eq!(stats.samples, 5);

        // Boost's second reading over 1,2 is 0.5.
        let mut pair = ScalarTracker::new(5);
        pair.push(1.0);
        pair.push(2.0);
        assert!((pair.stats().unwrap().variance - 0.5).abs() < 1e-9);

        // And it stays at 2.5 as the window slides, which is what boost reported for n=6..10.
        for value in 6..=10 {
            tracker.push(value as f64);
            let sliding = tracker.stats().unwrap();
            assert!(
                (sliding.variance - 2.5).abs() < 1e-9,
                "a sliding window of five consecutive integers has variance 2.5, got {}",
                sliding.variance
            );
        }
        assert_eq!(tracker.stats().unwrap().max, 10.0);
    }

    #[test]
    fn one_sample_has_no_spread_but_is_counted() {
        let mut tracker = ScalarTracker::new(10);
        assert!(tracker.stats().is_none());
        tracker.push(42.0);
        let stats = tracker.stats().unwrap();
        assert_eq!(stats.mean, 42.0);
        assert_eq!(stats.variance, 0.0);
        assert_eq!(
            stats.samples, 1,
            "the sample count is how you know the 0 is not a spread"
        );
        assert_eq!(stats.standard_deviation(), 0.0);
    }

    #[test]
    fn the_latency_tracker_records_the_difference() {
        let mut latency = LatencyTracker::new(100);
        assert!(latency.stats().is_none());
        latency.push_event(1_000, 1_016);
        latency.push_event(2_000, 2_033);
        let stats = latency.stats().unwrap();
        assert!((stats.mean - 24.5).abs() < 1e-9, "mean {}", stats.mean);
        assert_eq!(stats.max, 33.0);
    }

    /// A stamp from the future is clamped, not recorded as negative.
    #[test]
    fn a_negative_latency_is_clamped_to_zero() {
        let mut latency = LatencyTracker::new(10);
        latency.push_event(5_000, 4_000);
        let stats = latency.stats().unwrap();
        assert_eq!(
            stats.mean, 0.0,
            "a future stamp cannot pull the mean negative"
        );
        assert_eq!(stats.samples, 1, "and it is still counted as a reading");

        // Mixed with a real reading, the clamped one does not subtract from it.
        latency.push_event(1_000, 1_020);
        assert_eq!(latency.stats().unwrap().mean, 10.0);
    }

    /// The trackers must survive the window sliding many times over -- this is what will run for the
    /// length of a stroke.
    ///
    /// The spike period is chosen so a spike is inside the final window at read time. The first version
    /// of this test spiked every 250 steps over 5,000 steps with a 64-window, so the last spike landed
    /// at step 4,750 and the window held steps 4,937..4,999 -- no spike in it at all. Both means read
    /// 16.0 and the assertion that the plain mean carries the spike failed against a correct filter.
    #[test]
    fn the_trackers_hold_up_over_many_windows() {
        let window = 64;
        let mut tracker = ScalarTracker::new(window);
        let mut filtered = FilteredRollingMean::new(window, 0.8);
        // Every 40th step, so a 64-sample window always contains one or two.
        for step in 0..5_000u32 {
            let value = if step % 40 == 0 { 400.0 } else { 16.0 };
            tracker.push(value);
            filtered.add_value(value);
        }
        let stats = tracker.stats().unwrap();
        assert_eq!(
            stats.samples, window,
            "the window never grows past its size"
        );
        assert_eq!(
            stats.max, 400.0,
            "a spike is in the window, which is the point"
        );

        let filtered_mean = filtered.filtered_mean().unwrap();
        assert!(
            (filtered_mean - 16.0).abs() < 1e-9,
            "the filtered mean rejects the spikes entirely, got {filtered_mean}"
        );
        assert!(
            stats.mean > 20.0,
            "while the plain mean carries them -- got {}, and if this ever equals 16.0 the window \
             holds no spike and the comparison is vacuous",
            stats.mean
        );
    }
}
