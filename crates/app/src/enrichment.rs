//! On-selection aircraft/route enrichment (M3 item 3.4): the privacy-rule-2.2 gate, the
//! two-layer cache, and the only call site allowed to invoke `ingest::adsbdb`. See
//! `plans/DECISION_LOG.md`'s "2026-07-21 — M3 item 3.4" entry for the full design rationale
//! this module implements.
//!
//! `ingest::adsbdb::AdsbdbSource` is deliberately a pure HTTP adapter with no caching or
//! storage knowledge of its own — `ingest` and `store` must not depend on each other (the
//! workspace `Cargo.toml`'s own "dependency direction" comment) — so this module, which can see
//! both, is where the two meet. [`Enrichment::on_selection`] is [`crate::window::App`]'s only
//! call into either.
//!
//! Two cache layers, matching the checklist's "LRU + 24h negative cache" line exactly:
//! - Persistent: the `aircraft` table's `fetched_at`/`lookup_failed_at` columns (migration
//!   0001, unused until now) are the 24h negative-cache `store::Writer::aircraft_meta` reads
//!   before a hex is deemed worth fetching. `flights` has no equivalent column (docs/08 never
//!   gave it one), so a route's persistent check is instead "does `latest_flight` already carry
//!   this exact callsign" — a positive-only persistent check, no persistent route negative-cache.
//! - In-memory: a bounded `lru::LruCache` per lookup kind, fronting the persistent layer so a
//!   repeat selection within one process run never round-trips to the store thread again.

use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use look_above_core::contracts::{AircraftCategory, AircraftMeta, Flight};
use look_above_core::error::SourceError;
use look_above_core::sim::AircraftInstance;
use look_above_core::types::{CallSign, Icao24, UnixSeconds};
use look_above_ingest::adsbdb::{AdsbdbSource, should_enrich};
use look_above_store::Writer;
use lru::LruCache;

/// The two adsbdb lookups, behind a trait so tests can stand in a fake with no network at all —
/// `ingest`'s allowlist-widening constructors (`HttpClient::build`, `AdsbdbSource::build`) are
/// deliberately `pub(crate)` (privacy rule 1.1: the only way to a client outside that crate is
/// `HttpClient::new`, which cannot be talked out of the allowlist), so a real, mock-server-backed
/// `AdsbdbSource` cannot be built from here. `AdsbdbSource`'s own HTTP/parsing correctness is
/// `ingest`'s tested responsibility (`crates/ingest/src/adsbdb.rs`); what this module owns and
/// tests is the gate/cache/persistence orchestration around it, independent of the transport.
///
/// `#[async_trait]` for the same dyn-compatibility reason `core::contracts::LiveSource` uses it.
#[async_trait]
trait EnrichmentSource: Send + Sync + std::fmt::Debug {
    async fn fetch_aircraft(
        &self,
        hex: Icao24,
        now: UnixSeconds,
    ) -> Result<Option<AircraftMeta>, SourceError>;

    async fn fetch_route(
        &self,
        callsign: &CallSign,
        icao24: Icao24,
        now: UnixSeconds,
    ) -> Result<Option<Flight>, SourceError>;
}

#[async_trait]
impl EnrichmentSource for AdsbdbSource {
    async fn fetch_aircraft(
        &self,
        hex: Icao24,
        now: UnixSeconds,
    ) -> Result<Option<AircraftMeta>, SourceError> {
        AdsbdbSource::fetch_aircraft(self, hex, now).await
    }

    async fn fetch_route(
        &self,
        callsign: &CallSign,
        icao24: Icao24,
        now: UnixSeconds,
    ) -> Result<Option<Flight>, SourceError> {
        AdsbdbSource::fetch_route(self, callsign, icao24, now).await
    }
}

/// How long a confirmed "not found" is trusted before a fresh lookup is worth retrying
/// (docs/09: adsbdb is "LRU + negative cache (24h)").
const NEGATIVE_CACHE_SECONDS: i64 = 24 * 60 * 60;

