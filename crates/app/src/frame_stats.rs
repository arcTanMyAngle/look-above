//! Frame-time accounting for the debug log, and (from M2 2.1) the F3 stats mode.
//!
//! M0 item 0.6 built the mean/worst stub to give the window a heartbeat. M2 2.1 adds p50/p95
//! alongside it — percentiles need the interval's individual frame times, not just a running
//! sum, so this keeps a small per-window buffer and sorts it once a second. That's a few
//! hundred `Duration`s at most, well under the ≤4 ms render-thread budget in docs/01 since it
//! only runs on the once-per-second reporting edge, never per frame.
//!
//! On-screen text (the "overlay" the M2 checklist item names) is 2.1b, deferred until the
//! glyph atlas (2.5/2.7) exists — this module only produces the numbers.

use std::time::{Duration, Instant};

/// How much frame history one log line covers.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// One report interval's worth of frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSummary {
    /// Frames drawn in the interval.
    pub frames: u32,
    /// Wall time the interval actually covered — not exactly [`REPORT_INTERVAL`], since a
    /// report only lands on a frame boundary.
    pub elapsed: Duration,
    /// Mean time between those frames.
    pub mean: Duration,
    /// The worst single gap: where a stutter shows up that the mean hides.
    pub worst: Duration,
    /// Median frame time: half the interval's frames were at or under this.
    pub p50: Duration,
    /// 95th-percentile frame time: the budget docs/01 actually cares about, since it survives
    /// the occasional stutter the mean hides without being dominated by one outlier the way
    /// `worst` is.
    pub p95: Duration,
}

impl FrameSummary {
    /// Frames per second across the interval.
    pub fn fps(&self) -> f64 {
        if self.elapsed.is_zero() {
            return 0.0;
        }
        f64::from(self.frames) / self.elapsed.as_secs_f64()
    }
}

/// Accumulates frame times and hands back a [`FrameSummary`] once per [`REPORT_INTERVAL`].
///
/// Takes the clock as an argument rather than reading it, so the reporting logic is
/// testable without sleeping.
#[derive(Debug, Default)]
pub struct FrameStats {
    window_start: Option<Instant>,
    last_frame: Option<Instant>,
    total: Duration,
    worst: Duration,
    /// This interval's individual frame times, cleared on each report. Needed for
    /// percentiles — `total`/`worst` alone can't yield them.
    samples: Vec<Duration>,
}

impl FrameStats {
    /// Record a frame presented at `now`.
    ///
    /// Returns a summary on the first frame at or past the end of the interval, and `None`
    /// otherwise. What is measured is the gap *between* frames, so the very first frame
    /// starts the clock without contributing one.
    pub fn record(&mut self, now: Instant) -> Option<FrameSummary> {
        let started = *self.window_start.get_or_insert(now);

        if let Some(previous) = self.last_frame.replace(now) {
            let frame_time = now.saturating_duration_since(previous);
            self.total += frame_time;
            self.worst = self.worst.max(frame_time);
            self.samples.push(frame_time);
        }

        let elapsed = now.saturating_duration_since(started);
        let frames = u32::try_from(self.samples.len()).unwrap_or(u32::MAX);
        if elapsed < REPORT_INTERVAL || frames == 0 {
            return None;
        }

        self.samples.sort_unstable();
        let summary = FrameSummary {
            frames,
            elapsed,
            mean: self.total / frames,
            worst: self.worst,
            p50: percentile(&self.samples, 50),
            p95: percentile(&self.samples, 95),
        };

        self.window_start = Some(now);
        self.total = Duration::ZERO;
        self.worst = Duration::ZERO;
        self.samples.clear();

        Some(summary)
    }
}

