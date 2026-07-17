//! The poller — the loop that turns the adapters, the budget, and the failover chain into a
//! stream of position batches.
//!
//! Everything below it is stateless or pure: an adapter [`fetch`](LiveSource::fetch)es one
//! region, the [`budget`](crate::budget) prices and paces one cycle, the
//! [`allowlist`](crate::allowlist) refuses one host. The poller is where those become an
//! ongoing behaviour: *which* source to ask, *how often*, what to do when one fails, and how
//! to hand the result to the rest of the pipeline. It holds the only mutable ingest state
//! there is — the active source, the failure streak, and a [`CreditLedger`] per source.
//!
//! **The failover chain.** Sources are held in priority order — `OpenSky` primary, then the
//! keyless fallbacks (airplanes.live, adsb.lol) — and one is *active* at a time. A fetch error
//! is classified by [`SourceError::is_transient`]:
//!
//! - *transient* (`RateLimited`/`Network`/`Server`) — retry the same source with backoff
//!   ([`crate::http::backoff`]); only fail over after [`TRANSIENT_FAILOVER_THRESHOLD`]
//!   consecutive failures, because a single hiccup is not a dead source.
//! - *permanent but a real answer* (`Auth`/`Parse`/`Request`) — the identical request cannot
//!   succeed on a re-fetch, so fail over at once. A disabled `OpenSky` returns `Auth` and drops
//!   straight to the fallbacks this way.
//! - *our own refusal* (`Refused`) — an unauthorized host, or a global query to a point
//!   source. The next source would be asked the same wrong question, so this is **not** a
//!   failover: it holds and idles so the misconfiguration is noticed, not papered over.
//!
//! **Recovery.** A working fallback never errs, so nothing would ever pull us back to the
//! primary. [`PRIMARY_PROBE_INTERVAL`] is the fix: while failed over, the loop tries the
//! primary again every five minutes and switches back the moment it answers.
//!
//! **The budget.** Before every metered cycle the ledger's [`decide`](CreditLedger::decide)
//! both prices the cadence (how long to wait) and vetoes the cycle if it would cross the day's
//! cap ([`can_afford`](crate::budget::can_afford)). A vetoed cycle is *skipped* — no fetch, no
//! spend — and the poller idles until the budget recovers at the UTC-day rollover. Skipping is
//! deliberately not a failover: the fallbacks are for *failures*, and a primary that is merely
//! rationing its budget is not one (see the module note in `budget`).
//!
//! Two clocks appear, for the two reasons `budget` already draws the distinction: the ledger
//! reads a wall-clock [`WallClock`] because the day boundary is a calendar fact, while the
//! cadence sleeps and the probe timer use tokio's monotonic clock because "wait 27 s" and
//! "five minutes since the last probe" are elapsed-time facts.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use look_above_core::contracts::{LiveSource, RegionQuery};
use look_above_core::error::SourceError;
use look_above_core::types::{SourceId, StateVector, UnixSeconds};
use tokio::time::{Instant, sleep};

use crate::adsb_lol::AdsbLolSource;
use crate::airplanes_live::AirplanesLiveSource;
use crate::budget::{CreditLedger, MAX_INTERVAL, MIN_INTERVAL};
use crate::http::HttpClient;
use crate::http::backoff::next_retry_delay;
use crate::opensky::auth::OpenSkyAuth;
use crate::opensky::states::OpenSkySource;

/// The index of the primary source in the chain — the one the recovery probe targets.
pub const PRIMARY: usize = 0;

/// Consecutive *transient* failures on the active source before failing over. A single
/// timeout or 5xx is noise; three in a row is a source worth stepping away from. Permanent
/// errors do not wait for this — they fail over on the first.
pub const TRANSIENT_FAILOVER_THRESHOLD: u32 = 3;

/// How often, while failed over to a fallback, the loop re-probes the primary. Five minutes
/// keeps a recovered primary from being ignored for long without probing so often that a
/// metered primary's budget is nibbled away on probes alone.
pub const PRIMARY_PROBE_INTERVAL: Duration = Duration::from_mins(5);

/// A source of wall-clock time, injected so the credit ledger's day boundary is testable.
///
/// The poller's *other* clock — the cadence sleeps and the probe timer — is tokio's, so it is
/// virtual under `start_paused` and needs no injection. Only the ledger reads this one.
pub trait WallClock: fmt::Debug + Send + Sync {
    /// The current wall-clock time, in whole seconds since the Unix epoch.
    fn now(&self) -> UnixSeconds;
}

