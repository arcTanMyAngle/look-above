//! `SQLite` persistence for Look Above: numbered migrations and the single writer thread that
//! owns every write (docs/08).
//!
//! Migration 0001 creates `aircraft` and `source_status` — the pair item 1.11 needs. Migration
//! 0002 adds `airports` and `runways` (M3 item 3.1, the `OurAirports` import), seeded on first
//! run from the bundled snapshot in `assets/ourairports/` (see [`ourairports`]). Migration 0003
//! adds `metars` (M3 item 3.3, see [`metar`]). Migration 0004 adds `flights` (M3 item 3.4,
//! pulled forward from its originally planned M5 milestone to back on-selection adsbdb route
//! caching — `DECISION_LOG` 2026-07-21, M3 3.4; see [`flights`]), and that same item is also what
//! first implements reads/writes against migration 0001's `aircraft` table (see [`aircraft`]),
//! whose `fetched_at`/`lookup_failed_at` columns shipped back in M1 unused until now. The rest
//! of docs/08's eventual schema (`positions`, `airlines`) is tagged there with its own milestone
//! (M5, M3+ respectively) and lands as its own numbered migration when that milestone needs it;
//! migrations are append-only, so nothing is created ahead of the point it is actually used.
//!
//! This crate does **not** yet implement `core::contracts::Store`: that trait's `insert_positions`
//! and `prune` methods each need the `positions` table, which doesn't exist until M5's batch
//! write/prune shape lands. What exists here instead is [`Writer`], a concrete handle scoped to
//! what migrations 0001–0004 actually back: recording a poll cycle's outcome against
//! `source_status` and reading it back, plus [`Writer::airports_in_bbox`]/
//! [`Writer::runways_in_bbox`]/[`Writer::upsert_metars`]/[`Writer::metars_for_stations`]/
//! [`Writer::upsert_aircraft_meta`]/[`Writer::aircraft_meta`]/[`Writer::insert_flight`]/
//! [`Writer::latest_flight`] — the methods of the eventual `Store` trait these items' tables can
//! already serve, with the same signatures `core::contracts::Store` declares. Wiring the full
//! `Store` trait is a future item once `positions` exists.

mod aircraft;
mod error;
mod flights;
mod metar;
mod migrations;
mod ourairports;
pub mod writer;

pub use writer::{SourceStatus, Writer};