/// The `percent`-th percentile (`0..=100`) of an already-sorted, non-empty slice.
///
/// Nearest-rank, in whole samples: rank `ceil(percent * n / 100)`, 1-based and clamped into
/// range. Integer arithmetic throughout — this module's sample counts are at most a few
/// hundred (one reporting window's worth of frames per docs/01), so there's no precision to
/// lose by avoiding floats, and it sidesteps the `f64`/`usize` cast lints entirely.
fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    debug_assert!(!sorted.is_empty(), "percentile of an empty sample set");
    debug_assert!(percent <= 100, "percent out of range");
    let n = sorted.len();
    let rank = percent.saturating_mul(n).div_ceil(100).max(1);
    let index = (rank - 1).min(n - 1);
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed clock: `Instant` cannot be constructed from a number, so build one by
    /// stepping forward from a real origin.
    fn at(origin: Instant, millis: u64) -> Instant {
        origin + Duration::from_millis(millis)
    }

    #[test]
    fn reports_nothing_before_the_interval_is_up() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        for frame in 0..10 {
            assert_eq!(stats.record(at(origin, frame * 16)), None);
        }
    }

    #[test]
    fn reports_once_the_interval_has_elapsed() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        stats.record(origin);
        assert_eq!(stats.record(at(origin, 999)), None);

        let summary = stats
            .record(at(origin, 1_000))
            .expect("a frame at the interval boundary reports");
        assert_eq!(summary.frames, 2);
        assert_eq!(summary.elapsed, Duration::from_secs(1));
    }

    /// 60 frames one second apart… at 16 ms spacing should read as ~62 fps, and the mean
    /// should be the spacing itself.
    #[test]
    fn measures_rate_and_mean_over_the_interval() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        let mut reported = None;
        for frame in 0..=63 {
            if let Some(summary) = stats.record(at(origin, frame * 16)) {
                reported = Some(summary);
                break;
            }
        }

        let summary = reported.expect("a second of 16 ms frames reports");
        assert_eq!(summary.mean, Duration::from_millis(16));
        assert!(
            (summary.fps() - 62.5).abs() < 0.1,
            "fps was {}",
            summary.fps()
        );
    }

    /// The mean would swallow a single long frame; `worst` is there to keep it.
    #[test]
    fn worst_frame_survives_a_fast_interval() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        stats.record(origin);
        stats.record(at(origin, 200)); // one 200 ms stall
        let mut elapsed = 200;
        let mut summary = None;
        while summary.is_none() && elapsed <= 2_000 {
            elapsed += 16;
            summary = stats.record(at(origin, elapsed));
        }

        let summary = summary.expect("the interval reports");
        assert_eq!(summary.worst, Duration::from_millis(200));
        assert!(summary.mean < summary.worst);
    }

    /// After a report, the next interval starts clean rather than carrying the old stall.
    #[test]
    fn counters_reset_between_reports() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        stats.record(origin);
        stats
            .record(at(origin, 1_000))
            .expect("the first interval reports");

        let summary = stats
            .record(at(origin, 2_000))
            .expect("the second interval reports");
        assert_eq!(summary.frames, 1);
        assert_eq!(summary.worst, Duration::from_secs(1));
        assert_eq!(summary.elapsed, Duration::from_secs(1));
    }

    /// With no variance in frame time, every percentile reads back as the spacing itself.
    #[test]
    fn p50_and_p95_equal_the_spacing_when_frame_times_are_uniform() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        let mut reported = None;
        for frame in 0..=63 {
            if let Some(summary) = stats.record(at(origin, frame * 16)) {
                reported = Some(summary);
                break;
            }
        }

        let summary = reported.expect("a second of 16 ms frames reports");
        assert_eq!(summary.p50, Duration::from_millis(16));
        assert_eq!(summary.p95, Duration::from_millis(16));
    }

    /// A single stall among nine quick frames is rare enough that the median doesn't see it,
    /// but common enough (1 in 10) that p95 does — that's the whole reason p95 exists
    /// alongside `mean`/`worst`.
    #[test]
    fn p95_catches_a_stall_that_p50_does_not() {
        let origin = Instant::now();
        let mut stats = FrameStats::default();

        stats.record(origin); // starts the clock; the first call contributes no sample
        let mut elapsed_ms = 0u64;
        for _ in 0..9 {
            elapsed_ms += 10;
            assert_eq!(stats.record(at(origin, elapsed_ms)), None);
        }

        // The tenth gap is a 910 ms stall, which is also what pushes the window's elapsed
        // time past the 1 s reporting boundary.
        let summary = stats
            .record(at(origin, 1_000))
            .expect("the interval reports once 1s has elapsed");

        assert_eq!(summary.frames, 10);
        assert_eq!(summary.worst, Duration::from_millis(910));
        assert_eq!(summary.p50, Duration::from_millis(10));
        assert_eq!(summary.p95, Duration::from_millis(910));
    }

    /// The percentile helper itself, independent of [`FrameStats`]'s bookkeeping: nearest-rank
    /// on a small, hand-checkable sample set.
    #[test]
    fn percentile_uses_nearest_rank() {
        let samples: Vec<Duration> = [10, 20, 30, 40, 50]
            .into_iter()
            .map(Duration::from_millis)
            .collect();

        assert_eq!(percentile(&samples, 1), Duration::from_millis(10));
        assert_eq!(percentile(&samples, 50), Duration::from_millis(30));
        assert_eq!(percentile(&samples, 95), Duration::from_millis(50));
        assert_eq!(percentile(&samples, 100), Duration::from_millis(50));
    }
}
