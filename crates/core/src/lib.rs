//! Core types, geo math, and simulation contracts for Look Above.
//!
//! This crate must stay free of I/O dependencies: no network, no DB, no GPU.
//! Types (`StateVector`, `Icao24`, …) and the `LiveSource`/`Store` traits land
//! in M0 items 0.3–0.4.
