//! The daily credit ledger and the cadence controller — how often `OpenSky` may be polled
//! without overrunning its allowance.
//!
//! `OpenSky` bills a bbox query 1–4 credits ([`crate::opensky::states::credit_cost`]) against
//! a **4,000/day** allowance, and privacy rule 1.3 asks us to stay under **80%** of any
//! documented limit — so the number this module actually defends is [`DAILY_BUDGET`] = 3,200
//! credits per UTC day. Two jobs follow from that:
//!
//! - **The ledger** ([`CreditLedger`]) counts credits spent on the current UTC day and rolls
//!   the counter back to zero when the day turns over. M1 keeps it in memory; item 1.11 will
//!   persist it in `source_status.credits_used_today` and restore it through
//!   [`CreditLedger::restored`]. That seam is the whole reason this is a small owned struct
//!   rather than a reach into `store` (which does not exist yet).
//!
//! - **The cadence controller** ([`poll_interval`]) turns "how much budget is left, and how
//!   much of the day is left" into a poll interval. It spreads the *remaining* budget evenly
//!   across the *remaining* seconds of the day — pro-rating — so a cycle is scheduled just
//!   often enough to reach midnight as the allowance runs out, never sooner. The result is
//!   clamped to [[`MIN_INTERVAL`], [`MAX_INTERVAL`]] (5 s … 60 s).
//!
//! **Why even-spread is the pro-rating the plan asks for.** On the pro-rata line — credits
//! spent equal to `DAILY_BUDGET × (fraction of day elapsed)` — the interval works out to a
//! constant `86400 × cost / 3200 ≈ 27 s` per credit, the steady state that just fits the
//! budget. Spend *slower* than pro-rata and more budget survives into less day, so the
//! interval shrinks toward the 5 s floor (poll faster, we have savings). Spend *faster* and
//! the interval grows toward the 60 s ceiling (poll slower, we are ahead of budget). That is
//! exactly "the poll interval widens as the budget tightens", and the pro-rated target falls
//! straight out of the same arithmetic ([`prorated_target`]).
//!
//! Everything here is a **pure function of `(spent, cost, now)`** — the design note's
//! `ledger + bbox + clock → next_poll_at`. `now` is wall-clock [`UnixSeconds`] (the day
//! boundary is a *calendar* fact), not the monotonic `Instant` the token refresh uses: a
//! duration cannot roll over at midnight, and a wall-clock correction that shifts the day is
//! the behaviour we want, not a bug to guard against.

use std::time::Duration;

use look_above_core::types::UnixSeconds;

/// `OpenSky`'s daily credit allowance for a registered account (authorized-sources skill).
pub const DAILY_ALLOWANCE: u32 = 4_000;

/// The fraction of any documented allowance we hold ourselves under (privacy rule 1.3).
pub const TARGET_FRACTION: f64 = 0.8;

/// The credits we permit ourselves per UTC day: 80% of [`DAILY_ALLOWANCE`], the margin rule
/// 1.3 requires. `const` arithmetic is not available for the `f64` multiply, so this is the
/// computed value and [`tests::the_budget_is_eighty_percent_of_the_allowance`] pins it.
pub const DAILY_BUDGET: u32 = 3_200;

/// The fastest we ever poll — the floor on the interval, reached when the budget is loose
/// relative to the day remaining.
pub const MIN_INTERVAL: Duration = Duration::from_secs(5);

/// The slowest the *cadence* ever asks for — the ceiling, reached when the budget is tight.
/// It is not a spend guarantee on its own (a 4-credit query every 60 s would still overrun a
/// day); the hard stop is [`can_afford`], which refuses the cycle that would cross the cap.
pub const MAX_INTERVAL: Duration = Duration::from_mins(1);

/// Seconds in a day — the UTC day the ledger and the cadence are keyed to.
const SECONDS_PER_DAY: i64 = 86_400;

/// The same span as `f64`, for the ratios below. Written out rather than cast so the
/// day-length denominator is not itself a lossy conversion.
const SECONDS_PER_DAY_F64: f64 = 86_400.0;

