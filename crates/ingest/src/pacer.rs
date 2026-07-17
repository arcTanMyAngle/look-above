//! Minimum spacing between requests to a source that meters by rate, not credits.
//!
//! The keyless fallbacks (airplanes.live, adsb.lol) are community-run: no ledger, no
//! account, just a documented request rate and the expectation that we honor it. The
//! poller's cadence floor (item 1.7, 5 s) is a *scheduling* choice and could change; this
//! limit is the source's own, so it rides with the adapter — whatever the caller does, two
//! requests cannot leave less than the interval apart (privacy rule 1.3: never exceed
//! documented limits).

use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// Enforces a minimum interval between successive [`pause`](Pacer::pause) returns.
#[derive(Debug)]
pub struct Pacer {
    interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl Pacer {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Mutex::new(None),
        }
    }

    /// The configured spacing — what an adapter's wiring test asserts against.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Returns once at least the interval has passed since the previous return.
    ///
    /// The first call never waits. The lock is held across the sleep on purpose:
    /// concurrent callers queue rather than all waking at the same deadline, so the
    /// spacing holds between *any* two requests, not just consecutive ones from a single
    /// task. A deadline already in the past costs nothing (`sleep_until` returns
    /// immediately), so a caller slower than the interval is never delayed.
    pub async fn pause(&self) {
        let mut last = self.last.lock().await;
        if let Some(previous) = *last {
            tokio::time::sleep_until(previous + self.interval).await;
        }
        *last = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All under `start_paused`: tokio's clock advances only when every task is idle, so
    // the sleeps are virtual but the *schedule* is real — a two-second gap asserts as an
    // exact two seconds of virtual time, instantly and without flakes. (This is also why
    // the adapter's own tests don't re-test pacing over wiremock: paused time plus real
    // sockets lets the auto-advancing clock fire the 10 s request timeout while a reply
    // is genuinely in flight.)

    const INTERVAL: Duration = Duration::from_secs(2);

    #[tokio::test(start_paused = true)]
    async fn the_first_pass_does_not_wait() {
        let pacer = Pacer::new(INTERVAL);
        let start = Instant::now();
        pacer.pause().await;
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_rapid_second_pass_waits_out_the_full_interval() {
        let pacer = Pacer::new(INTERVAL);
        let start = Instant::now();
        pacer.pause().await;
        pacer.pause().await;
        assert_eq!(start.elapsed(), INTERVAL);
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_slower_than_the_interval_is_never_delayed() {
        let pacer = Pacer::new(INTERVAL);
        pacer.pause().await;
        tokio::time::sleep(Duration::from_secs(3)).await;

        let start = Instant::now();
        pacer.pause().await;
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    /// The lock-across-the-sleep property: three concurrent requests leave spaced, not
    /// bunched at the first deadline.
    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_are_spaced_not_released_together() {
        let pacer = Pacer::new(INTERVAL);
        let start = Instant::now();
        tokio::join!(pacer.pause(), pacer.pause(), pacer.pause());
        assert_eq!(
            start.elapsed(),
            INTERVAL * 2,
            "three passes must span two full intervals"
        );
    }

    /// A partially elapsed interval is topped up, not restarted or ignored.
    #[tokio::test(start_paused = true)]
    async fn a_partial_wait_is_only_topped_up() {
        let pacer = Pacer::new(INTERVAL);
        pacer.pause().await;
        tokio::time::sleep(Duration::from_millis(1500)).await;

        let start = Instant::now();
        pacer.pause().await;
        assert_eq!(start.elapsed(), Duration::from_millis(500));
    }
}
