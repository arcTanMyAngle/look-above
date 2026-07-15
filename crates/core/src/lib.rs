//! Core types, geo math, and simulation contracts for Look Above.
//!
//! This crate must stay free of I/O dependencies: no network, no DB, no GPU.
//! It owns the vocabulary ([`StateVector`], [`Icao24`], …) and the seams
//! ([`LiveSource`], [`Store`]) that `ingest`, `store`, and `render` implement
//! against — see `docs/09_API_CONTRACTS.md`.
//!
//! M0 item 0.3 defines those shapes; `core::geo` lands in 0.4 and the
//! interpolation pipeline (`core::sim`, `core::merge`) in M2.

pub mod contracts;
pub mod error;
pub mod types;

pub use contracts::{
    AircraftCategory, AircraftMeta, Airport, AirportSize, LiveSource, RegionQuery, Store,
};
pub use error::{SourceError, StoreError};
pub use types::{
    BBox, BBoxError, CallSign, Icao24, Icao24ParseError, SourceId, StateVector, UnixSeconds,
    UnknownSourceId,
};
