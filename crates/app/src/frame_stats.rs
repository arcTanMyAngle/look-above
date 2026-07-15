//! Frame-time accounting for the debug log.
//!
//! A stub, per M0 item 0.6: it gives the window a heartbeat and makes an obviously wrong
//! frame time visible now rather than at M2. The real thing — a p95 overlay drawn on screen
//! and held to the 16.6 ms budget in docs/11 §M2 — replaces it there.

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
    frames: u32,
    total: Duration,
    worst: Duration,
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
            self.frames += 1;
            self.total += frame_time;
            self.worst = self.worst.max(frame_time);
        }

        let elapsed = now.saturating_duration_since(started);
        if elapsed < REPORT_INTERVAL || self.frames == 0 {
            return None;
        }

        let summary = FrameSummary {
            frames: self.frames,
            elapsed,
            mean: self.total / self.frames,
            worst: self.worst,
        };

        self.window_start = Some(now);
        self.frames = 0;
        self.total = Duration::ZERO;
        self.worst = Duration::ZERO;

        Some(summary)
    }
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
}