/// Wall-clock time from the operating system.
#[derive(Debug, Clone, Copy)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now(&self) -> UnixSeconds {
        // Never panics: a pre-epoch clock reads as 0, and a clock past year 292-billion pins at
        // `i64::MAX`. Neither is reachable, but a poller must not crash on a wild system clock.
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            });
        UnixSeconds(seconds)
    }
}

/// One poll cycle's result, handed to the rest of the pipeline over the `crossbeam` channel.
///
/// Carries the cycle's own spend so the store writer (item 1.11) and the headless readout
/// (item 1.12) need not reach back into the poller's private ledger. An *empty* `states` is a
/// real answer — a quiet region — and is delivered like any other, so a consumer sees that the
/// cycle happened.
#[derive(Debug, Clone, PartialEq)]
pub struct PollBatch {
    /// Which source produced this batch — also the value the store stamps on each row.
    pub source: SourceId,
    /// The source's own time of applicability for the cycle (wall clock at fetch).
    pub fetched_at: UnixSeconds,
    /// The normalized positions, already trimmed to the requested region by the adapter.
    pub states: Vec<StateVector>,
    /// Credits this one cycle cost against `source`'s ledger — `0` for the unmetered fallbacks.
    pub credits_spent: u32,
    /// Running total spent on `source`'s ledger for the current UTC day.
    pub spent_today: u32,
}

/// The live-ingest loop: drives the active source at the budgeted cadence, fails over on
/// error, probes for the primary's recovery, and emits [`PollBatch`]es.
pub struct Poller {
    /// Sources in priority order; index [`PRIMARY`] is the one probed for recovery.
    sources: Vec<Box<dyn LiveSource>>,
    /// One ledger per source, aligned by index. Only metered sources (`OpenSky`) ever
    /// accumulate; an unmetered source's ledger stays at zero and never vetoes a cycle.
    ledgers: Vec<CreditLedger>,
    /// The region every cycle asks for. Fixed for M1 (bboxes ≤ ~1,000 km across); the camera
    /// will drive it in M2/M4.
    query: RegionQuery,
    /// Where finished batches go. A dropped receiver is the shutdown signal.
    sender: Sender<PollBatch>,
    clock: Arc<dyn WallClock>,
    /// The source currently being polled.
    active: usize,
    /// Consecutive transient failures on `active`, reset on any success or failover.
    transient_streak: u32,
}

/// Manual because `Box<dyn LiveSource>` is not `Debug` (the trait has no such supertrait, so
/// adapters need not carry one). Reports the shape without reaching into the trait objects:
/// the active source's `id` is the useful bit, and it is always in range.
impl fmt::Debug for Poller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Poller")
            .field(
                "sources",
                &self.sources.iter().map(|s| s.id()).collect::<Vec<_>>(),
            )
            .field("query", &self.query)
            .field("active", &self.active_source())
            .field("transient_streak", &self.transient_streak)
            .finish_non_exhaustive()
    }
}

impl Poller {
    /// A poller over `sources` (priority order, `sources[0]` primary) for `query`, delivering
    /// to `sender`.
    pub fn new(
        sources: Vec<Box<dyn LiveSource>>,
        query: RegionQuery,
        sender: Sender<PollBatch>,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        let ledgers = vec![CreditLedger::new(); sources.len()];
        Self {
            sources,
            ledgers,
            query,
            sender,
            clock,
            active: PRIMARY,
            transient_streak: 0,
        }
    }

    /// The authorized failover chain: `OpenSky` primary, then the keyless fallbacks in the
    /// documented order, sharing one client's connection pool (cloning [`HttpClient`] is cheap
    /// and shares the pool — one TLS setup, not three).
    pub fn with_default_chain(
        client: HttpClient,
        auth: OpenSkyAuth,
        query: RegionQuery,
        sender: Sender<PollBatch>,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        let sources: Vec<Box<dyn LiveSource>> = vec![
            Box::new(OpenSkySource::new(client.clone(), auth)),
            Box::new(AirplanesLiveSource::new(client.clone())),
            Box::new(AdsbLolSource::new(client)),
        ];
        Self::new(sources, query, sender, clock)
    }

    /// Which source is currently being polled — for the headless readout (item 1.12) and logs.
    #[must_use]
    pub fn active_source(&self) -> SourceId {
        self.sources[self.active].id()
    }

