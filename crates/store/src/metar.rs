//! Backs [`Writer::upsert_metars`](crate::writer::Writer::upsert_metars) and
//! [`Writer::metars_for_stations`](crate::writer::Writer::metars_for_stations) (migration 0003,
//! M3 item 3.3) — `core::contracts::Store::upsert_metars`/`metars_for_stations`'s query half,
//! the same split `ourairports.rs` draws for the airports/runways queries.
//!
//! Retention (docs/08: "keep latest 2 per station, delete older on insert") is enforced here,
//! not left to a caller: every [`upsert_metars`] call prunes each station it just touched down
//! to its two freshest rows, in the same transaction as the insert.

use look_above_core::contracts::{FlightCategory, Metar};
use look_above_core::error::StoreError;
use look_above_core::types::UnixSeconds;
use rusqlite::{Connection, params, params_from_iter};

use crate::error::backend_error;

/// How many observations [`upsert_metars`] keeps per station — docs/08's retention line.
const RETAIN_PER_STATION: u32 = 2;

/// Upserts every observation in `batch` (one transaction), then prunes each touched station to
/// its [`RETAIN_PER_STATION`] most recent rows.
///
/// `(station, observed_at)` is the table's primary key (migration 0003), so re-upserting an
/// observation the poller already recorded overwrites it in place rather than duplicating it —
/// the same "insert re-sends are harmless" shape `writer::record_success` relies on for
/// `source_status`.
pub(crate) fn upsert_metars(conn: &Connection, batch: &[Metar]) -> Result<(), StoreError> {
    if batch.is_empty() {
        return Ok(());
    }

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| backend_error(&error))?;
    match upsert_and_prune(conn, batch) {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| backend_error(&error)),
        Err(error) => {
            // Best-effort: if the rollback itself fails there is nothing more useful to do than
            // report the original error that triggered it.
            let _ignored = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn upsert_and_prune(conn: &Connection, batch: &[Metar]) -> Result<(), StoreError> {
    {
        let mut upsert = conn
            .prepare(
                "INSERT INTO metars
                     (station, observed_at, raw, flight_cat, wind_dir_deg, wind_kt, visibility_sm)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (station, observed_at) DO UPDATE SET
                     raw = excluded.raw,
                     flight_cat = excluded.flight_cat,
                     wind_dir_deg = excluded.wind_dir_deg,
                     wind_kt = excluded.wind_kt,
                     visibility_sm = excluded.visibility_sm",
            )
            .map_err(|error| backend_error(&error))?;
        for metar in batch {
            upsert
                .execute(params![
                    metar.station,
                    metar.observed_at.0,
                    metar.raw,
                    metar.flight_category.map(FlightCategory::as_str),
                    metar.wind_dir_deg,
                    metar.wind_kt,
                    metar.visibility_sm,
                ])
                .map_err(|error| backend_error(&error))?;
        }
    }

    // Prune once per *distinct* station in the batch, not once per record — a batch that
    // re-observes the same station twice must not run the delete twice for no reason.
    let mut stations: Vec<&str> = batch.iter().map(|metar| metar.station.as_str()).collect();
    stations.sort_unstable();
    stations.dedup();

    let mut prune = conn
        .prepare(
            "DELETE FROM metars
             WHERE station = ?1
               AND observed_at NOT IN (
                   SELECT observed_at FROM metars
                   WHERE station = ?1
                   ORDER BY observed_at DESC
                   LIMIT ?2
               )",
        )
        .map_err(|error| backend_error(&error))?;
    for station in stations {
        prune
            .execute(params![station, RETAIN_PER_STATION])
            .map_err(|error| backend_error(&error))?;
    }
    Ok(())
}

/// The freshest cached observation for each of `stations` that has one — the query half of
/// `core::contracts::Store::metars_for_stations`.
///
/// `stations` is bound as a dynamic `IN (...)` list (`rusqlite::params_from_iter`) rather than
/// one query per station: the caller passes a whole viewport's worth at once (a handful to a
/// few dozen large airports), and one round trip is both simpler and cheaper than many.
pub(crate) fn metars_for_stations(
    conn: &Connection,
    stations: &[String],
) -> Result<Vec<Metar>, StoreError> {
    if stations.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = std::iter::repeat_n("?", stations.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT station, observed_at, raw, flight_cat, wind_dir_deg, wind_kt, visibility_sm
         FROM metars
         WHERE station IN ({placeholders})
           AND observed_at = (
               SELECT MAX(observed_at) FROM metars AS latest
               WHERE latest.station = metars.station
           )"
    );

    let mut stmt = conn.prepare(&sql).map_err(|error| backend_error(&error))?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<
        rusqlite::Result<(
            String,
            i64,
            String,
            Option<String>,
            Option<i32>,
            Option<i32>,
            Option<f64>,
        )>,
    > = stmt
        .query_map(params_from_iter(stations), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .map_err(|error| backend_error(&error))?
        .collect();

    rows.into_iter()
        .map(|row| {
            let (station, observed_at, raw, flight_cat, wind_dir_deg, wind_kt, visibility_sm) =
                row.map_err(|error| backend_error(&error))?;
            Ok(Metar {
                station,
                observed_at: UnixSeconds(observed_at),
                raw,
                flight_category: flight_cat
                    .as_deref()
                    .and_then(FlightCategory::from_metar_str),
                wind_dir_deg,
                wind_kt,
                visibility_sm,
            })
        })
        .collect()
}

/// Reads one `(station, observed_at)` row's `flight_cat`, for a test to check retention pruned
/// (or kept) a specific observation without going through [`metars_for_stations`]'s "freshest
/// only" filter.
#[cfg(test)]
fn row_exists(conn: &Connection, station: &str, observed_at: i64) -> bool {
    use rusqlite::OptionalExtension;

    conn.query_row(
        "SELECT 1 FROM metars WHERE station = ?1 AND observed_at = ?2",
        params![station, observed_at],
        |_| Ok(()),
    )
    .optional()
    .expect("query succeeds")
    .is_some()
}

#[cfg(test)]
mod tests {
    use crate::migrations;

    use super::*;

    fn migrated_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory connection opens");
        migrations::apply(&conn).expect("migrations apply");
        conn
    }

    fn metar(station: &str, observed_at: i64, category: Option<FlightCategory>) -> Metar {
        Metar {
            station: station.to_owned(),
            observed_at: UnixSeconds(observed_at),
            raw: format!("{station} {observed_at}Z RAW"),
            flight_category: category,
            wind_dir_deg: Some(280),
            wind_kt: Some(12),
            visibility_sm: Some(10.0),
        }
    }

    #[test]
    fn an_empty_batch_is_a_harmless_no_op() {
        let conn = migrated_conn();
        upsert_metars(&conn, &[]).expect("empty batch succeeds");
        assert_eq!(
            metars_for_stations(&conn, &["KJFK".to_owned()]).expect("queries"),
            Vec::new()
        );
    }

    #[test]
    fn upserting_then_querying_round_trips_every_field() {
        let conn = migrated_conn();
        let observation = metar("KJFK", 100, Some(FlightCategory::Ifr));
        upsert_metars(&conn, std::slice::from_ref(&observation)).expect("upserts");

        let found = metars_for_stations(&conn, &["KJFK".to_owned()]).expect("queries");
        assert_eq!(found, vec![observation]);
    }

    #[test]
    fn a_station_with_no_cached_observation_is_simply_absent() {
        let conn = migrated_conn();
        upsert_metars(&conn, &[metar("KJFK", 100, Some(FlightCategory::Vfr))]).expect("upserts");

        let found =
            metars_for_stations(&conn, &["KJFK".to_owned(), "KLAX".to_owned()]).expect("queries");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].station, "KJFK");
    }

    #[test]
    fn a_repeat_upsert_of_the_same_station_and_time_overwrites_in_place() {
        let conn = migrated_conn();
        upsert_metars(&conn, &[metar("KJFK", 100, Some(FlightCategory::Vfr))]).expect("first");
        upsert_metars(&conn, &[metar("KJFK", 100, Some(FlightCategory::Lifr))]).expect("second");

        let found = metars_for_stations(&conn, &["KJFK".to_owned()]).expect("queries");
        assert_eq!(
            found.len(),
            1,
            "same primary key must overwrite, not duplicate"
        );
        assert_eq!(found[0].flight_category, Some(FlightCategory::Lifr));
    }

    #[test]
    fn querying_returns_the_freshest_observation_per_station() {
        let conn = migrated_conn();
        upsert_metars(
            &conn,
            &[
                metar("KJFK", 100, Some(FlightCategory::Vfr)),
                metar("KJFK", 200, Some(FlightCategory::Mvfr)),
            ],
        )
        .expect("upserts");

        let found = metars_for_stations(&conn, &["KJFK".to_owned()]).expect("queries");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].observed_at, UnixSeconds(200));
        assert_eq!(found[0].flight_category, Some(FlightCategory::Mvfr));
    }

    #[test]
    fn a_third_observation_prunes_the_oldest_but_keeps_the_two_newest() {
        let conn = migrated_conn();
        upsert_metars(&conn, &[metar("KJFK", 100, None)]).expect("first");
        upsert_metars(&conn, &[metar("KJFK", 200, None)]).expect("second");
        upsert_metars(&conn, &[metar("KJFK", 300, None)]).expect("third");

        assert!(
            !row_exists(&conn, "KJFK", 100),
            "the oldest of three must be pruned"
        );
        assert!(row_exists(&conn, "KJFK", 200));
        assert!(row_exists(&conn, "KJFK", 300));
    }

    #[test]
    fn pruning_one_station_does_not_touch_another() {
        let conn = migrated_conn();
        upsert_metars(
            &conn,
            &[
                metar("KJFK", 100, None),
                metar("KJFK", 200, None),
                metar("KJFK", 300, None),
                metar("KLAX", 100, None),
            ],
        )
        .expect("upserts");

        assert!(!row_exists(&conn, "KJFK", 100), "KJFK's oldest is pruned");
        assert!(row_exists(&conn, "KLAX", 100), "KLAX is untouched");
    }

    #[test]
    fn an_unparseable_stored_flight_cat_reads_back_as_none_rather_than_erroring() {
        // Simulates a future/foreign value landing in the column outside this crate's own
        // writes (or a value `aviationweather.gov` stops reporting) — the read path must not
        // fail the whole query over one row's unrecognized category.
        let conn = migrated_conn();
        conn.execute(
            "INSERT INTO metars (station, observed_at, raw, flight_cat) VALUES (?1, ?2, ?3, ?4)",
            params!["KJFK", 100_i64, "KJFK RAW", "UNKNOWN"],
        )
        .expect("direct insert");

        let found = metars_for_stations(&conn, &["KJFK".to_owned()]).expect("queries");
        assert_eq!(found[0].flight_category, None);
    }
}
