//! The session's merged live picture — one deduplicated [`StateVector`] per aircraft, fed by
//! every source through the failover chain.
//!
//! A [`PollBatch`](../../look_above_ingest/poller/struct.PollBatch.html) arrives from *one*
//! source per cycle, but across cycles the same aircraft is seen by different sources (the
//! failover chain switches between them) and by the same source repeatedly. This module is the
//! single place those observations are reconciled into "what is where, now":
//!
//! - **Dedup, newest-`ts`-wins.** Keyed by [`Icao24`]. A record whose source time of
//!   applicability (`ts`) is strictly newer than the one held replaces it; anything not newer
//!   is dropped. Because sources disagree on how fresh their view is, this is what stops a
//!   slower feed from dragging an aircraft backwards to an older fix (M2's dead reckoning would
//!   then advance it from a place it had already left — the same reasoning as item 1.4's
//!   `time_position` choice).
//!
//! - **Out-of-order drop.** The same rule handles a late-arriving stale update: `ts` not newer
//!   ⇒ not accepted. Equal-`ts` duplicates (two sources reporting the identical fix) are drops
//!   too — there is no newer information in them.
//!
//! - **Sticky anonymity (privacy rule 2.2).** Anonymity is a **one-way latch**: once any record
//!   for a hex is marked [`anonymous`](StateVector::anonymous), the tracked target stays
//!   anonymous for the rest of the session, and its [`callsign`](StateVector::callsign) is held
//!   at `None` — *even if a later, newer record carries an identity*. The latch is honored
//!   regardless of `ts` ordering: an anonymity signal is a privacy fact, not a position, so
//!   even a record too stale to update the position still latches it. This is the code side of
//!   the rule that says we never un-anonymize a target we once showed as unidentified.
//!
//! - **Staleness tracking.** Each entry carries its `ts`, so its age against a wall clock is
//!   [`age`](SessionTable::age). The table reports how many tracks have gone stale
//!   ([`stale_count`](SessionTable::stale_count)) and forgets the ones past a horizon
//!   ([`evict_stale`](SessionTable::evict_stale)) so the picture does not accumulate ghosts.
//!   The *visual* fade of a stale track (alpha ramp, frozen extrapolation) is the render
//!   layer's job in M2; this module only decides when a track stops being fresh
//!   ([`STALE_AFTER_S`]) and when it is dropped from tracking altogether ([`DROP_AFTER_S`]).
//!
//! The merge itself is a pure function of the records and the held state — no clock, no I/O —
//! so the dedup and stickiness rules are unit-testable in isolation (docs/10 §1). Only the
//! staleness queries take a `now`, since age is the one thing that depends on the wall clock.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::types::{Icao24, StateVector, UnixSeconds};

/// Age, in seconds, beyond which a track is reported *stale*. It matches the render layer's
/// "begin fade" point (high-fidelity-flight-visualization skill): the moment a fix stops being
/// fresh enough to trust without a caveat. Reporting only — a stale track is still tracked.
pub const STALE_AFTER_S: i64 = 60;

/// Age, in seconds, beyond which a stale track is dropped from the table entirely. Past this
/// the render layer has stopped extrapolating the position (skill: "stop extrapolating at
/// 90 s"), so holding the entry would only serve a frozen ghost; [`evict_stale`] forgets it.
pub const DROP_AFTER_S: i64 = 90;

/// How one record landed against the table — the tally [`SessionTable::merge`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MergeStats {
    /// Aircraft seen for the first time this session.
    pub new: usize,
    /// Existing aircraft advanced to a newer fix.
    pub updated: usize,
    /// Records not newer than the held fix (out-of-order or duplicate), so dropped. A dropped
    /// record may still have latched anonymity (privacy rule 2.2) even though its position was
    /// not taken.
    pub dropped: usize,
}

impl MergeStats {
    /// Every record accounted for — `new + updated + dropped` equals the batch length.
    #[must_use]
    pub const fn total(self) -> usize {
        self.new + self.updated + self.dropped
    }
}

/// The merged, deduplicated live picture: at most one [`StateVector`] per [`Icao24`], the
/// freshest the session has seen, with anonymity latched.
///
/// This is the session-scoped store the plan calls the pipeline's source of truth. It is
/// deliberately clock-free for merging (so dedup and stickiness test in isolation) and takes a
/// `now` only for the staleness queries.
#[derive(Debug, Clone, Default)]
pub struct SessionTable {
    entries: HashMap<Icao24, StateVector>,
}