    /// Runs until the pipeline receiver is dropped.
    ///
    /// Each iteration either re-probes the primary (if we have failed over and the probe is
    /// due) or polls the active source, then sleeps for the interval that cycle asked for. The
    /// loop never returns on a *source* failure — a fully dead chain just idles and retries
    /// (the plan's "the app idles and retries; it never crashes"); only a gone receiver stops
    /// it.
    pub async fn run(mut self) {
        if self.sources.is_empty() {
            tracing::error!("poller started with no sources; nothing to do");
            return;
        }

        let mut last_probe = Instant::now();
        loop {
            let tick = if probe_due(self.active, last_probe.elapsed()) {
                last_probe = Instant::now();
                self.attempt_recovery().await
            } else {
                self.poll_active().await
            };

            if !tick.connected {
                tracing::info!("pipeline receiver dropped; poller shutting down");
                return;
            }
            sleep(tick.interval).await;
        }
    }

    /// One cycle on the active source, applying the failover policy to any error.
    async fn poll_active(&mut self) -> Tick {
        let index = self.active;
        match self.run_cycle(index).await {
            CycleOutcome::Fetched {
                count,
                credits,
                delivered,
                interval,
            } => {
                self.transient_streak = 0;
                tracing::debug!(source = %self.sources[index].id(), count, credits, "poll cycle");
                Tick {
                    interval,
                    connected: delivered,
                }
            }
            CycleOutcome::Skipped { interval } => {
                // Budget-capped, not failed: idle until the UTC-day reset restores it. Failing
                // over here would poll a redundant source while the primary's allowance is just
                // resting, so it does not.
                tracing::debug!(
                    source = %self.sources[index].id(),
                    "cycle skipped: budget exhausted; idling until the daily reset"
                );
                Tick {
                    interval,
                    connected: true,
                }
            }
            CycleOutcome::Failed { error } => Tick {
                interval: self.handle_error(index, &error),
                connected: true,
            },
        }
    }

    /// While failed over, try the primary again. On success, switch back and deliver its batch;
    /// otherwise stay on the fallback and try again at the next probe window.
    async fn attempt_recovery(&mut self) -> Tick {
        match self.run_cycle(PRIMARY).await {
            CycleOutcome::Fetched {
                count,
                credits,
                delivered,
                interval,
            } => {
                tracing::info!(count, credits, "primary recovered; switching back to it");
                self.active = PRIMARY;
                self.transient_streak = 0;
                Tick {
                    interval,
                    connected: delivered,
                }
            }
            // Up but budget-capped, or still down: keep serving from the fallback. A short
            // interval so the active fallback is polled again promptly rather than waiting out
            // the probe's own (possibly ceiling) cadence.
            CycleOutcome::Skipped { .. } | CycleOutcome::Failed { .. } => Tick {
                interval: MIN_INTERVAL,
                connected: true,
            },
        }
    }

    /// Prices, budget-gates, fetches, and (on success) records and delivers one cycle against
    /// `index`. Touches that source's ledger but never the failover state — the caller decides
    /// what an error means.
    async fn run_cycle(&mut self, index: usize) -> CycleOutcome {
        let now = self.clock.now();
        let cost = self.sources[index].cost(&self.query);

        // The hard cap (privacy rule 1.3): a cycle that would cross the day's budget is not
        // sent at all. `decide` also yields the interval to idle for meanwhile.
        let decision = self.ledgers[index].decide(cost, now);
        if !decision.affordable {
            return CycleOutcome::Skipped {
                interval: decision.interval,
            };
        }

        match self.sources[index].fetch(&self.query).await {
            Ok(states) => {
                self.ledgers[index].record(cost, now);
                let spent_today = self.ledgers[index].spent_today(now);
                let count = states.len();
                let batch = PollBatch {
                    source: self.sources[index].id(),
                    fetched_at: now,
                    states,
                    credits_spent: cost,
                    spent_today,
                };
                // A gone receiver means the pipeline shut down; report it so the loop stops.
                let delivered = self.sender.send(batch).is_ok();
                CycleOutcome::Fetched {
                    count,
                    credits: cost,
                    delivered,
                    // Priced off the post-spend ledger, so the next wait reflects what this
                    // cycle just cost.
                    interval: self.ledgers[index].decide(cost, now).interval,
                }
            }
            Err(error) => CycleOutcome::Failed { error },
        }
    }

