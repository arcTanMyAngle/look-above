//! Backs [`Writer::insert_flight`](crate::writer::Writer::insert_flight) and
//! [`Writer::latest_flight`](crate::writer::Writer::latest_flight) (migration 0004's `flights`
//! table, M3 item 3.4 — pulled forward from its originally planned M5 milestone, `DECISION_LOG`
//! 2026-07-21) — `core::contracts::Store::insert_flight`/`latest_flight`'s write/read halves.
//!
//! [`insert_flight`] is a plain `INSERT`, never an upsert: `flights.id` is a surrogate key, and
//! M3 only ever records one row per resolved, non-cached adsbdb route lookup. The
//! session-boundary merge (extending `last_seen`, detecting gaps via `positions`) `flights`' own
//! shape implies is still M5's job, once `positions` exists — see
//! `core::contracts::Store::insert_flight`'s own doc comment.

use look_above_core::contracts::Flight;
use look_above_core::error::StoreError;
use look_above_core::types::{CallSign, Icao24, UnixSeconds};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::backend_error;

/// Inserts one resolved flight/route observation — a plain insert, never an upsert (see module
/// doc).
pub(crate) fn insert_flight(conn: &Connection, flight: &Flight) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO flights (icao24, callsign, origin, destination, first_seen, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            flight.icao24.to_string(),
            flight.callsign.as_ref().map(CallSign::as_str),
            flight.origin,
            flight.destination,
            flight.first_seen.0,
            flight.last_seen.0,
        ],
    )
    .map_err(|error| backend_error(&error))?;
    Ok(())
}

/// The most recently observed row for `icao24` (highest `last_seen`), or `None` if none has
/// ever been recorded.
pub(crate) fn latest_flight(
    conn: &Connection,
    icao24: Icao24,
) -> Result<Option<Flight>, StoreError> {
    let row = conn
        .query_row(
            "SELECT callsign, origin, destination, first_seen, last_seen
             FROM flights WHERE icao24 = ?1 ORDER BY last_seen DESC LIMIT 1",
            params![icao24.to_string()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| backend_error(&error))?;

    Ok(row.map(
        |(callsign, origin, destination, first_seen, last_seen)| Flight {
            icao24,
            callsign: callsign.as_deref().and_then(CallSign::new),
            origin,
            destination,
            first_seen: UnixSeconds(first_seen),
            last_seen: UnixSeconds(last_seen),
        },
    ))
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

    fn flight(icao24: Icao24, first_seen: i64, last_seen: i64) -> Flight {
        Flight {
            icao24,
            callsign: CallSign::new("UAL123"),
            origin: Some("KJFK".to_owned()),
            destination: Some("KLAX".to_owned()),
            first_seen: UnixSeconds(first_seen),
            last_seen: UnixSeconds(last_seen),
        }
    }

    #[test]
    fn an_icao24_with_no_rows_returns_none() {
        let conn = migrated_conn();
        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        assert_eq!(latest_flight(&conn, icao24).expect("queries"), None);
    }

    #[test]
    fn inserting_then_querying_round_trips_every_field() {
        let conn = migrated_conn();
        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        let record = flight(icao24, 100, 200);
        insert_flight(&conn, &record).expect("inserts");

        assert_eq!(latest_flight(&conn, icao24).expect("queries"), Some(record));
    }

    #[test]
    fn a_second_row_with_a_later_last_seen_becomes_the_latest() {
        let conn = migrated_conn();
        let icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        insert_flight(&conn, &flight(icao24, 100, 150)).expect("first insert");
        let newer = flight(icao24, 200, 250);
        insert_flight(&conn, &newer).expect("second insert");

        assert_eq!(
            latest_flight(&conn, icao24).expect("queries"),
            Some(newer),
            "ORDER BY last_seen DESC must pick the newer row, not just any row"
        );
    }

    #[test]
    fn inserting_for_a_different_icao24_does_not_affect_the_first() {
        let conn = migrated_conn();
        let first_icao24 = Icao24::from_hex("a1b2c3").expect("valid test ICAO24");
        let other_icao24 = Icao24::from_hex("d4e5f6").expect("valid test ICAO24");
        let first_record = flight(first_icao24, 100, 200);
        insert_flight(&conn, &first_record).expect("first icao24");
        insert_flight(&conn, &flight(other_icao24, 500, 999)).expect("different icao24");

        assert_eq!(
            latest_flight(&conn, first_icao24).expect("queries"),
            Some(first_record)
        );
    }
}
