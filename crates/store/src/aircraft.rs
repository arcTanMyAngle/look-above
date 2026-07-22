//! Backs [`Writer::upsert_aircraft_meta`](crate::writer::Writer::upsert_aircraft_meta) and
//! [`Writer::aircraft_meta`](crate::writer::Writer::aircraft_meta) — `core::contracts::Store::
//! upsert_aircraft_meta`/`aircraft_meta`'s read/write halves, the same split `metar.rs` draws
//! for METARs.
//!
//! Migration 0001 already ships the `aircraft` table (M1 item 1.11), but `fetched_at`/
//! `lookup_failed_at` sat unused until M3 item 3.4's adsbdb lookups: this module is what first
//! reads and writes them, backing the 24 h negative-cache gate `Store::aircraft_meta`'s own doc
//! comment describes (`DECISION_LOG` 2026-07-21, M3 3.4). The cache-gate *decision* itself (worth
//! a fresh lookup or not) is `ingest::adsbdb`'s job, not this crate's.

use look_above_core::contracts::{AircraftCategory, AircraftMeta};
use look_above_core::error::StoreError;
use look_above_core::types::{Icao24, UnixSeconds};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::backend_error;

/// Upserts `meta` by `icao24` (the table's primary key, migration 0001) — a second call for the
/// same `icao24` overwrites the row rather than inserting a second one, the same "insert
/// re-sends are harmless" shape `writer::record_success` relies on for `source_status`.
pub(crate) fn upsert_aircraft_meta(
    conn: &Connection,
    meta: &AircraftMeta,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO aircraft
             (icao24, registration, type_code, category, operator, is_anonymous, fetched_at,
              lookup_failed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (icao24) DO UPDATE SET
             registration = excluded.registration,
             type_code = excluded.type_code,
             category = excluded.category,
             operator = excluded.operator,
             is_anonymous = excluded.is_anonymous,
             fetched_at = excluded.fetched_at,
             lookup_failed_at = excluded.lookup_failed_at",
        params![
            meta.icao24.to_string(),
            meta.registration,
            meta.type_code,
            meta.category.as_str(),
            meta.operator,
            meta.is_anonymous,
            meta.fetched_at.map(|ts| ts.0),
            meta.lookup_failed_at.map(|ts| ts.0),
        ],
    )
    .map_err(|error| backend_error(&error))?;
    Ok(())
}

/// The cached row for `icao24`, or `None` if it has never been looked up (no row exists yet).
pub(crate) fn aircraft_meta(
    conn: &Connection,
    icao24: Icao24,
) -> Result<Option<AircraftMeta>, StoreError> {
    conn.query_row(
        "SELECT registration, type_code, category, operator, is_anonymous, fetched_at,
                lookup_failed_at
         FROM aircraft WHERE icao24 = ?1",
        params![icao24.to_string()],
        |row| {
            Ok(AircraftMeta {
                icao24,
                registration: row.get(0)?,
                type_code: row.get(1)?,
                category: row
                    .get::<_, Option<String>>(2)?
                    .as_deref()
                    .map_or(AircraftCategory::Unknown, AircraftCategory::from_store_str),
                operator: row.get(3)?,
                is_anonymous: row.get(4)?,
                fetched_at: row.get::<_, Option<i64>>(5)?.map(UnixSeconds),
                lookup_failed_at: row.get::<_, Option<i64>>(6)?.map(UnixSeconds),
            })
        },
    )
    .optional()
    .map_err(|error| backend_error(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn migrated_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory connection opens");
        migrations::apply(&conn).expect("migrations apply");
        conn
    }

    fn meta(icao24: Icao24, category: AircraftCategory) -> AircraftMeta {
        AircraftMeta {
            icao24,
            registration: Some("N12345".to_owned()),
            type_code: Some("B738".to_owned()),
            category,
            operator: Some("Test Air".to_owned()),
            is_anonymous: false,
            fetched_at: Some(UnixSeconds(100)),
            lookup_failed_at: None,
        }
    }

    #[test]
    fn a_never_looked_up_icao24_returns_none() {
        let conn = migrated_conn();
        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        assert_eq!(aircraft_meta(&conn, icao24).expect("queries"), None);
    }

    #[test]
    fn upserting_then_querying_round_trips_every_field() {
        let conn = migrated_conn();
        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        let record = meta(icao24, AircraftCategory::Jet);
        upsert_aircraft_meta(&conn, &record).expect("upserts");

        assert_eq!(aircraft_meta(&conn, icao24).expect("queries"), Some(record));
    }

    #[test]
    fn a_second_upsert_for_the_same_icao24_overwrites_rather_than_inserting_a_second_row() {
        let conn = migrated_conn();
        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        upsert_aircraft_meta(&conn, &meta(icao24, AircraftCategory::Jet)).expect("first");
        upsert_aircraft_meta(&conn, &meta(icao24, AircraftCategory::Turboprop)).expect("second");

        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM aircraft WHERE icao24 = ?1",
                params![icao24.to_string()],
                |row| row.get(0),
            )
            .expect("counts rows");
        assert_eq!(
            row_count, 1,
            "icao24 is the primary key: a repeat write must overwrite, not duplicate"
        );

        let found = aircraft_meta(&conn, icao24)
            .expect("queries")
            .expect("row exists");
        assert_eq!(found.category, AircraftCategory::Turboprop);
    }

    #[test]
    fn an_unrecognized_stored_category_reads_back_as_unknown_rather_than_erroring() {
        // Simulates a future/foreign value landing in the column outside this crate's own
        // writes — the read path must not fail the whole query over one row's unrecognized
        // category (the same shape `metar.rs` covers for `flight_cat`).
        let conn = migrated_conn();
        conn.execute(
            "INSERT INTO aircraft (icao24, category) VALUES (?1, ?2)",
            params!["a1b2c3", "rocket"],
        )
        .expect("direct insert");

        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        let found = aircraft_meta(&conn, icao24)
            .expect("queries")
            .expect("row exists");
        assert_eq!(found.category, AircraftCategory::Unknown);
    }
}