    /// Applies the failover policy to a fetch error and returns how long to wait next.
    fn handle_error(&mut self, index: usize, error: &SourceError) -> Duration {
        match error_response(error, self.transient_streak + 1) {
            ErrorResponse::Backoff => {
                self.transient_streak += 1;
                let retry_after = match error {
                    SourceError::RateLimited { retry_after } => *retry_after,
                    _ => None,
                };
                // 0-based attempt: the first failure backs off ~`BASE_DELAY`. `retry_after` is
                // honored as a floor when the source sent one.
                let delay = next_retry_delay(self.transient_streak - 1, retry_after);
                tracing::warn!(
                    source = %self.sources[index].id(),
                    error = %error,
                    streak = self.transient_streak,
                    "transient source failure; backing off before retrying the same source"
                );
                delay
            }
            ErrorResponse::FailOver => {
                let from = self.sources[index].id();
                self.advance();
                let to = self.sources[self.active].id();
                tracing::warn!(%from, %to, error = %error, "failing over to the next source");
                // Try the new source promptly, but not instantly — a small floor keeps a fully
                // dead chain from spinning.
                MIN_INTERVAL
            }
            ErrorResponse::Hold => {
                tracing::error!(
                    source = %self.sources[index].id(),
                    error = %error,
                    "refused our own request — a bug or misconfiguration, not a source failure; \
                     not failing over"
                );
                MAX_INTERVAL
            }
        }
    }

    /// Advances to the next source in the chain, wrapping, and resets the failure streak.
    ///
    /// Wrapping keeps every source in rotation when the chain is unhealthy; the recovery probe
    /// is the *separate*, faster path back to the primary specifically.
    fn advance(&mut self) {
        self.active = (self.active + 1) % self.sources.len();
        self.transient_streak = 0;
    }
}

/// Whether the primary is due for a recovery probe: only when we have failed over, and only
/// once [`PRIMARY_PROBE_INTERVAL`] has elapsed since the last probe.
fn probe_due(active: usize, since_last_probe: Duration) -> bool {
    active != PRIMARY && since_last_probe >= PRIMARY_PROBE_INTERVAL
}

/// What a fetch error means for the active source. `transient_streak` counts consecutive
/// transient failures *including this one*.
fn error_response(error: &SourceError, transient_streak: u32) -> ErrorResponse {
    match error {
        // Our own pre-flight refusal (unauthorized host, or a global query to a point source).
        // The next source would be asked the same wrong question — hold, do not fail over, and
        // do not count it as a source failure.
        SourceError::Refused { .. } => ErrorResponse::Hold,
        // A transient upstream hiccup: retry the same source until the streak proves it is not
        // a hiccup, then fail over.
        transient if transient.is_transient() => {
            if transient_streak >= TRANSIENT_FAILOVER_THRESHOLD {
                ErrorResponse::FailOver
            } else {
                ErrorResponse::Backoff
            }
        }
        // Auth / Parse / Request: the same request will fail the same way, so there is nothing
        // to wait for — fail over at once.
        _ => ErrorResponse::FailOver,
    }
}

/// The result of one [`Poller::run_cycle`], before the failover policy is applied.
enum CycleOutcome {
    /// Fetched and delivered. `delivered` is false only when the receiver is gone.
    Fetched {
        count: usize,
        credits: u32,
        delivered: bool,
        interval: Duration,
    },
    /// The budget vetoed the cycle; nothing was fetched. Idle for `interval`.
    Skipped { interval: Duration },
    /// The fetch errored.
    Failed { error: SourceError },
}

/// What the active-source policy decides an error warrants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorResponse {
    /// Retry the same source after a backoff.
    Backoff,
    /// Advance to the next source in the chain.
    FailOver,
    /// Stay put and idle — the error is ours, and no other source would answer differently.
    Hold,
}

/// One loop iteration's outcome: how long to sleep, and whether the pipeline is still there.
struct Tick {
    interval: Duration,
    connected: bool,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use crossbeam_channel::{Receiver, unbounded};
    use look_above_core::types::{BBox, Icao24};

    use super::*;
    use crate::budget::DAILY_BUDGET;

    /// 2021-01-01 12:00 UTC — noon of an exact-multiple-of-a-day midnight, matching `budget`'s
    /// own anchor so the ledger arithmetic lines up with that module's tests.
    const NOON: UnixSeconds = UnixSeconds(1_609_459_200 + 43_200);

    #[derive(Debug)]
    struct FixedClock(UnixSeconds);

    impl WallClock for FixedClock {
        fn now(&self) -> UnixSeconds {
            self.0
        }
    }