impl SessionTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Merges one source's batch into the table, returning how the records landed.
    ///
    /// Records are absorbed in order, so two records for the same hex within a single batch
    /// reconcile against each other just as they would across batches.
    pub fn merge(&mut self, batch: &[StateVector]) -> MergeStats {
        let mut stats = MergeStats::default();
        for state in batch {
            match self.absorb(state) {
                Landing::New => stats.new += 1,
                Landing::Updated => stats.updated += 1,
                Landing::Dropped => stats.dropped += 1,
            }
        }
        stats
    }

    /// Reconciles a single record against the held state under the dedup + latch rules.
    fn absorb(&mut self, incoming: &StateVector) -> Landing {
        match self.entries.entry(incoming.icao24) {
            Entry::Vacant(slot) => {
                let mut first = incoming.clone();
                // A first sighting that is anonymous must never carry a callsign, even if the
                // adapter left one on — the invariant is enforced here, not trusted upstream.
                if first.anonymous {
                    first.callsign = None;
                }
                slot.insert(first);
                Landing::New
            }
            Entry::Occupied(mut slot) => {
                let stored = slot.get_mut();
                // The anonymity latch is one-way and independent of `ts`: any record marking
                // the hex anonymous keeps it anonymous, even one we are about to drop as stale.
                let anonymous = stored.anonymous || incoming.anonymous;
                if incoming.ts > stored.ts {
                    let mut next = incoming.clone();
                    next.anonymous = anonymous;
                    if anonymous {
                        // Sticky: a newer, *identified* record does not un-anonymize the target.
                        next.callsign = None;
                    }
                    *stored = next;
                    Landing::Updated
                } else {
                    // Not newer: keep the fresher held position, but still honor the latch if
                    // this stale record is the one revealing the target is anonymous.
                    if anonymous && !stored.anonymous {
                        stored.anonymous = true;
                        stored.callsign = None;
                    }
                    Landing::Dropped
                }
            }
        }
    }

    /// The held state for `icao24`, if the session has seen it.
    #[must_use]
    pub fn get(&self, icao24: Icao24) -> Option<&StateVector> {
        self.entries.get(&icao24)
    }

    /// Every tracked aircraft, in arbitrary order.
    pub fn states(&self) -> impl Iterator<Item = &StateVector> {
        self.entries.values()
    }

    /// How many aircraft are tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table holds no aircraft.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The age of `icao24`'s held fix at `now`, in seconds (`now − ts`); `None` if untracked.
    ///
    /// Negative when a fix's `ts` is in the future relative to `now` (clock skew between a
    /// source and this machine); callers treating it as an unsigned age should clamp at zero.
    #[must_use]
    pub fn age(&self, icao24: Icao24, now: UnixSeconds) -> Option<i64> {
        self.entries.get(&icao24).map(|s| s.ts.seconds_until(now))
    }

    /// How many tracked aircraft are older than `max_age_s` seconds at `now`.
    ///
    /// The headless readout (item 1.12) passes [`STALE_AFTER_S`] for its "stale" count.
    #[must_use]
    pub fn stale_count(&self, now: UnixSeconds, max_age_s: i64) -> usize {
        self.entries
            .values()
            .filter(|s| s.ts.seconds_until(now) > max_age_s)
            .count()
    }

    /// Forgets every tracked aircraft older than `max_age_s` seconds at `now`, returning how
    /// many were dropped.
    ///
    /// Callers pass [`DROP_AFTER_S`] for the standard horizon. Eviction only affects *stale*
    /// entries; a re-acquired aircraft is simply merged back in as a new sighting.
    pub fn evict_stale(&mut self, now: UnixSeconds, max_age_s: i64) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, s| s.ts.seconds_until(now) <= max_age_s);
        before - self.entries.len()
    }
}