/// Bounds the in-memory cache so a long session cannot grow it without limit. The persistent
/// negative-cache (the `aircraft` table's own columns) is what survives an eviction here, not
/// this — an evicted entry just means the next selection re-checks the store before adsbdb.
const CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(256) {
    Some(capacity) => capacity,
    None => unreachable!(),
};

/// A cached lookup's outcome — shared shape for both the aircraft and route caches, since
/// neither ever needs to hand the cached value itself back out (M3 item 3.4 only decides
/// "already resolved, skip the fetch"; 3.5 reads the persisted `AircraftMeta`/`Flight` straight
/// from the store when it builds the info card, not from this cache). A positive hit
/// (`found: true`) is trusted indefinitely; a negative one expires after
/// [`NEGATIVE_CACHE_SECONDS`] — see [`CacheEntry::still_valid`].
#[derive(Debug, Clone, Copy)]
struct CacheEntry {
    checked_at: UnixSeconds,
    found: bool,
}

impl CacheEntry {
    fn still_valid(self, now: UnixSeconds) -> bool {
        self.found || self.checked_at.seconds_until(now) < NEGATIVE_CACHE_SECONDS
    }
}

/// On-selection enrichment: adsbdb lookups gated by privacy rule 2.2, cached, and persisted.
#[derive(Debug)]
pub struct Enrichment {
    source: Box<dyn EnrichmentSource>,
    store: Writer,
    aircraft_cache: Mutex<LruCache<Icao24, CacheEntry>>,
    route_cache: Mutex<LruCache<CallSign, CacheEntry>>,
}

impl Enrichment {
    pub fn new(source: AdsbdbSource, store: Writer) -> Self {
        Self::from_source(Box::new(source), store)
    }

    fn from_source(source: Box<dyn EnrichmentSource>, store: Writer) -> Self {
        Self {
            source,
            store,
            aircraft_cache: Mutex::new(LruCache::new(CACHE_CAPACITY)),
            route_cache: Mutex::new(LruCache::new(CACHE_CAPACITY)),
        }
    }

    /// The selection-path entry point — [`crate::window::App::maybe_select`]'s only call into
    /// this module.
    ///
    /// The privacy-rule-2.2 gate is the first thing this does, not an incidental check buried
    /// later: `instance.anonymous` decides everything, and nothing below this line runs for an
    /// anonymous target. The acceptance criterion (M3 item 3.4: "selecting an anonymous aircraft
    /// fires zero enrichment HTTP requests") depends on this ordering, not just on
    /// [`should_enrich`] existing somewhere.
    pub async fn on_selection(&self, instance: &AircraftInstance, now: UnixSeconds) {
        if !should_enrich(instance.anonymous) {
            return;
        }
        self.lookup_aircraft(instance.icao24, now).await;
        if let Some(callsign) = &instance.callsign {
            self.lookup_route(instance.icao24, callsign, now).await;
        }
    }

    async fn lookup_aircraft(&self, icao24: Icao24, now: UnixSeconds) {
        if self.aircraft_cache_hit(icao24, now) {
            return;
        }
        match self.source.fetch_aircraft(icao24, now).await {
            Ok(Some(meta)) => {
                if let Err(error) = self.store.upsert_aircraft_meta(meta) {
                    tracing::warn!(%error, %icao24, "could not persist aircraft metadata");
                }
                self.cache_aircraft(
                    icao24,
                    CacheEntry {
                        checked_at: now,
                        found: true,
                    },
                );
            }
            Ok(None) => {
                // Persist the negative so a future process (not just this session's LRU) also
                // skips a known-absent hex within the 24h window (docs/08's own
                // `lookup_failed_at` comment: "negative-cache 404s for 24h").
                let negative = AircraftMeta {
                    icao24,
                    registration: None,
                    type_code: None,
                    category: AircraftCategory::Unknown,
                    operator: None,
                    is_anonymous: false,
                    fetched_at: None,
                    lookup_failed_at: Some(now),
                };
                if let Err(error) = self.store.upsert_aircraft_meta(negative) {
                    tracing::warn!(%error, %icao24, "could not persist the negative aircraft lookup");
                }
                self.cache_aircraft(
                    icao24,
                    CacheEntry {
                        checked_at: now,
                        found: false,
                    },
                );
            }
            Err(error) => {
                // Transient (network/parse) failure: never cached, positively or negatively —
                // the next selection of this aircraft is worth a fresh try.
                tracing::warn!(
                    %error, %icao24,
                    "adsbdb aircraft lookup failed; will retry on next selection"
                );
            }
        }
    }