    /// A test double `LiveSource`: each `fetch` pops the next scripted result; an exhausted
    /// queue repeats `default`. It counts calls so a test can prove a *skipped* cycle sent
    /// nothing.
    #[derive(Debug)]
    struct ScriptedSource {
        id: SourceId,
        cost: u32,
        queue: Mutex<VecDeque<Result<Vec<StateVector>, SourceError>>>,
        default: Result<Vec<StateVector>, SourceError>,
        calls: AtomicUsize,
    }

    impl ScriptedSource {
        fn new(
            id: SourceId,
            cost: u32,
            queued: Vec<Result<Vec<StateVector>, SourceError>>,
            default: Result<Vec<StateVector>, SourceError>,
        ) -> Self {
            Self {
                id,
                cost,
                queue: Mutex::new(queued.into()),
                default,
                calls: AtomicUsize::new(0),
            }
        }

        /// Always succeeds with one aircraft stamped for this source.
        fn always_ok(id: SourceId, cost: u32) -> Self {
            Self::new(id, cost, Vec::new(), Ok(vec![one_state(id)]))
        }

        /// Always fails with `error`.
        fn always_err(id: SourceId, error: SourceError) -> Self {
            Self::new(id, 0, Vec::new(), Err(error))
        }

        fn boxed(self) -> Box<dyn LiveSource> {
            Box::new(self)
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LiveSource for ScriptedSource {
        fn id(&self) -> SourceId {
            self.id
        }

        fn cost(&self, _query: &RegionQuery) -> u32 {
            self.cost
        }

        async fn fetch(&self, _query: &RegionQuery) -> Result<Vec<StateVector>, SourceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut queue = self.queue.lock().expect("scripted queue lock");
            queue.pop_front().unwrap_or_else(|| self.default.clone())
        }
    }

    /// A shared handle to a [`ScriptedSource`]: a local newtype (the orphan rule forbids
    /// implementing the foreign `LiveSource` for a bare `Arc`) so a test can keep an `Arc` to
    /// read `call_count` *and* hand the same source to the poller as a trait object.
    #[derive(Debug)]
    struct SharedSource(Arc<ScriptedSource>);

    #[async_trait]
    impl LiveSource for SharedSource {
        fn id(&self) -> SourceId {
            self.0.id()
        }

        fn cost(&self, query: &RegionQuery) -> u32 {
            self.0.cost(query)
        }

        async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError> {
            self.0.fetch(query).await
        }
    }

    fn one_state(source: SourceId) -> StateVector {
        StateVector {
            icao24: Icao24::from_hex("3c6444").expect("valid ICAO24 in test"),
            callsign: None,
            ts: UnixSeconds(1_721_000_000),
            lat_deg: 47.0,
            lon_deg: 8.0,
            baro_alt_m: None,
            velocity_ms: None,
            heading_deg: None,
            vert_rate_ms: None,
            on_ground: false,
            anonymous: true,
            source,
        }
    }

    fn a_region() -> RegionQuery {
        RegionQuery::region(BBox::new(46.0, 7.0, 48.0, 9.0).expect("valid bbox in test"))
    }

    fn make_poller(sources: Vec<Box<dyn LiveSource>>) -> (Poller, Receiver<PollBatch>) {
        let (tx, rx) = unbounded();
        let poller = Poller::new(sources, a_region(), tx, Arc::new(FixedClock(NOON)));
        (poller, rx)
    }

    // ---- The failover policy, as a pure function -------------------------------------------------

    #[test]
    fn a_transient_error_retries_the_same_source_until_the_threshold_then_fails_over() {
        let network = SourceError::Network {
            message: "timed out".to_owned(),
        };
        for streak in 1..TRANSIENT_FAILOVER_THRESHOLD {
            assert_eq!(
                error_response(&network, streak),
                ErrorResponse::Backoff,
                "streak {streak} is still below the threshold"
            );
        }
        assert_eq!(
            error_response(&network, TRANSIENT_FAILOVER_THRESHOLD),
            ErrorResponse::FailOver,
            "the threshold-th consecutive transient failure fails over"
        );
    }

    #[test]
    fn a_permanent_answer_fails_over_on_the_first_error() {
        // Auth, Parse, and Request are all "the source understood us and will say the same
        // thing again" — no point retrying, so the first one fails over.
        for error in [
            SourceError::Auth {
                message: "no credentials".to_owned(),
            },
            SourceError::Parse {
                message: "not JSON".to_owned(),
            },
            SourceError::Request { status: 404 },
        ] {
            assert_eq!(
                error_response(&error, 1),
                ErrorResponse::FailOver,
                "{error:?} should fail over immediately"
            );
        }
    }

