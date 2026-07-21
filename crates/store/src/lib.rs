//! `SQLite` persistence for Look Above: numbered migrations and the single writer thread that
//! owns every write (docs/08).
//!
//! Migration 0001 creates `aircraft` and `source_status` — the pair item 1.11 needs. Migration
//! 0002 adds `airports` and `runways` (M3 item 3.1, the `OurAirports` import), seeded on first
//! run from the bundled snapshot in `assets/ourairports/` (see [`ourairports`]). Migration 0003
//! adds `metars` (M3 item 3.3, see [`metar`]). The rest of docs/08's eventual schema
//! (`positions`, `flights`, `airlines`) is tagged there with its own milestone (M5) and lands as
//! its own numbered migration when that milestone needs it; migrations are append-only, so
//! nothing is created ahead of the point it is actually used.
//!
//! This crate does **not** yet implement `core::contracts::Store`: that trait's methods
//! (`insert_positions`, `upsert_aircraft_meta`, `prune`) each need a table that does not exist,
//! or a batch-write/prune shape that isn't built, until later items land (`positions` is M5's).
//! What exists here instead is [`Writer`], a concrete handle scoped to what migrations
//! 0001–0003 actually back: recording a poll cycle's outcome against `source_status` and
//! reading it back, plus [`Writer::airports_in_bbox`]/[`Writer::runways_in_bbox`]/
//! [`Writer::upsert_metars`]/[`Writer::metars_for_stations`] — the methods of the eventual
//! `Store` trait these items' tables can already serve, with the same signatures
//! `core::contracts::Store` declares. Wiring the full `Store` trait is a future item once
//! `positions` exists.

mod error;
mod metar;
mod migrations;
mod ourairports;
pub mod writer;

pub use writer::{SourceStatus, Writer};