    /// `true` means a cached answer (positive, or a still-fresh negative) already covers
    /// `icao24` — no fetch needed. Checks the in-memory LRU first, then falls back to the
    /// persistent store, populating the LRU from either hit.
    fn aircraft_cache_hit(&self, icao24: Icao24, now: UnixSeconds) -> bool {
        {
            let mut cache = self
                .aircraft_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(entry) = cache.get(&icao24) {
                return entry.still_valid(now);
            }
        }
        match self.store.aircraft_meta(icao24) {
            Ok(Some(meta)) if meta.fetched_at.is_some() => {
                self.cache_aircraft(
                    icao24,
                    CacheEntry {
                        checked_at: now,
                        found: true,
                    },
                );
                true
            }
            Ok(Some(AircraftMeta {
                lookup_failed_at: Some(checked_at),
                ..
            })) if checked_at.seconds_until(now) < NEGATIVE_CACHE_SECONDS => {
                self.cache_aircraft(
                    icao24,
                    CacheEntry {
                        checked_at,
                        found: false,
                    },
                );
                true
            }
            Ok(_) => false,
            Err(error) => {
                tracing::warn!(
                    %error, %icao24,
                    "could not read cached aircraft metadata; fetching fresh"
                );
                false
            }
        }
    }

    fn cache_aircraft(&self, icao24: Icao24, entry: CacheEntry) {
        let mut cache = self
            .aircraft_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cache.put(icao24, entry);
    }

    async fn lookup_route(&self, icao24: Icao24, callsign: &CallSign, now: UnixSeconds) {
        if self.route_cache_hit(icao24, callsign, now) {
            return;
        }
        match self.source.fetch_route(callsign, icao24, now).await {
            Ok(Some(flight)) => {
                if let Err(error) = self.store.insert_flight(flight) {
                    tracing::warn!(
                        %error, %icao24, callsign = %callsign,
                        "could not persist the resolved route"
                    );
                }
                self.cache_route(
                    callsign.clone(),
                    CacheEntry {
                        checked_at: now,
                        found: true,
                    },
                );
            }
            Ok(None) => {
                // No persistent negative-cache for routes — `flights` carries no
                // `lookup_failed_at`-equivalent column (docs/08 never gave it one), so this
                // 24h negative is in-memory only.
                self.cache_route(
                    callsign.clone(),
                    CacheEntry {
                        checked_at: now,
                        found: false,
                    },
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error, callsign = %callsign,
                    "adsbdb route lookup failed; will retry on next selection"
                );
            }
        }
    }

    /// `true` means a cached route already covers `callsign` — no fetch needed. Checks the
    /// in-memory LRU first, then whether the store's most recent `flights` row for `icao24`
    /// already carries this exact callsign (a positive-only persistent check; see this module's
    /// own doc comment for why there is no persistent route negative-cache).
    fn route_cache_hit(&self, icao24: Icao24, callsign: &CallSign, now: UnixSeconds) -> bool {
        {
            let mut cache = self
                .route_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(entry) = cache.get(callsign) {
                return entry.still_valid(now);
            }
        }
        match self.store.latest_flight(icao24) {
            Ok(Some(flight)) if flight.callsign.as_ref() == Some(callsign) => {
                self.cache_route(
                    callsign.clone(),
                    CacheEntry {
                        checked_at: now,
                        found: true,
                    },
                );
                true
            }
            Ok(_) => false,
            Err(error) => {
                tracing::warn!(%error, %icao24, "could not read the cached route; fetching fresh");
                false
            }
        }
    }

