//! Core types, geo math, and simulation contracts for Look Above.
//!
//! This crate must stay free of I/O dependencies: no network, no DB, no GPU.
//! It owns the vocabulary ([`StateVector`], [`Icao24`], …) and the seams
//! ([`LiveSource`], [`Store`]) that `ingest`, `store`, and `render` implement
//! against — see `docs/09_API_CONTRACTS.md`.
//!
//! M0 items 0.3–0.4 define those shapes and the geo math; the cross-source merge
//! (`core::merge`) lands in M1 item 1.9, and the interpolation pipeline
//! (`core::sim`) in M2.

pub mod camera;
pub mod contracts;
pub mod error;
pub mod geo;
pub mod merge;
pub mod secret;
pub mod types;

pub use contracts::{
    AircraftCategory, AircraftMeta, Airport, AirportSize, LiveSource, RegionQuery, Store,
};
pub use error::{SourceError, StoreError};
pub use merge::{MergeStats, SessionTable};
pub use secret::SecretString;
pub use types::{
    BBox, BBoxError, CallSign, Icao24, Icao24ParseError, SourceId, StateVector, UnixSeconds,
    UnknownSourceId,
};
