//! Live data ingestion for Look Above: `LiveSource` adapters, pollers, and
//! rate/credit budgeting.
//!
//! Every host this crate may contact is fixed by the authorized-aviation-sources skill:
//! [`allowlist`] is that list in code, and [`http`] — the client every adapter must use —
//! enforces it while carrying the cross-cutting rules (User-Agent, timeout, backoff, error
//! mapping) docs/09 requires of all of them.
//!
//! [`opensky`] is the budgeted primary; the keyless fallbacks ([`airplanes_live`], and
//! adsb.lol at item 1.6) share one readsb parser ([`readsb`]) and space their requests
//! with a [`pacer`].

pub mod airplanes_live;
pub mod allowlist;
pub mod http;
mod normalize;
pub mod opensky;
pub mod pacer;
pub mod readsb;
