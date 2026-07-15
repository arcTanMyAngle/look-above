//! Retry scheduling for transient source failures.
//!
//! The schedule is docs/09's: base 5 s, doubling, capped at 5 min, jittered, and
//! never sooner than a `Retry-After` the source sent us.
//!
//! [`retry_delay`] is pure — the jitter factor is an argument — so the schedule is
//! testable without a clock or an RNG. [`next_retry_delay`] is the thin wrapper that
//! draws the factor.

use std::time::Duration;

/// The first retry waits ~this long (docs/09).
pub const BASE_DELAY: Duration = Duration::from_secs(5);

/// Ceiling on the exponential term, however many attempts have failed (docs/09).
///
/// A `Retry-After` longer than this is still honored in full — see [`retry_delay`].
pub const MAX_DELAY: Duration = Duration::from_mins(5);

/// Doubling stops here; 2^16 × 5 s already exceeds [`MAX_DELAY`] by three orders of
/// magnitude, and this keeps the shift away from overflow.
const MAX_SHIFT: u32 = 16;

/// How long to wait before retry number `attempt` (0-based: 0 is the first retry).
///
/// `jitter` is the caller's random factor in `[0, 1]`; values outside that range (and
/// NaN) are clamped rather than rejected, since a bad factor must never panic a poller.
///
/// `retry_after` is treated as a *floor*, not an instruction to retry at exactly that
/// moment: `Retry-After` says "not before", so waiting longer always honors it while
/// waiting less never does. Taking the max with the exponential term means repeated 429s
/// still escalate instead of pinning us to whatever the server last suggested. A
/// `Retry-After` beyond [`MAX_DELAY`] is honored in full — the cap governs our own
/// guesswork, not the source's explicit request (CLAUDE.md: never exceed documented
/// rate limits).
pub fn retry_delay(attempt: u32, retry_after: Option<Duration>, jitter: f64) -> Duration {
    let backoff = jittered_backoff(attempt, jitter);
    match retry_after {
        Some(floor) => floor.max(backoff),
        None => backoff,
    }
}

/// [`retry_delay`] with the jitter factor drawn for you.
pub fn next_retry_delay(attempt: u32, retry_after: Option<Duration>) -> Duration {
    retry_delay(attempt, retry_after, fastrand::f64())
}

/// Equal jitter: the delay lands in `[d/2, d]` for the capped exponential `d`.
///
/// Full jitter (`[0, d]`) is the more common recipe, but it can schedule a retry
/// milliseconds after a 429 — the one response that means *stop asking*. Keeping half the
/// delay fixed puts a floor under every retry while still spreading them out.
fn jittered_backoff(attempt: u32, jitter: f64) -> Duration {
    let jitter = if jitter.is_nan() {
        0.0
    } else {
        jitter.clamp(0.0, 1.0)
    };
    let doublings = 1_u32 << attempt.min(MAX_SHIFT);
    let capped = BASE_DELAY.saturating_mul(doublings).min(MAX_DELAY);
    capped.mul_f64(0.5 + 0.5 * jitter)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lower and upper bound of the jitter window for `attempt`.
    fn window(attempt: u32) -> (Duration, Duration) {
        (
            retry_delay(attempt, None, 0.0),
            retry_delay(attempt, None, 1.0),
        )
    }

    #[test]
    fn first_retry_waits_about_the_base_delay() {
        let (low, high) = window(0);
        assert_eq!(high, BASE_DELAY);
        assert_eq!(low, BASE_DELAY / 2);
    }

    #[test]
    fn delay_doubles_per_attempt_until_the_cap() {
        for attempt in 0..6 {
            let (_, high) = window(attempt);
            assert_eq!(high, BASE_DELAY * (1 << attempt), "attempt {attempt}");
        }
    }

    #[test]
    fn exponential_term_never_exceeds_the_cap() {
        // Including the attempts past MAX_SHIFT, where a naive shift would overflow.
        for attempt in [6, 7, 16, 17, 31, 32, 64, u32::MAX] {
            let (_, high) = window(attempt);
            assert_eq!(high, MAX_DELAY, "attempt {attempt}");
        }
    }

    #[test]
    fn jitter_stays_inside_the_upper_half_of_the_window() {
        for attempt in 0..8 {
            let (low, high) = window(attempt);
            assert_eq!(low, high / 2, "attempt {attempt}");
            for step in 0..=10 {
                let delay = retry_delay(attempt, None, f64::from(step) / 10.0);
                assert!(
                    delay >= low && delay <= high,
                    "attempt {attempt}, {delay:?}"
                );
            }
        }
    }

    #[test]
    fn a_bad_jitter_factor_clamps_instead_of_panicking() {
        let (low, high) = window(3);
        for bad in [f64::NAN, -1.0, f64::NEG_INFINITY, 2.0, f64::INFINITY] {
            let delay = retry_delay(3, None, bad);
            assert!(delay >= low && delay <= high, "jitter {bad}");
        }
    }

    #[test]
    fn retry_after_is_a_floor_never_a_shortcut() {
        // Longer than the backoff: we wait as told.
        let long = Duration::from_mins(2);
        assert_eq!(retry_delay(0, Some(long), 1.0), long);

        // Shorter than the backoff: we wait the backoff, which still honors "not before".
        let short = Duration::from_secs(1);
        assert_eq!(retry_delay(0, Some(short), 1.0), BASE_DELAY);
    }

    #[test]
    fn retry_after_beyond_the_cap_is_honored_in_full() {
        let hour = Duration::from_hours(1);
        assert!(hour > MAX_DELAY);
        assert_eq!(retry_delay(9, Some(hour), 1.0), hour);
    }

    #[test]
    fn drawn_jitter_lands_in_the_window() {
        let (low, high) = window(2);
        for _ in 0..1_000 {
            let delay = next_retry_delay(2, None);
            assert!(delay >= low && delay <= high, "{delay:?}");
        }
    }
}