    fn cache_route(&self, callsign: CallSign, entry: CacheEntry) {
        let mut cache = self
            .route_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cache.put(callsign, entry);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use look_above_core::geo::MercatorXy;
    use look_above_core::sim::AltitudeBucket;
    use look_above_core::types::SourceId;

    use super::*;

    /// A no-network stand-in for [`AdsbdbSource`]: counts calls and returns a canned answer, so
    /// tests exercise this module's own gate/cache/persistence logic without depending on
    /// `ingest`'s HTTP shapes (already covered by `crates/ingest/src/adsbdb.rs`'s own tests).
    /// Wrapped in `Arc` so a test can keep its own handle to inspect call counts after handing a
    /// clone into `Enrichment` (which only sees it as `Box<dyn EnrichmentSource>`).
    #[derive(Debug, Default)]
    struct FakeSource {
        aircraft_calls: AtomicUsize,
        route_calls: AtomicUsize,
        aircraft_response: Mutex<Option<AircraftMeta>>,
        route_response: Mutex<Option<Flight>>,
    }

    impl FakeSource {
        fn returning_aircraft(meta: Option<AircraftMeta>) -> Arc<Self> {
            Arc::new(Self {
                aircraft_response: Mutex::new(meta),
                ..Self::default()
            })
        }

        fn returning_both(aircraft: Option<AircraftMeta>, route: Option<Flight>) -> Arc<Self> {
            Arc::new(Self {
                aircraft_response: Mutex::new(aircraft),
                route_response: Mutex::new(route),
                ..Self::default()
            })
        }

        fn aircraft_calls(&self) -> usize {
            self.aircraft_calls.load(Ordering::SeqCst)
        }

        fn route_calls(&self) -> usize {
            self.route_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EnrichmentSource for FakeSource {
        async fn fetch_aircraft(
            &self,
            _hex: Icao24,
            _now: UnixSeconds,
        ) -> Result<Option<AircraftMeta>, SourceError> {
            self.aircraft_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .aircraft_response
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone())
        }

        async fn fetch_route(
            &self,
            _callsign: &CallSign,
            _icao24: Icao24,
            _now: UnixSeconds,
        ) -> Result<Option<Flight>, SourceError> {
            self.route_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .route_response
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone())
        }
    }

    /// Delegates through the shared `Arc`, so `Enrichment` and the test's own handle see the
    /// same call counters.
    #[async_trait]
    impl EnrichmentSource for Arc<FakeSource> {
        async fn fetch_aircraft(
            &self,
            hex: Icao24,
            now: UnixSeconds,
        ) -> Result<Option<AircraftMeta>, SourceError> {
            self.as_ref().fetch_aircraft(hex, now).await
        }

        async fn fetch_route(
            &self,
            callsign: &CallSign,
            icao24: Icao24,
            now: UnixSeconds,
        ) -> Result<Option<Flight>, SourceError> {
            self.as_ref().fetch_route(callsign, icao24, now).await
        }
    }

    fn hex(text: &str) -> Icao24 {
        Icao24::from_hex(text).expect("valid ICAO24 in test")
    }

    fn callsign(text: &str) -> CallSign {
        CallSign::new(text).expect("valid callsign in test")
    }

    fn found_aircraft(icao24: Icao24, now: UnixSeconds) -> AircraftMeta {
        AircraftMeta {
            icao24,
            registration: Some("N401TT".to_owned()),
            type_code: Some("SR22".to_owned()),
            category: AircraftCategory::Unknown,
            operator: Some("Some Owner".to_owned()),
            is_anonymous: false,
            fetched_at: Some(now),
            lookup_failed_at: None,
        }
    }

    fn found_route(icao24: Icao24, cs: &CallSign, now: UnixSeconds) -> Flight {
        Flight {
            icao24,
            callsign: Some(cs.clone()),
            origin: Some("PANC".to_owned()),
            destination: Some("KORD".to_owned()),
            first_seen: now,
            last_seen: now,
        }
    }

    /// A minimal, otherwise-empty instance — every test overrides `anonymous`/`callsign`.
    fn instance(icao24: Icao24, anonymous: bool, callsign: Option<CallSign>) -> AircraftInstance {
        AircraftInstance {
            icao24,
            position: MercatorXy::new(0.0, 0.0),
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::Ground,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous,
            callsign,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: false,
            source: SourceId::OpenSky,
        }
    }

    fn enrichment_with(spy: &Arc<FakeSource>) -> Enrichment {
        let store = Writer::open(":memory:").expect("in-memory store opens");
        Enrichment::from_source(Box::new(Arc::clone(spy)), store)
    }

    // ---- The privacy-rule-2.2 gate: the acceptance criterion itself -------------------------

    #[tokio::test]
    async fn selecting_an_anonymous_aircraft_fires_zero_enrichment_requests() {
        let target = instance(hex("a4b213"), true, Some(callsign("UAL123")));
        let spy = FakeSource::returning_both(None, None);
        let enrichment = enrichment_with(&spy);

        enrichment.on_selection(&target, UnixSeconds(0)).await;

        assert_eq!(spy.aircraft_calls(), 0, "anonymous: zero aircraft lookups");
        assert_eq!(spy.route_calls(), 0, "anonymous: zero route lookups");
    }

    // ---- The happy path: fetch, persist, cache ------------------------------------------------

    #[tokio::test]
    async fn a_non_anonymous_selection_with_no_callsign_only_looks_up_the_aircraft() {
        let target_hex = hex("a4b213");
        let target = instance(target_hex, false, None);
        let now = UnixSeconds(1_000);
        let spy = FakeSource::returning_aircraft(Some(found_aircraft(target_hex, now)));
        let enrichment = enrichment_with(&spy);

        enrichment.on_selection(&target, now).await;

        assert_eq!(spy.aircraft_calls(), 1);
        assert_eq!(
            spy.route_calls(),
            0,
            "no callsign means no route lookup at all"
        );
        let stored = enrichment
            .store
            .aircraft_meta(target_hex)
            .expect("reads")
            .expect("upserted");
        assert_eq!(stored.type_code.as_deref(), Some("SR22"));
    }

    #[tokio::test]
    async fn a_non_anonymous_selection_with_a_callsign_looks_up_both_and_persists_both() {
        let target_hex = hex("a4b213");
        let cs = callsign("UAL123");
        let target = instance(target_hex, false, Some(cs.clone()));
        let now = UnixSeconds(1_000);
        let spy = FakeSource::returning_both(
            Some(found_aircraft(target_hex, now)),
            Some(found_route(target_hex, &cs, now)),
        );
        let enrichment = enrichment_with(&spy);

        enrichment.on_selection(&target, now).await;

        assert_eq!(spy.aircraft_calls(), 1);
        assert_eq!(spy.route_calls(), 1);
        assert!(
            enrichment
                .store
                .aircraft_meta(target_hex)
                .expect("reads")
                .is_some()
        );
        let flight = enrichment
            .store
            .latest_flight(target_hex)
            .expect("reads")
            .expect("route was inserted");
        assert_eq!(flight.origin.as_deref(), Some("PANC"));
        assert_eq!(flight.destination.as_deref(), Some("KORD"));
    }

    // ---- The two-layer cache: repeats never re-hit the network -------------------------------

    #[tokio::test]
    async fn reselecting_the_same_aircraft_within_a_session_does_not_refetch() {
        let target_hex = hex("a4b213");
        let target = instance(target_hex, false, None);
        let now = UnixSeconds(1_000);
        let spy = FakeSource::returning_aircraft(Some(found_aircraft(target_hex, now)));
        let enrichment = enrichment_with(&spy);

        enrichment.on_selection(&target, now).await;
        enrichment
            .on_selection(&target, UnixSeconds(now.0 + 100))
            .await;

        assert_eq!(
            spy.aircraft_calls(),
            1,
            "the second selection must be served from the LRU, not the wire"
        );
    }

    #[tokio::test]
    async fn a_negative_lookup_is_cached_for_the_documented_window() {
        let target_hex = hex("a4b213");
        let target = instance(target_hex, false, None);
        let first = UnixSeconds(1_000);
        let spy = FakeSource::returning_aircraft(None);
        let enrichment = enrichment_with(&spy);

        enrichment.on_selection(&target, first).await;
        // A reselect one hour later is still within the 24h negative window.
        enrichment
            .on_selection(&target, UnixSeconds(first.0 + 3_600))
            .await;

        assert_eq!(
            spy.aircraft_calls(),
            1,
            "a fresh negative must not be re-fetched within 24h"
        );
        let stored = enrichment
            .store
            .aircraft_meta(target_hex)
            .expect("reads")
            .expect("the negative was persisted");
        assert_eq!(stored.fetched_at, None);
        assert_eq!(stored.lookup_failed_at, Some(first));
    }

    #[tokio::test]
    async fn a_negative_lookup_past_the_window_is_retried() {
        let target_hex = hex("a4b213");
        let target = instance(target_hex, false, None);
        let first = UnixSeconds(1_000);
        let spy = FakeSource::returning_aircraft(None);
        let enrichment = enrichment_with(&spy);

        enrichment.on_selection(&target, first).await;
        enrichment
            .on_selection(&target, UnixSeconds(first.0 + NEGATIVE_CACHE_SECONDS + 1))
            .await;

        assert_eq!(
            spy.aircraft_calls(),
            2,
            "a negative past the 24h window must be retried"
        );
    }

    // ---- The persistent layer alone (no LRU entry yet) is honored -----------------------------

    #[tokio::test]
    async fn a_fresh_process_with_a_pre_populated_store_row_does_not_fetch() {
        let target_hex = hex("a4b213");
        let target = instance(target_hex, false, None);
        let spy = FakeSource::returning_both(None, None);

        let store = Writer::open(":memory:").expect("in-memory store opens");
        store
            .upsert_aircraft_meta(found_aircraft(target_hex, UnixSeconds(500)))
            .expect("seeds the store directly, as a prior process run would have");
        let enrichment = Enrichment::from_source(Box::new(Arc::clone(&spy)), store);

        enrichment.on_selection(&target, UnixSeconds(1_000)).await;

        assert_eq!(
            spy.aircraft_calls(),
            0,
            "a store hit alone (no LRU entry existed yet) must skip the network"
        );
    }

    #[tokio::test]
    async fn a_persisted_route_for_the_current_callsign_is_reused_without_a_fetch() {
        let target_hex = hex("a4b213");
        let cs = callsign("UAL123");
        let target = instance(target_hex, false, Some(cs.clone()));
        // The aircraft side still has to fetch (nothing persisted for it), but the route side
        // should not: a matching `flights` row for this exact callsign already exists.
        let spy =
            FakeSource::returning_aircraft(Some(found_aircraft(target_hex, UnixSeconds(2_000))));

        let store = Writer::open(":memory:").expect("in-memory store opens");
        store
            .insert_flight(found_route(target_hex, &cs, UnixSeconds(500)))
            .expect("seeds the store directly, as a prior process run would have");
        let enrichment = Enrichment::from_source(Box::new(Arc::clone(&spy)), store);

        enrichment.on_selection(&target, UnixSeconds(2_000)).await;

        assert_eq!(spy.aircraft_calls(), 1);
        assert_eq!(
            spy.route_calls(),
            0,
            "a persisted route for the same callsign must be reused, not re-fetched"
        );
    }
}