    #[test]
    fn our_own_refusal_holds_and_never_fails_over() {
        let refused = SourceError::Refused {
            reason: "global query to a point source".to_owned(),
        };
        // Even at a high streak, a refusal is never a failover — the next source is no better.
        for streak in [1, TRANSIENT_FAILOVER_THRESHOLD, 99] {
            assert_eq!(error_response(&refused, streak), ErrorResponse::Hold);
        }
    }

    // ---- The recovery-probe gate ----------------------------------------------------------------

    #[test]
    fn the_primary_is_never_probed_while_it_is_the_active_source() {
        assert!(!probe_due(PRIMARY, PRIMARY_PROBE_INTERVAL * 10));
    }

    #[test]
    fn a_fallback_is_probed_only_after_the_interval_elapses() {
        let just_under = PRIMARY_PROBE_INTERVAL
            .checked_sub(Duration::from_secs(1))
            .expect("the probe interval is over a second");
        assert!(!probe_due(1, just_under));
        assert!(probe_due(1, PRIMARY_PROBE_INTERVAL));
        assert!(probe_due(2, PRIMARY_PROBE_INTERVAL * 3));
    }

    // ---- A successful cycle ---------------------------------------------------------------------

    #[tokio::test]
    async fn a_successful_cycle_emits_a_batch_and_stays_on_the_primary() {
        let (mut poller, rx) = make_poller(vec![
            ScriptedSource::always_ok(SourceId::OpenSky, 1).boxed(),
        ]);

        poller.poll_active().await;

        let batch = rx.try_recv().expect("a batch was emitted");
        assert_eq!(batch.source, SourceId::OpenSky);
        assert_eq!(batch.states.len(), 1);
        assert_eq!(batch.credits_spent, 1);
        assert_eq!(batch.spent_today, 1, "the metered cycle was recorded");
        assert_eq!(poller.active_source(), SourceId::OpenSky, "no failover");
    }

    #[tokio::test]
    async fn spend_accumulates_across_metered_cycles() {
        let (mut poller, rx) = make_poller(vec![
            ScriptedSource::always_ok(SourceId::OpenSky, 2).boxed(),
        ]);

        poller.poll_active().await;
        poller.poll_active().await;

        let first = rx.try_recv().expect("first batch");
        let second = rx.try_recv().expect("second batch");
        assert_eq!(first.spent_today, 2);
        assert_eq!(second.spent_today, 4, "two 2-credit cycles have spent four");
    }

    #[tokio::test]
    async fn an_unmetered_source_records_no_spend() {
        let (mut poller, rx) = make_poller(vec![
            ScriptedSource::always_ok(SourceId::AirplanesLive, 0).boxed(),
        ]);

        poller.poll_active().await;

        let batch = rx.try_recv().expect("a batch was emitted");
        assert_eq!(batch.credits_spent, 0);
        assert_eq!(batch.spent_today, 0);
    }

    // ---- The budget veto ------------------------------------------------------------------------

    #[tokio::test]
    async fn an_exhausted_budget_skips_the_cycle_without_fetching_or_failing_over() {
        // An Arc handle lets us read the source's call counter after handing the same source to
        // the poller as a trait object.
        let source = Arc::new(ScriptedSource::always_ok(SourceId::OpenSky, 1));
        let (tx, rx) = unbounded();
        let mut poller = Poller::new(
            vec![Box::new(SharedSource(Arc::clone(&source)))],
            a_region(),
            tx,
            Arc::new(FixedClock(NOON)),
        );

        // Spend the whole day's budget so the next cycle crosses the cap and is refused.
        poller.ledgers[PRIMARY] = CreditLedger::restored(DAILY_BUDGET, NOON);

        let tick = poller.poll_active().await;

        assert_eq!(
            source.call_count(),
            0,
            "a budget-vetoed cycle must not even send the request"
        );
        assert!(rx.try_recv().is_err(), "and it must emit nothing");
        assert_eq!(
            poller.active_source(),
            SourceId::OpenSky,
            "a spent budget is not a failure — the poller stays on the primary"
        );
        assert_eq!(
            tick.interval, MAX_INTERVAL,
            "an exhausted budget idles at the ceiling until the daily reset"
        );
    }

    // ---- Failover -------------------------------------------------------------------------------

