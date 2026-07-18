//! `SQLite` persistence for Look Above: numbered migrations and the single writer thread that
//! owns every write (docs/08).
//!
//! Migration 0001 creates exactly two tables — `aircraft` and `source_status` — the pair item
//! 1.11 needs. The rest of docs/08's eventual schema (`positions`, `flights`, `airports`,
//! `runways`, `airlines`, `metars`) is tagged there with its own milestone (M3/M5) and lands
//! as its own numbered migration when that milestone needs it; migrations are append-only, so
//! nothing is created ahead of the point it is actually used.
//!
//! This crate does **not** yet implement `core::contracts::Store`: that trait's methods
//! (`insert_positions`, `upsert_aircraft_meta`, `airports_in_bbox`, `prune`) each need a table
//! (`positions`, `airports`) that does not exist until later migrations land. What exists here
//! instead is [`Writer`], a concrete handle scoped to what migration 0001 actually backs —
//! recording a poll cycle's outcome against `source_status`, and reading it back. Wiring
//! `Store` is a future item once `positions`/`airports` exist.

mod error;
mod migrations;
pub mod writer;

pub use writer::{SourceStatus, Writer};
