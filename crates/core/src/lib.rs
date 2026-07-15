//! Core types, geo math, and simulation contracts for Look Above.
//!
//! This crate must stay free of I/O dependencies: no network, no DB, no GPU.
//! It owns the vocabulary ([`StateVector`], [`Icao24`], …) and the seams
//! ([`LiveSource`], [`Store`]) that `ingest`, `store`, and `render` implement
//! against — see `docs/09_API_CONTRACTS.md`.
//!
//! M0 items 0.3–0.4 define those shapes and the geo math; the interpolation
//! pipeline (`core::sim`, `core::merge`) lands in M2.

pub mod contracts;
pub mod error;
pub mod geo;
pub mod secret;
pub mod types;

pub use contracts::{
    AircraftCategory, AircraftMeta, Airport, AirportSize, LiveSource, RegionQuery, Store,
};
pub use error::{SourceError, StoreError};
pub use secret::SecretString;
pub use types::{
    BBox, BBoxError, CallSign, Icao24, Icao24ParseError, SourceId, StateVector, UnixSeconds,
    UnknownSourceId,
};