/// A count of seconds within a day as `f64`.
///
/// Every value fed here is in `0..=SECONDS_PER_DAY`, which `f64` represents exactly, so the
/// cast the pedantic lint flags cannot actually lose precision — the `allow` records that.
#[allow(clippy::cast_precision_loss)]
fn day_seconds_as_f64(seconds: i64) -> f64 {
    seconds as f64
}

/// The UTC day `now` falls in, counted in whole days since the epoch.
///
/// `div_euclid`, not `/`: it floors toward negative infinity, so the day index is monotonic
/// across the epoch rather than folding either side of it onto day 0.
fn day_index(now: UnixSeconds) -> i64 {
    now.0.div_euclid(SECONDS_PER_DAY)
}

/// Seconds elapsed since the start of `now`'s UTC day, in `0..SECONDS_PER_DAY`.
///
/// `rem_euclid` for the same reason [`day_index`] uses `div_euclid`: a non-negative
/// remainder even for pre-epoch instants, so the fraction below is always in `[0, 1)`.
fn seconds_into_day(now: UnixSeconds) -> i64 {
    now.0.rem_euclid(SECONDS_PER_DAY)
}

/// Seconds from `now` to the next UTC midnight, in `1..=SECONDS_PER_DAY`.
///
/// Never zero: at midnight exactly a whole fresh day lies ahead, which is what keeps the
/// division in [`poll_interval`] free of a zero denominator.
fn seconds_until_midnight(now: UnixSeconds) -> i64 {
    SECONDS_PER_DAY - seconds_into_day(now)
}

/// How far through its UTC day `now` is, in `[0.0, 1.0)`.
pub fn fraction_of_day_elapsed(now: UnixSeconds) -> f64 {
    day_seconds_as_f64(seconds_into_day(now)) / SECONDS_PER_DAY_F64
}

/// The credits we intend to have spent by `now`, spreading [`DAILY_BUDGET`] evenly across the
/// UTC day. Actual spend below this is "ahead of budget"; above it is "behind".
///
/// The cadence does not read this — it works from remaining budget and remaining time, which
/// is the same line seen from the other end — but the poller and the headless readout (item
/// 1.12) want the target itself as an at-a-glance health number.
pub fn prorated_target(now: UnixSeconds) -> f64 {
    f64::from(DAILY_BUDGET) * fraction_of_day_elapsed(now)
}

/// Credits still available before today's cap, given what has been spent.
pub fn remaining_budget(spent_today: u32) -> u32 {
    DAILY_BUDGET.saturating_sub(spent_today)
}

/// Whether a `cost`-credit query fits inside today's remaining budget.
///
/// This is the hard stop rule 1.3 turns on — the poller must not run a cycle this refuses,
/// because a single overrun is exactly the thing the 80% margin exists to prevent. An
/// unmetered query (`cost == 0`) is always affordable; the budget governs credits, and a
/// source that spends none is bounded by its own pacer, not by this ledger.
pub fn can_afford(spent_today: u32, cost: u32) -> bool {
    // A 0-credit query changes nothing, so it is affordable whatever has been spent — the
    // early return, not the comparison, is what carries that: a metered source is always
    // held at or under the cap, but an unmetered one keeps no credit count to compare.
    cost == 0 || spent_today.saturating_add(cost) <= DAILY_BUDGET
}

/// How long to wait before the next `cost`-credit poll, given today's spend and the time.
///
/// The remaining budget spread across the remaining day: `seconds_until_midnight ÷ polls we
/// can still afford`, clamped to [[`MIN_INTERVAL`], [`MAX_INTERVAL`]]. See the module docs for
/// why that is the pro-rated cadence rather than a separate calculation.
///
/// The edges:
/// - `cost == 0` (unmetered) — the credit budget imposes nothing, so poll at the floor and
///   let the source's own pacer widen it if it must.
/// - no affordable poll left (`remaining < cost`) — nothing to schedule; return the ceiling
///   so the poller idles slowly and picks the budget back up at the midnight rollover. The
///   cycle itself is stopped by [`can_afford`], not by this interval.
pub fn poll_interval(spent_today: u32, cost: u32, now: UnixSeconds) -> Duration {
    if cost == 0 {
        return MIN_INTERVAL;
    }

    let polls_remaining = remaining_budget(spent_today) / cost;
    if polls_remaining == 0 {
        return MAX_INTERVAL;
    }

    // `seconds_until_midnight` is in `1..=86_400` and `polls_remaining` in `1..=3_200`, both
    // exact in `f64`; the quotient is finite and positive, so the clamp — not `from_secs_f64`
    // — is what bounds it.
    let ideal_secs = day_seconds_as_f64(seconds_until_midnight(now)) / f64::from(polls_remaining);
    let clamped = ideal_secs.clamp(min_secs(), max_secs());
    Duration::from_secs_f64(clamped)
}

