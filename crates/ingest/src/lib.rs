//! Live data ingestion for Look Above: `LiveSource` adapters, pollers, and
//! rate/credit budgeting.
//!
//! Every host this crate may contact is fixed by the authorized-aviation-sources skill:
//! [`allowlist`] is that list in code, and [`http`] — the client every adapter must use —
//! enforces it while carrying the cross-cutting rules (User-Agent, timeout, backoff, error
//! mapping) docs/09 requires of all of them.
//!
//! [`opensky`] is the budgeted primary; the keyless fallbacks ([`airplanes_live`] and
//! [`adsb_lol`]) share one readsb parser ([`readsb`]) and one point-query implementation
//! ([`point`]) — bbox → covering circle, the 250 nm clamp, [`pacer`] spacing, and trimming
//! the reply back to the bbox — differing only in host, id, spacing, and fixtures.
//!
//! [`budget`] is what keeps the primary inside `OpenSky`'s allowance: a daily credit ledger
//! and the pure cadence controller (`ledger + cost + clock → poll interval`).
//!
//! [`poller`] ties it together: the loop that drives the active source at that cadence, fails
//! over through the chain on error, probes for the primary's recovery, and emits
//! [`PollBatch`](poller::PollBatch)es into the `crossbeam` channel the rest of the pipeline
//! reads.
//!
//! [`metar`] is the separate, single-source enrichment poller for `aviationweather.gov`
//! (M3 item 3.3) — no failover chain, no credit budget, just a fixed ≥10-minute cadence over
//! whatever station list the camera's viewport currently asks for.

pub mod adsb_lol;
pub mod airplanes_live;
pub mod allowlist;
pub mod budget;
pub mod http;
pub mod metar;
mod normalize;
pub mod opensky;
pub mod pacer;
pub mod point;
pub mod poller;
pub mod readsb;