    #[tokio::test]
    async fn a_disabled_primary_fails_over_to_the_first_fallback_at_once() {
        let (mut poller, rx) = make_poller(vec![
            ScriptedSource::always_err(
                SourceId::OpenSky,
                SourceError::Auth {
                    message: "no credentials configured".to_owned(),
                },
            )
            .boxed(),
            ScriptedSource::always_ok(SourceId::AirplanesLive, 0).boxed(),
        ]);

        // One poll on the disabled primary: Auth is permanent, so it fails over immediately.
        poller.poll_active().await;
        assert_eq!(poller.active_source(), SourceId::AirplanesLive);
        assert!(rx.try_recv().is_err(), "the failed primary emitted nothing");

        // The next poll lands on the working fallback.
        poller.poll_active().await;
        let batch = rx.try_recv().expect("the fallback emitted a batch");
        assert_eq!(batch.source, SourceId::AirplanesLive);
    }

    #[tokio::test]
    async fn a_transient_primary_fails_over_only_after_repeated_failures() {
        let (mut poller, _rx) = make_poller(vec![
            ScriptedSource::always_err(SourceId::OpenSky, SourceError::Server { status: 503 })
                .boxed(),
            ScriptedSource::always_ok(SourceId::AirplanesLive, 0).boxed(),
        ]);

        // The first threshold-1 failures retry the same source.
        for _ in 1..TRANSIENT_FAILOVER_THRESHOLD {
            poller.poll_active().await;
            assert_eq!(
                poller.active_source(),
                SourceId::OpenSky,
                "still retrying the primary below the threshold"
            );
        }
        // The threshold-th fails over.
        poller.poll_active().await;
        assert_eq!(poller.active_source(), SourceId::AirplanesLive);
    }

    #[tokio::test]
    async fn a_refusal_holds_on_the_same_source_rather_than_failing_over() {
        let (mut poller, _rx) = make_poller(vec![
            ScriptedSource::always_err(
                SourceId::OpenSky,
                SourceError::Refused {
                    reason: "unauthorized host".to_owned(),
                },
            )
            .boxed(),
            ScriptedSource::always_ok(SourceId::AirplanesLive, 0).boxed(),
        ]);

        for _ in 0..5 {
            poller.poll_active().await;
        }
        assert_eq!(
            poller.active_source(),
            SourceId::OpenSky,
            "a refusal is our bug, not a source failure — never a failover"
        );
    }

    #[tokio::test]
    async fn failover_wraps_around_the_chain() {
        // Every source permanently down: the active source rotates and wraps back to primary.
        let (mut poller, _rx) = make_poller(vec![
            ScriptedSource::always_err(SourceId::OpenSky, SourceError::Request { status: 400 })
                .boxed(),
            ScriptedSource::always_err(
                SourceId::AirplanesLive,
                SourceError::Request { status: 400 },
            )
            .boxed(),
            ScriptedSource::always_err(SourceId::AdsbLol, SourceError::Request { status: 400 })
                .boxed(),
        ]);

        assert_eq!(poller.active_source(), SourceId::OpenSky);
        poller.poll_active().await;
        assert_eq!(poller.active_source(), SourceId::AirplanesLive);
        poller.poll_active().await;
        assert_eq!(poller.active_source(), SourceId::AdsbLol);
        poller.poll_active().await;
        assert_eq!(
            poller.active_source(),
            SourceId::OpenSky,
            "wrapped back to primary"
        );
    }

    // ---- Recovery -------------------------------------------------------------------------------

    #[tokio::test]
    async fn a_recovered_primary_is_switched_back_to_and_delivers_its_batch() {
        // The primary errs enough to fail over (a full transient streak), then — its queue
        // exhausted — its default is healthy, so the probe finds it recovered.
        let primary = ScriptedSource::new(
            SourceId::OpenSky,
            1,
            vec![Err(SourceError::Server { status: 503 }); TRANSIENT_FAILOVER_THRESHOLD as usize],
            Ok(vec![one_state(SourceId::OpenSky)]),
        );
        let (mut poller, rx) = make_poller(vec![
            primary.boxed(),
            ScriptedSource::always_ok(SourceId::AirplanesLive, 0).boxed(),
        ]);

        // Push the primary over the transient threshold so it fails over.
        for _ in 0..TRANSIENT_FAILOVER_THRESHOLD {
            poller.poll_active().await;
        }
        assert_eq!(poller.active_source(), SourceId::AirplanesLive);
        // Drain the pre-recovery state (nothing from the failed primary).
        while rx.try_recv().is_ok() {}

        // The probe finds the primary healthy (its queue is now exhausted → default Ok).
        poller.attempt_recovery().await;
        assert_eq!(
            poller.active_source(),
            SourceId::OpenSky,
            "recovered to the primary"
        );
        let batch = rx
            .try_recv()
            .expect("the recovery probe delivered the primary's batch");
        assert_eq!(batch.source, SourceId::OpenSky);
    }