fn min_secs() -> f64 {
    MIN_INTERVAL.as_secs_f64()
}

fn max_secs() -> f64 {
    MAX_INTERVAL.as_secs_f64()
}

/// The budget's verdict on one prospective poll cycle — everything the poller (item 1.8) and
/// the headless readout (item 1.12) need from a single [`CreditLedger::decide`] call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetDecision {
    /// Whether this cycle may run at all ([`can_afford`]). When `false`, the poller must not
    /// fetch — it idles for `interval` and asks again.
    pub affordable: bool,
    /// How long to wait before the next cycle ([`poll_interval`]): 5 s … 60 s.
    pub interval: Duration,
    /// Credits already spent on this UTC day.
    pub spent_today: u32,
    /// Credits left before today's 80% cap ([`remaining_budget`]).
    pub remaining: u32,
}

/// A day's credit spend for a single metered source.
///
/// Small and owned rather than a handle into `store`: `source_status` does not exist until
/// item 1.11, and the seam this module commits to is "in-memory now, persisted then". The
/// counter is scoped to one UTC day and resets itself when [`record`](Self::record) first sees
/// a later day — the poller never has to notice midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditLedger {
    /// The UTC day the counter belongs to. A read or write for a different day treats the
    /// count as zero (see [`spent_today`](Self::spent_today)).
    day: i64,
    spent: u32,
}

impl CreditLedger {
    /// An empty ledger, before any day is known.
    ///
    /// `day` is [`i64::MIN`] so the first [`record`](Self::record) rolls it onto the real day
    /// and any [`spent_today`](Self::spent_today) before then reads zero — an untouched ledger
    /// has spent nothing, on whatever day it is asked about.
    pub const fn new() -> Self {
        Self {
            day: i64::MIN,
            spent: 0,
        }
    }

    /// A ledger restored from persistence (item 1.11): `spent` credits already used on the
    /// UTC day of `now`. If the persisted figure is from an earlier day, the next read or
    /// write discards it — a stored count only counts against the day it was made on.
    pub fn restored(spent: u32, now: UnixSeconds) -> Self {
        Self {
            day: day_index(now),
            spent,
        }
    }

    /// Credits spent so far on the UTC day containing `now`.
    ///
    /// A pure read: it never mutates, so a stale day simply reads as zero rather than being
    /// rolled over here. The rollover is [`record`](Self::record)'s job, at the point a new
    /// spend is actually booked.
    pub fn spent_today(&self, now: UnixSeconds) -> u32 {
        if day_index(now) == self.day {
            self.spent
        } else {
            0
        }
    }

    /// Books `cost` credits spent at `now`, rolling the counter over first if the UTC day has
    /// turned since the last write. `saturating_add` so a runaway count pins at [`u32::MAX`]
    /// instead of wrapping to a tiny number that would read as budget restored.
    pub fn record(&mut self, cost: u32, now: UnixSeconds) {
        let today = day_index(now);
        if today == self.day {
            self.spent = self.spent.saturating_add(cost);
        } else {
            self.day = today;
            self.spent = cost;
        }
    }

    /// The full verdict for a `cost`-credit query at `now`: affordability and the interval to
    /// the next cycle, plus the spend figures behind them.
    pub fn decide(&self, cost: u32, now: UnixSeconds) -> BudgetDecision {
        let spent_today = self.spent_today(now);
        BudgetDecision {
            affordable: can_afford(spent_today, cost),
            interval: poll_interval(spent_today, cost, now),
            spent_today,
            remaining: remaining_budget(spent_today),
        }
    }
}