/// How a single record landed — the private per-record result [`SessionTable::merge`] tallies.
enum Landing {
    New,
    Updated,
    Dropped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CallSign, SourceId};

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    /// A minimal state for `icao` from `source` at `ts`, at a fixed benign position. Optional
    /// facets (callsign, anonymity) are set by the `with_*` helpers below.
    fn state(icao: &str, source: SourceId, ts: i64) -> StateVector {
        StateVector {
            icao24: hex(icao),
            callsign: None,
            ts: UnixSeconds(ts),
            lat_deg: 47.0,
            lon_deg: 8.0,
            baro_alt_m: Some(10_000.0),
            velocity_ms: Some(230.0),
            heading_deg: Some(90.0),
            vert_rate_ms: None,
            on_ground: false,
            anonymous: false,
            source,
        }
    }

    fn with_callsign(mut s: StateVector, cs: &str) -> StateVector {
        s.callsign = CallSign::new(cs);
        s
    }

    fn anonymous(mut s: StateVector) -> StateVector {
        s.anonymous = true;
        s
    }

    // ---- Dedup: newest ts wins ------------------------------------------------------------------

    #[test]
    fn a_first_sighting_is_counted_new_and_stored() {
        let mut table = SessionTable::new();
        let stats = table.merge(&[state("3c6444", SourceId::OpenSky, 100)]);

        assert_eq!(
            stats,
            MergeStats {
                new: 1,
                updated: 0,
                dropped: 0
            }
        );
        assert_eq!(table.len(), 1);
        assert_eq!(
            table.get(hex("3c6444")).map(|s| s.ts),
            Some(UnixSeconds(100))
        );
    }

    #[test]
    fn a_newer_record_from_another_source_wins() {
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", SourceId::OpenSky, 100)]);

        let stats = table.merge(&[state("3c6444", SourceId::AirplanesLive, 130)]);

        assert_eq!(
            stats,
            MergeStats {
                new: 0,
                updated: 1,
                dropped: 0
            }
        );
        let held = table.get(hex("3c6444")).expect("still tracked");
        assert_eq!(
            held.ts,
            UnixSeconds(130),
            "the newer fix replaced the older"
        );
        assert_eq!(held.source, SourceId::AirplanesLive);
        assert_eq!(table.len(), 1, "same aircraft, not a second entry");
    }

    #[test]
    fn an_out_of_order_record_is_dropped() {
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", SourceId::OpenSky, 130)]);

        let stats = table.merge(&[state("3c6444", SourceId::AdsbLol, 100)]);

        assert_eq!(
            stats,
            MergeStats {
                new: 0,
                updated: 0,
                dropped: 1
            }
        );
        let held = table.get(hex("3c6444")).expect("still tracked");
        assert_eq!(held.ts, UnixSeconds(130), "the fresher fix was kept");
        assert_eq!(held.source, SourceId::OpenSky);
    }

    #[test]
    fn an_equal_ts_duplicate_is_dropped() {
        // Two sources reporting the identical time of applicability: no newer information, so
        // the second is a drop, not an update.
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", SourceId::OpenSky, 100)]);

        let stats = table.merge(&[state("3c6444", SourceId::AirplanesLive, 100)]);

        assert_eq!(
            stats,
            MergeStats {
                new: 0,
                updated: 0,
                dropped: 1
            }
        );
        assert_eq!(
            table.get(hex("3c6444")).map(|s| s.source),
            Some(SourceId::OpenSky),
            "the first-seen fix is retained on a tie"
        );
    }

    #[test]
    fn distinct_aircraft_are_tracked_separately() {
        let mut table = SessionTable::new();
        let stats = table.merge(&[
            state("3c6444", SourceId::OpenSky, 100),
            state("4b1815", SourceId::OpenSky, 100),
        ]);

        assert_eq!(
            stats,
            MergeStats {
                new: 2,
                updated: 0,
                dropped: 0
            }
        );
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn repeated_records_within_one_batch_reconcile_in_order() {
        // A single batch carrying the same hex twice: the first is new, the second (newer) an
        // update — the same rule as across batches.
        let mut table = SessionTable::new();
        let stats = table.merge(&[
            state("3c6444", SourceId::OpenSky, 100),
            state("3c6444", SourceId::OpenSky, 140),
        ]);

        assert_eq!(
            stats,
            MergeStats {
                new: 1,
                updated: 1,
                dropped: 0
            }
        );
        assert_eq!(
            table.get(hex("3c6444")).map(|s| s.ts),
            Some(UnixSeconds(140))
        );
    }

    // ---- Sticky anonymity (privacy rule 2.2) ----------------------------------------------------

    #[test]
    fn a_first_anonymous_sighting_never_carries_a_callsign() {
        // Even if an adapter mistakenly left a callsign on an anonymous record, merge strips it.
        let mut table = SessionTable::new();
        let rogue = anonymous(with_callsign(
            state("3c6444", SourceId::OpenSky, 100),
            "DAL123",
        ));
        table.merge(&[rogue]);

        let held = table.get(hex("3c6444")).expect("tracked");
        assert!(held.anonymous);
        assert_eq!(held.callsign, None, "an anonymous target shows no identity");
    }

    #[test]
    fn a_later_identified_record_does_not_un_anonymize() {
        // The core of rule 2.2: once anonymous, a newer record that *does* carry an identity
        // must not reveal it.
        let mut table = SessionTable::new();
        table.merge(&[anonymous(state("3c6444", SourceId::OpenSky, 100))]);

        let identified = with_callsign(state("3c6444", SourceId::AirplanesLive, 200), "DAL123");
        table.merge(&[identified]);

        let held = table.get(hex("3c6444")).expect("tracked");
        assert!(held.anonymous, "anonymity is sticky for the session");
        assert_eq!(held.callsign, None, "the newer identity stays hidden");
        assert_eq!(held.ts, UnixSeconds(200), "the position still advanced");
    }

    #[test]
    fn an_out_of_order_anonymous_record_still_latches_anonymity() {
        // A stale record too old to update the position is still an anonymity signal, and the
        // latch is honored regardless of ts ordering.
        let mut table = SessionTable::new();
        table.merge(&[with_callsign(
            state("3c6444", SourceId::OpenSky, 200),
            "DAL123",
        )]);

        let stale_anon = anonymous(state("3c6444", SourceId::AdsbLol, 100));
        let stats = table.merge(&[stale_anon]);

        assert_eq!(stats.dropped, 1, "the stale position was dropped");
        let held = table.get(hex("3c6444")).expect("tracked");
        assert!(held.anonymous, "but its anonymity latched");
        assert_eq!(held.callsign, None, "and the earlier callsign was cleared");
        assert_eq!(held.ts, UnixSeconds(200), "the fresher position was kept");
    }

    #[test]
    fn a_non_anonymous_update_leaves_an_identified_target_identified() {
        // The latch is one-way *toward* anonymity only — an ordinary target is not touched.
        let mut table = SessionTable::new();
        table.merge(&[with_callsign(
            state("3c6444", SourceId::OpenSky, 100),
            "DAL123",
        )]);
        table.merge(&[with_callsign(
            state("3c6444", SourceId::OpenSky, 200),
            "DAL123",
        )]);

        let held = table.get(hex("3c6444")).expect("tracked");
        assert!(!held.anonymous);
        assert_eq!(held.callsign.as_ref().map(CallSign::as_str), Some("DAL123"));
    }

    // ---- Staleness tracking ---------------------------------------------------------------------

    #[test]
    fn age_is_now_minus_ts_and_none_when_untracked() {
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", SourceId::OpenSky, 100)]);

        assert_eq!(table.age(hex("3c6444"), UnixSeconds(175)), Some(75));
        assert_eq!(table.age(hex("000000"), UnixSeconds(175)), None);
    }

    #[test]
    fn stale_count_uses_the_age_horizon() {
        let mut table = SessionTable::new();
        table.merge(&[
            state("3c6444", SourceId::OpenSky, 100), // age 30 at now=130 — fresh
            state("4b1815", SourceId::OpenSky, 40),  // age 90 — stale past STALE_AFTER_S
        ]);

        let now = UnixSeconds(130);
        assert_eq!(table.stale_count(now, STALE_AFTER_S), 1);
        assert_eq!(
            table.stale_count(now, 200),
            0,
            "a looser horizon finds none stale"
        );
    }

    #[test]
    fn evict_stale_forgets_only_tracks_past_the_horizon() {
        let mut table = SessionTable::new();
        table.merge(&[
            state("3c6444", SourceId::OpenSky, 100), // age 20 at now=120 — kept
            state("4b1815", SourceId::OpenSky, 10),  // age 110 — evicted at DROP_AFTER_S
        ]);

        let dropped = table.evict_stale(UnixSeconds(120), DROP_AFTER_S);

        assert_eq!(dropped, 1);
        assert_eq!(table.len(), 1);
        assert!(
            table.get(hex("3c6444")).is_some(),
            "the fresh track survived"
        );
        assert!(
            table.get(hex("4b1815")).is_none(),
            "the stale track was forgotten"
        );
    }

    #[test]
    fn the_stale_horizon_is_at_or_below_the_drop_horizon() {
        // A track is reported stale before it is forgotten, never the reverse.
        const { assert!(STALE_AFTER_S <= DROP_AFTER_S) };
    }

    #[test]
    fn merge_stats_total_accounts_for_every_record() {
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", SourceId::OpenSky, 100)]);

        let stats = table.merge(&[
            state("3c6444", SourceId::OpenSky, 90),  // dropped (older)
            state("3c6444", SourceId::OpenSky, 150), // updated
            state("4b1815", SourceId::OpenSky, 100), // new
        ]);

        assert_eq!(
            stats,
            MergeStats {
                new: 1,
                updated: 1,
                dropped: 1
            }
        );
        assert_eq!(stats.total(), 3);
    }
}