    #[tokio::test]
    async fn a_still_down_primary_leaves_us_on_the_fallback() {
        let (mut poller, _rx) = make_poller(vec![
            ScriptedSource::always_err(SourceId::OpenSky, SourceError::Server { status: 503 })
                .boxed(),
            ScriptedSource::always_ok(SourceId::AirplanesLive, 0).boxed(),
        ]);

        for _ in 0..TRANSIENT_FAILOVER_THRESHOLD {
            poller.poll_active().await;
        }
        assert_eq!(poller.active_source(), SourceId::AirplanesLive);

        poller.attempt_recovery().await;
        assert_eq!(
            poller.active_source(),
            SourceId::AirplanesLive,
            "the probe found the primary still down; stay on the fallback"
        );
    }

    // ---- Shutdown -------------------------------------------------------------------------------

    #[tokio::test]
    async fn a_dropped_receiver_reports_a_disconnected_pipeline() {
        let (tx, rx) = unbounded();
        let mut poller = Poller::new(
            vec![ScriptedSource::always_ok(SourceId::OpenSky, 1).boxed()],
            a_region(),
            tx,
            Arc::new(FixedClock(NOON)),
        );
        drop(rx);

        let tick = poller.poll_active().await;
        assert!(
            !tick.connected,
            "a send to a dropped receiver marks the pipeline gone so run() can stop"
        );
    }

    // ---- Construction ---------------------------------------------------------------------------

    #[test]
    fn the_default_chain_is_the_documented_failover_order() {
        let (tx, _rx) = unbounded();
        let poller = Poller::with_default_chain(
            HttpClient::new().expect("client builds"),
            OpenSkyAuth::disabled(),
            a_region(),
            tx,
            Arc::new(SystemWallClock),
        );
        assert_eq!(poller.sources.len(), 3);
        assert_eq!(poller.sources[0].id(), SourceId::OpenSky);
        assert_eq!(poller.sources[1].id(), SourceId::AirplanesLive);
        assert_eq!(poller.sources[2].id(), SourceId::AdsbLol);
        assert_eq!(poller.active_source(), SourceId::OpenSky);
    }

    // ---- The real chain, end to end -------------------------------------------------------------

    /// The one test that drives real network: `OpenSky` disabled, so the poller fails over to
    /// the keyless fallback and emits a real batch. It exercises the whole seam this item adds
    /// — the failover branch, the unmetered cadence, the ledger, and the channel — against a
    /// live source, keyless and free (0 credits).
    ///
    /// ```text
    /// cargo test -p look-above-ingest -- --ignored live_poller
    /// ```
    ///
    /// Nothing here prints a payload — only counts (docs/06).
    #[tokio::test]
    #[ignore = "hits real airplanes.live via the failover chain; keyless and free"]
    async fn live_poller_fails_over_to_a_keyless_fallback_and_emits_a_batch() {
        let (tx, rx) = unbounded();
        let mut poller = Poller::with_default_chain(
            HttpClient::new().expect("client builds"),
            // Disabled primary → Auth → immediate failover to the first keyless fallback.
            OpenSkyAuth::disabled(),
            a_region(),
            tx,
            Arc::new(SystemWallClock),
        );

        // Cycle 1: OpenSky disabled → fails over to airplanes.live (emits nothing).
        poller.poll_active().await;
        assert_ne!(
            poller.active_source(),
            SourceId::OpenSky,
            "a disabled primary must have failed over"
        );

        // Cycle 2: the keyless fallback actually fetches.
        poller.poll_active().await;

        let batch = rx.try_recv().expect("the fallback emitted a batch");
        assert!(
            matches!(batch.source, SourceId::AirplanesLive | SourceId::AdsbLol),
            "the batch came from a keyless fallback, got {:?}",
            batch.source
        );
        assert_eq!(batch.credits_spent, 0, "the fallbacks are unmetered");
        assert!(
            !batch.states.is_empty(),
            "no aircraft over Switzerland — the sky is empty or the failover path is broken"
        );
        eprintln!(
            "live poller: failed over to {:?}, {} aircraft, 0 credits",
            batch.source,
            batch.states.len()
        );
    }
}