impl Default for CreditLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2021-01-01 00:00:00 UTC — `1_609_459_200`, an exact multiple of [`SECONDS_PER_DAY`], so
    /// a fixed and human-checkable midnight to anchor the day-boundary arithmetic. Noon is
    /// half a day on from it.
    const MIDNIGHT: UnixSeconds = UnixSeconds(1_609_459_200);
    const NOON: UnixSeconds = UnixSeconds(1_609_459_200 + SECONDS_PER_DAY / 2);

    fn at(day_midnight: UnixSeconds, into_day: i64) -> UnixSeconds {
        UnixSeconds(day_midnight.0 + into_day)
    }

    // ---- The budget constant ------------------------------------------------------------------

    #[test]
    fn the_budget_is_eighty_percent_of_the_allowance() {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let computed = (f64::from(DAILY_ALLOWANCE) * TARGET_FRACTION) as u32;
        assert_eq!(
            DAILY_BUDGET, computed,
            "the defended budget must be 80% of OpenSky's 4,000/day (privacy rule 1.3)"
        );
        assert_eq!(DAILY_BUDGET, 3_200);
    }

    // ---- Day arithmetic -----------------------------------------------------------------------

    #[test]
    fn a_utc_day_is_a_single_index_from_its_first_to_its_last_second() {
        let day = day_index(MIDNIGHT);
        assert_eq!(day_index(at(MIDNIGHT, 0)), day, "the first second");
        assert_eq!(
            day_index(at(MIDNIGHT, SECONDS_PER_DAY - 1)),
            day,
            "the last"
        );
        assert_eq!(
            day_index(at(MIDNIGHT, SECONDS_PER_DAY)),
            day + 1,
            "the next midnight is the next day"
        );
    }

    #[test]
    fn the_day_boundary_arithmetic_survives_pre_epoch_instants() {
        // A second before the epoch is the previous day, not day 0 — the reason for the
        // Euclidean division. Nothing polls in 1969, but the total functions must not fold.
        assert_eq!(day_index(UnixSeconds(-1)), -1);
        assert_eq!(seconds_into_day(UnixSeconds(-1)), SECONDS_PER_DAY - 1);
        assert_eq!(seconds_until_midnight(UnixSeconds(-1)), 1);
    }

    #[test]
    fn midnight_has_a_full_day_ahead_and_none_elapsed() {
        assert_eq!(seconds_into_day(MIDNIGHT), 0);
        assert_eq!(seconds_until_midnight(MIDNIGHT), SECONDS_PER_DAY);
        assert!((fraction_of_day_elapsed(MIDNIGHT) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn noon_is_halfway_through_the_day() {
        assert_eq!(seconds_until_midnight(NOON), SECONDS_PER_DAY / 2);
        assert!((fraction_of_day_elapsed(NOON) - 0.5).abs() < 1e-12);
    }

    // ---- The pro-rated target -----------------------------------------------------------------

    #[test]
    fn the_prorated_target_tracks_the_time_of_day() {
        assert!((prorated_target(MIDNIGHT) - 0.0).abs() < 1e-9);
        assert!((prorated_target(NOON) - f64::from(DAILY_BUDGET) / 2.0).abs() < 1e-9);
        let almost_midnight = at(MIDNIGHT, SECONDS_PER_DAY - 1);
        assert!(
            prorated_target(almost_midnight) < f64::from(DAILY_BUDGET),
            "the target only reaches the full budget at the next midnight, never before"
        );
    }

    // ---- Affordability: the hard cap ----------------------------------------------------------

    #[test]
    fn a_query_that_fits_under_the_cap_is_affordable_and_one_that_crosses_it_is_not() {
        assert!(can_afford(0, 4));
        assert!(
            can_afford(DAILY_BUDGET - 4, 4),
            "landing exactly on the cap is fine"
        );
        assert!(
            !can_afford(DAILY_BUDGET - 3, 4),
            "a cycle that would cross 3,200 must be refused — the 80% margin is the point"
        );
        assert!(
            !can_afford(DAILY_BUDGET, 1),
            "spent to the cap, nothing more fits"
        );
    }

    #[test]
    fn an_unmetered_query_is_always_affordable() {
        assert!(
            can_afford(DAILY_BUDGET, 0),
            "a 0-credit query spends nothing"
        );
        assert!(can_afford(u32::MAX, 0));
    }

    #[test]
    fn remaining_budget_never_underflows() {
        assert_eq!(remaining_budget(0), DAILY_BUDGET);
        assert_eq!(remaining_budget(DAILY_BUDGET), 0);
        assert_eq!(
            remaining_budget(DAILY_BUDGET + 100),
            0,
            "an over-spend reads as zero remaining, never a huge wrapped number"
        );
    }

    // ---- The cadence controller ---------------------------------------------------------------

    #[test]
    fn an_unmetered_source_polls_at_the_floor() {
        // The budget says nothing about a source that costs no credits; its pacer, not this,
        // sets the real spacing.
        assert_eq!(poll_interval(0, 0, NOON), MIN_INTERVAL);
    }

    #[test]
    fn on_the_prorata_line_the_cadence_is_the_steady_state() {
        // Spent exactly the pro-rated target at noon (half the budget), the interval is the
        // ~27 s/credit steady state that just fills the day.
        let spent = DAILY_BUDGET / 2;
        let interval = poll_interval(spent, 1, NOON);
        // 43_200 s ÷ 1_600 polls = 27 s.
        assert_eq!(interval, Duration::from_secs(27));
    }

    #[test]
    fn spending_ahead_of_budget_widens_the_interval_toward_the_ceiling() {
        // At noon, but nearly all the budget already gone: little left for the rest of the day,
        // so the interval stretches to (and is clamped at) the ceiling.
        let spent = DAILY_BUDGET - 10;
        assert_eq!(poll_interval(spent, 1, NOON), MAX_INTERVAL);
    }

    #[test]
    fn saved_budget_late_in_the_day_tightens_the_interval_to_the_floor() {
        // One minute before midnight with most of the budget unspent: the even spread wants
        // sub-second polling, and the floor is what stops it.
        let almost_midnight = at(MIDNIGHT, SECONDS_PER_DAY - 60);
        assert_eq!(poll_interval(100, 1, almost_midnight), MIN_INTERVAL);
    }

    #[test]
    fn a_fresh_budget_at_the_start_of_the_day_polls_within_budget_not_at_the_floor() {
        // The steady state, not the floor: at cost 1 a whole fresh day is 86_400 ÷ 3_200 = 27 s.
        // Polling at the 5 s floor here would burn the day's budget in about four hours.
        assert_eq!(poll_interval(0, 1, MIDNIGHT), Duration::from_secs(27));
    }

    #[test]
    fn a_dearer_query_widens_the_interval() {
        // Four credits a poll, fresh budget: 800 affordable polls over the day want 108 s each,
        // clamped to the 60 s ceiling. The interval never shrinks as cost rises.
        assert_eq!(poll_interval(0, 4, MIDNIGHT), MAX_INTERVAL);
        // And a 2-credit query sits between the 1- and 4-credit cases at the same spend.
        let two = poll_interval(0, 2, MIDNIGHT);
        assert!(
            two > poll_interval(0, 1, MIDNIGHT) && two <= MAX_INTERVAL,
            "cost 2 must not be cheaper to poll than cost 1"
        );
    }

    #[test]
    fn an_exhausted_budget_returns_the_ceiling_rather_than_dividing_by_zero() {
        // Nothing affordable left: the cadence yields the ceiling so the poller idles slowly,
        // and `can_afford` — not the interval — is what stops the cycle.
        assert_eq!(poll_interval(DAILY_BUDGET, 1, NOON), MAX_INTERVAL);
        assert_eq!(
            poll_interval(DAILY_BUDGET - 2, 4, NOON),
            MAX_INTERVAL,
            "two credits left cannot afford a four-credit poll"
        );
        assert!(!can_afford(DAILY_BUDGET, 1));
    }

    #[test]
    fn every_interval_stays_within_the_documented_bounds() {
        // Sweep spend and cost across a fixed day: the clamp must hold on every combination.
        for into_day in [0, 1, SECONDS_PER_DAY / 2, SECONDS_PER_DAY - 1] {
            let now = at(MIDNIGHT, into_day);
            for spent in [0, 1, 800, DAILY_BUDGET / 2, DAILY_BUDGET - 1, DAILY_BUDGET] {
                for cost in 1..=4 {
                    let interval = poll_interval(spent, cost, now);
                    assert!(
                        (MIN_INTERVAL..=MAX_INTERVAL).contains(&interval),
                        "interval {interval:?} out of bounds at into_day={into_day}, \
                         spent={spent}, cost={cost}"
                    );
                }
            }
        }
    }

    // ---- The ledger ---------------------------------------------------------------------------

    #[test]
    fn a_new_ledger_has_spent_nothing_on_any_day() {
        let ledger = CreditLedger::new();
        assert_eq!(ledger.spent_today(MIDNIGHT), 0);
        assert_eq!(ledger.spent_today(NOON), 0);
        assert_eq!(CreditLedger::default(), CreditLedger::new());
    }

    #[test]
    fn recording_accumulates_within_a_day() {
        let mut ledger = CreditLedger::new();
        ledger.record(1, NOON);
        ledger.record(2, at(MIDNIGHT, SECONDS_PER_DAY / 2 + 100));
        assert_eq!(ledger.spent_today(NOON), 3);
    }

    #[test]
    fn the_counter_resets_when_the_utc_day_turns_over() {
        let mut ledger = CreditLedger::new();
        ledger.record(1_000, NOON);
        assert_eq!(ledger.spent_today(NOON), 1_000);

        // A read for the next day sees nothing — the stored count belongs to the day it was
        // made on and does not leak forward.
        let next_noon = at(MIDNIGHT, SECONDS_PER_DAY + SECONDS_PER_DAY / 2);
        assert_eq!(ledger.spent_today(next_noon), 0);

        // And the first write on the new day starts a fresh count, not 1_000 + cost.
        ledger.record(5, next_noon);
        assert_eq!(ledger.spent_today(next_noon), 5);
        assert_eq!(
            ledger.spent_today(NOON),
            0,
            "the old day is now the stale one and reads zero"
        );
    }

    #[test]
    fn a_restored_ledger_carries_todays_spend_but_not_an_earlier_days() {
        let restored = CreditLedger::restored(500, NOON);
        assert_eq!(restored.spent_today(NOON), 500);

        // Persisted yesterday, restored today: the stale figure must not count.
        let tomorrow = at(MIDNIGHT, SECONDS_PER_DAY + 10);
        assert_eq!(CreditLedger::restored(500, NOON).spent_today(tomorrow), 0);
    }

    #[test]
    fn recording_saturates_rather_than_wrapping() {
        let mut ledger = CreditLedger::restored(u32::MAX, NOON);
        ledger.record(10, NOON);
        assert_eq!(
            ledger.spent_today(NOON),
            u32::MAX,
            "a runaway count must pin at the max, never wrap to a small number that reads as \
             budget restored"
        );
    }

    // ---- The ledger's combined verdict --------------------------------------------------------

    #[test]
    fn decide_bundles_affordability_the_interval_and_the_spend_figures() {
        let mut ledger = CreditLedger::new();
        ledger.record(DAILY_BUDGET / 2, NOON);

        let decision = ledger.decide(1, NOON);
        assert_eq!(
            decision,
            BudgetDecision {
                affordable: true,
                interval: Duration::from_secs(27),
                spent_today: DAILY_BUDGET / 2,
                remaining: DAILY_BUDGET / 2,
            }
        );
    }

    #[test]
    fn decide_refuses_and_idles_when_the_budget_is_spent() {
        let mut ledger = CreditLedger::new();
        ledger.record(DAILY_BUDGET, NOON);

        let decision = ledger.decide(1, NOON);
        assert!(!decision.affordable, "nothing fits under the cap");
        assert_eq!(
            decision.interval, MAX_INTERVAL,
            "idle slowly until the reset"
        );
        assert_eq!(decision.remaining, 0);
    }

    #[test]
    fn decide_agrees_with_the_free_functions_it_composes() {
        let ledger = CreditLedger::restored(1_234, NOON);
        for cost in 0..=4 {
            let decision = ledger.decide(cost, NOON);
            assert_eq!(decision.spent_today, ledger.spent_today(NOON));
            assert_eq!(decision.affordable, can_afford(1_234, cost));
            assert_eq!(decision.interval, poll_interval(1_234, cost, NOON));
            assert_eq!(decision.remaining, remaining_budget(1_234));
        }
    }
}
