//! `SQLite` persistence for Look Above: numbered migrations and the single writer thread that
//! owns every write (docs/08).
//!
//! Migration 0001 creates `aircraft` and `source_status` — the pair item 1.11 needs. Migration
//! 0002 adds `airports` and `runways` (M3 item 3.1, the `OurAirports` import), seeded on first
//! run from the bundled snapshot in `assets/ourairports/` (see [`ourairports`]). The rest of
//! docs/08's eventual schema (`positions`, `flights`, `airlines`, `metars`) is tagged there with
//! its own milestone (M3/M5) and lands as its own numbered migration when that milestone needs
//! it; migrations are append-only, so nothing is created ahead of the point it is actually used.
//!
//! This crate does **not** yet implement `core::contracts::Store`: that trait's methods
//! (`insert_positions`, `upsert_aircraft_meta`, `airports_in_bbox`, `prune`) each need a table
//! that does not exist, or a batch-write/prune shape that isn't built, until later items land
//! (`positions` is M5's). What exists here instead is [`Writer`], a concrete handle scoped to
//! what migrations 0001/0002 actually back: recording a poll cycle's outcome against
//! `source_status` and reading it back, plus [`Writer::airports_in_bbox`] — the one method of
//! the eventual `Store` trait this item's tables can already serve, with the same signature
//! `core::contracts::Store::airports_in_bbox` declares. Wiring the full `Store` trait is a
//! future item once `positions` exists.

mod error;
mod migrations;
mod ourairports;
pub mod writer;

pub use writer::{SourceStatus, Writer};
