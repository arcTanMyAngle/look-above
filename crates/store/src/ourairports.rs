//! Seeds `airports`/`runways` (migration 0002) from the bundled `OurAirports` snapshot, and
//! serves [`Writer::airports_in_bbox`]'s and [`Writer::runways_in_bbox`]'s queries (docs/09
//! `Store::airports_in_bbox`/`Store::runways_in_bbox`).
//!
//! The snapshot itself is produced by `crates/import`'s `import-ourairports` binary — this
//! crate never touches the network (M0 acceptance line 3) and never re-derives the type-drop
//! decision (`seaplane_base`/`balloonport`/`closed` are already gone from the bundled CSVs by
//! the time this module sees them). The two files are `include_str!`-embedded, the same
//! self-contained-binary reasoning `migrations.rs` already documents for its own SQL files: the
//! compiled binary needs nothing on disk beside it to seed a fresh database.
//!
//! Seeding is idempotent by row count, not by migration version: `Writer::open` runs against a
//! persistent on-disk file on every app start, so "the `airports` table is already non-empty"
//! is the check, not "this is the first time migration 0002 has applied" (docs/08's own
//! migration mechanism is schema-only and says nothing about *data*).

use look_above_core::contracts::{Airport, AirportSize, Runway};
use look_above_core::error::StoreError;
use look_above_core::types::BBox;
use rusqlite::{Connection, params};
use serde::Deserialize;

use crate::error::backend_error;

/// The bundled, trimmed snapshot `import-ourairports` writes (`crates/import/src/
/// import_ourairports.rs`) — column-for-column with the `airports` table (docs/08), already
/// filtered to the kept `AirportSize` tiers.
const BUNDLED_AIRPORTS_CSV: &str = include_str!("../assets/ourairports/airports.csv");

/// The bundled, trimmed runways snapshot, column-for-column with the `runways` table
/// (docs/08), already filtered to runways whose airport survived the type-drop above.
const BUNDLED_RUNWAYS_CSV: &str = include_str!("../assets/ourairports/runways.csv");

/// One row of the bundled `airports.csv` asset.
#[derive(Debug, Deserialize)]
struct BundledAirportRow {
    ident: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    lat: f64,
    lon: f64,
    elevation_ft: Option<i32>,
    iso_country: Option<String>,
    iata: Option<String>,
}

/// One row of the bundled `runways.csv` asset.
#[derive(Debug, Deserialize)]
struct BundledRunwayRow {
    airport_ident: String,
    le_ident: Option<String>,
    le_lat: Option<f64>,
    le_lon: Option<f64>,
    le_heading_deg: Option<f64>,
    he_ident: Option<String>,
    he_lat: Option<f64>,
    he_lon: Option<f64>,
    he_heading_deg: Option<f64>,
    length_ft: Option<i32>,
    width_ft: Option<i32>,
    surface: Option<String>,
    closed: i64,
}

/// Seeds `airports`/`runways` from the bundled snapshot unless `airports` already has rows.
///
/// Called once from `Writer::open`, before the writer thread starts, so a broken bundled asset
/// (a build-time defect, not a runtime one) surfaces to the caller as an `Err` the same way a
/// broken migration does, rather than silently leaving the tables empty.
pub(crate) fn seed_if_empty(conn: &Connection) -> Result<(), StoreError> {
    let existing_airports: i64 = conn
        .query_row("SELECT COUNT(*) FROM airports", [], |row| row.get(0))
        .map_err(|error| backend_error(&error))?;
    if existing_airports > 0 {
        return Ok(());
    }
    seed(conn)
}

/// Parses both bundled CSVs and inserts every row in one transaction.
fn seed(conn: &Connection) -> Result<(), StoreError> {
    let airports = parse_bundled_airports()?;
    let runways = parse_bundled_runways()?;

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| backend_error(&error))?;
    match insert_seed_rows(conn, &airports, &runways) {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| backend_error(&error)),
        Err(error) => {
            // Best-effort: if the rollback itself fails there is nothing more useful to do
            // than report the original error that triggered it.
            let _ignored = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn insert_seed_rows(
    conn: &Connection,
    airports: &[BundledAirportRow],
    runways: &[BundledRunwayRow],
) -> Result<(), StoreError> {
    {
        let mut insert_airport = conn
            .prepare(
                "INSERT INTO airports (ident, name, type, lat, lon, elevation_ft, iso_country, iata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|error| backend_error(&error))?;
        for row in airports {
            insert_airport
                .execute(params![
                    row.ident,
                    row.name,
                    row.kind,
                    row.lat,
                    row.lon,
                    row.elevation_ft,
                    row.iso_country,
                    row.iata,
                ])
                .map_err(|error| backend_error(&error))?;
        }
    }
    {
        let mut insert_runway = conn
            .prepare(
                "INSERT INTO runways (airport_ident, le_ident, le_lat, le_lon, le_heading_deg,
                                       he_ident, he_lat, he_lon, he_heading_deg, length_ft,
                                       width_ft, surface, closed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )
            .map_err(|error| backend_error(&error))?;
        for row in runways {
            insert_runway
                .execute(params![
                    row.airport_ident,
                    row.le_ident,
                    row.le_lat,
                    row.le_lon,
                    row.le_heading_deg,
                    row.he_ident,
                    row.he_lat,
                    row.he_lon,
                    row.he_heading_deg,
                    row.length_ft,
                    row.width_ft,
                    row.surface,
                    row.closed,
                ])
                .map_err(|error| backend_error(&error))?;
        }
    }
    Ok(())
}

fn parse_bundled_airports() -> Result<Vec<BundledAirportRow>, StoreError> {
    csv::Reader::from_reader(BUNDLED_AIRPORTS_CSV.as_bytes())
        .deserialize::<BundledAirportRow>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| StoreError::SeedAsset {
            asset: "ourairports/airports.csv".to_owned(),
            message: error.to_string(),
        })
}

fn parse_bundled_runways() -> Result<Vec<BundledRunwayRow>, StoreError> {
    csv::Reader::from_reader(BUNDLED_RUNWAYS_CSV.as_bytes())
        .deserialize::<BundledRunwayRow>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| StoreError::SeedAsset {
            asset: "ourairports/runways.csv".to_owned(),
            message: error.to_string(),
        })
}

/// Airports within `bbox` at or above `min_size` — the query half of `core::contracts::Store::
/// airports_in_bbox`, run against the connection the writer thread owns.
///
/// The `bbox` clause is plain `BETWEEN`, per docs/08's "no `SQLite` spatial extension" note;
/// `min_size` is filtered in Rust rather than SQL, since it needs the same `type` string ->
/// `AirportSize` mapping the importer used, not a second copy of that ladder written as a SQL
/// `CASE` expression.
pub(crate) fn airports_in_bbox(
    conn: &Connection,
    bbox: BBox,
    min_size: AirportSize,
) -> Result<Vec<Airport>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT ident, name, type, lat, lon, elevation_ft, iso_country, iata
             FROM airports
             WHERE lat BETWEEN ?1 AND ?2 AND lon BETWEEN ?3 AND ?4",
        )
        .map_err(|error| backend_error(&error))?;

    #[allow(clippy::type_complexity)]
    let rows: Vec<
        rusqlite::Result<(
            String,
            String,
            String,
            f64,
            f64,
            Option<i32>,
            Option<String>,
            Option<String>,
        )>,
    > = stmt
        .query_map(
            params![
                bbox.lat_min(),
                bbox.lat_max(),
                bbox.lon_min(),
                bbox.lon_max(),
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|error| backend_error(&error))?
        .collect();

    let mut airports = Vec::with_capacity(rows.len());
    for row in rows {
        let (ident, name, type_str, lat, lon, elevation_ft, iso_country, iata) =
            row.map_err(|error| backend_error(&error))?;
        // Every seeded row was already through the type-drop at import time, so this should
        // always be `Some` — a `None` here would mean the bundled asset itself is corrupt.
        // Skipping rather than erroring keeps one bad row from failing the whole query.
        let Some(size) = AirportSize::from_ourairports_type(&type_str) else {
            continue;
        };
        if size < min_size {
            continue;
        }
        airports.push(Airport {
            ident,
            name,
            size,
            lat_deg: lat,
            lon_deg: lon,
            elevation_ft,
            iso_country,
            iata,
        });
    }
    Ok(airports)
}

/// Runways whose airport is within `bbox` at or above `min_size` — the query half of the new
/// `runways_in_bbox` contract (docs/09), run against the connection the writer thread owns.
///
/// `closed = 0` is filtered directly in SQL (cheap, direct, unlike the `type` -> `AirportSize`
/// mapping below), joined against `airports` for the same `bbox` `BETWEEN` clause
/// `airports_in_bbox` uses; `min_size` still needs `AirportSize::from_ourairports_type` against
/// `airports.type`, so that column is selected and filtered in Rust exactly like
/// `airports_in_bbox` does, skipping any row whose airport type isn't recognized. A runway with
/// a `NULL` `le_*`/`he_*` end (the source CSV has some incomplete rows) is still returned — the
/// render side decides what to draw for a partial runway, not this query.
pub(crate) fn runways_in_bbox(
    conn: &Connection,
    bbox: BBox,
    min_size: AirportSize,
) -> Result<Vec<Runway>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT airports.type, runways.airport_ident, runways.le_ident, runways.le_lat,
                    runways.le_lon, runways.le_heading_deg, runways.he_ident, runways.he_lat,
                    runways.he_lon, runways.he_heading_deg, runways.length_ft, runways.width_ft,
                    runways.surface
             FROM runways
             JOIN airports ON runways.airport_ident = airports.ident
             WHERE airports.lat BETWEEN ?1 AND ?2 AND airports.lon BETWEEN ?3 AND ?4
               AND runways.closed = 0",
        )
        .map_err(|error| backend_error(&error))?;

    #[allow(clippy::type_complexity)]
    let rows: Vec<
        rusqlite::Result<(
            String,
            String,
            Option<String>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<String>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<i32>,
            Option<i32>,
            Option<String>,
        )>,
    > = stmt
        .query_map(
            params![
                bbox.lat_min(),
                bbox.lat_max(),
                bbox.lon_min(),
                bbox.lon_max(),
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                ))
            },
        )
        .map_err(|error| backend_error(&error))?
        .collect();

    let mut runways = Vec::with_capacity(rows.len());
    for row in rows {
        let (
            type_str,
            airport_ident,
            le_ident,
            le_lat_deg,
            le_lon_deg,
            le_heading_deg,
            he_ident,
            he_lat_deg,
            he_lon_deg,
            he_heading_deg,
            length_ft,
            width_ft,
            surface,
        ) = row.map_err(|error| backend_error(&error))?;
        // Same reasoning as `airports_in_bbox`: every seeded airport already passed the
        // type-drop at import time, so `None` here would mean the bundled asset is corrupt.
        // Skip rather than error so one bad row doesn't fail the whole query.
        let Some(size) = AirportSize::from_ourairports_type(&type_str) else {
            continue;
        };
        if size < min_size {
            continue;
        }
        runways.push(Runway {
            airport_ident,
            le_ident,
            le_lat_deg,
            le_lon_deg,
            le_heading_deg,
            he_ident,
            he_lat_deg,
            he_lon_deg,
            he_heading_deg,
            length_ft,
            width_ft,
            surface,
        });
    }
    Ok(runways)
}

#[cfg(test)]
mod tests {
    use look_above_core::types::BBox;

    use super::*;
    use crate::migrations;

    fn migrated_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory connection opens");
        migrations::apply(&conn).expect("migrations apply");
        conn
    }

    #[test]
    fn bundled_csvs_parse_without_error() {
        let airports = parse_bundled_airports().expect("bundled airports.csv parses");
        let runways = parse_bundled_runways().expect("bundled runways.csv parses");
        assert!(!airports.is_empty(), "bundled airports.csv is not empty");
        assert!(!runways.is_empty(), "bundled runways.csv is not empty");
    }

    #[test]
    fn every_bundled_runway_references_a_bundled_airport() {
        let airports = parse_bundled_airports().expect("parses");
        let runways = parse_bundled_runways().expect("parses");
        let idents: std::collections::HashSet<&str> =
            airports.iter().map(|a| a.ident.as_str()).collect();
        for runway in &runways {
            assert!(
                idents.contains(runway.airport_ident.as_str()),
                "runway references dropped airport {}",
                runway.airport_ident
            );
        }
    }

    #[test]
    fn seeding_an_empty_database_populates_both_tables() {
        let conn = migrated_conn();
        seed_if_empty(&conn).expect("seeds");

        let airport_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM airports", [], |row| row.get(0))
            .expect("counts airports");
        let runway_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runways", [], |row| row.get(0))
            .expect("counts runways");
        assert!(airport_count > 0, "airports should be seeded");
        assert!(runway_count > 0, "runways should be seeded");
    }

    /// Airport row count must land within 5% of the bundled CSV's own row count — the bundled
    /// CSV *is* the kept-type source (`seaplane_base`/`balloonport`/`closed` already dropped by
    /// `import-ourairports`), so this doubles as the acceptance line's "kept-type" comparison
    /// for the seeded database.
    ///
    /// Row counts here (tens of thousands) are nowhere near `f64`'s 52-bit mantissa limit, so
    /// the widening casts below lose nothing in practice; the lint is silenced rather than
    /// worked around with an equivalent-but-noisier `u32` round trip.
    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn seeded_airport_count_is_within_five_percent_of_the_bundled_csv_row_count() {
        let conn = migrated_conn();
        seed_if_empty(&conn).expect("seeds");

        let bundled_row_count = parse_bundled_airports().expect("parses").len() as f64;
        let db_row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM airports", [], |row| row.get(0))
            .expect("counts airports");

        let difference = (db_row_count as f64 - bundled_row_count).abs();
        let tolerance = bundled_row_count * 0.05;
        assert!(
            difference <= tolerance,
            "seeded count {db_row_count} vs bundled CSV row count {bundled_row_count} \
             differs by more than 5%"
        );
    }

    #[test]
    fn re_seeding_an_already_seeded_database_does_not_duplicate_rows() {
        let conn = migrated_conn();
        seed_if_empty(&conn).expect("first seed");
        let after_first: i64 = conn
            .query_row("SELECT COUNT(*) FROM airports", [], |row| row.get(0))
            .expect("counts airports");

        // Simulates a second `Writer::open` against the same on-disk file.
        seed_if_empty(&conn).expect("second call is a no-op, not a duplicate insert");
        let after_second: i64 = conn
            .query_row("SELECT COUNT(*) FROM airports", [], |row| row.get(0))
            .expect("counts airports");

        assert_eq!(
            after_first, after_second,
            "seeding twice must not duplicate rows"
        );
    }

    #[test]
    fn airports_in_bbox_returns_only_airports_within_the_box_and_at_or_above_min_size() {
        let conn = migrated_conn();
        // A small, hand-built fixture rather than the full bundled asset, so the expected
        // subset is known exactly instead of depending on live OurAirports contents.
        conn.execute_batch(
            "INSERT INTO airports (ident, name, type, lat, lon, elevation_ft, iso_country, iata)
             VALUES
                ('KJFK', 'John F Kennedy Intl', 'large_airport', 40.6413, -73.7781, 13, 'US', 'JFK'),
                ('KTEB', 'Teterboro', 'medium_airport', 40.8501, -74.0608, 9, 'US', 'TEB'),
                ('N51',  'Small Field', 'small_airport', 40.7, -74.0, 100, 'US', NULL),
                ('KLAX', 'Los Angeles Intl', 'large_airport', 33.9425, -118.4081, 125, 'US', 'LAX');",
        )
        .expect("fixture inserts");

        let bbox = BBox::new(40.0, -75.0, 41.0, -73.0).expect("valid bbox");

        let large_and_medium = airports_in_bbox(&conn, bbox, AirportSize::Medium).expect("queries");
        let idents: std::collections::HashSet<&str> =
            large_and_medium.iter().map(|a| a.ident.as_str()).collect();
        assert_eq!(idents, std::collections::HashSet::from(["KJFK", "KTEB"]));

        let everything_in_bbox =
            airports_in_bbox(&conn, bbox, AirportSize::Heliport).expect("queries");
        let idents: std::collections::HashSet<&str> = everything_in_bbox
            .iter()
            .map(|a| a.ident.as_str())
            .collect();
        assert_eq!(
            idents,
            std::collections::HashSet::from(["KJFK", "KTEB", "N51"]),
            "KLAX is outside the bbox and must not appear regardless of size"
        );
    }

    #[test]
    fn runways_in_bbox_excludes_out_of_bbox_below_size_and_closed_runways() {
        let conn = migrated_conn();
        // A small, hand-built fixture rather than the full bundled asset, so the expected
        // subset is known exactly instead of depending on live OurAirports contents.
        conn.execute_batch(
            "INSERT INTO airports (ident, name, type, lat, lon, elevation_ft, iso_country, iata)
             VALUES
                ('KJFK', 'John F Kennedy Intl', 'large_airport', 40.6413, -73.7781, 13, 'US', 'JFK'),
                ('N51',  'Small Field', 'small_airport', 40.7, -74.0, 100, 'US', NULL),
                ('KLAX', 'Los Angeles Intl', 'large_airport', 33.9425, -118.4081, 125, 'US', 'LAX');

             INSERT INTO runways (airport_ident, le_ident, le_lat, le_lon, le_heading_deg,
                                   he_ident, he_lat, he_lon, he_heading_deg, length_ft,
                                   width_ft, surface, closed)
             VALUES
                -- in-bbox, above min_size: returned.
                ('KJFK', '04L', 40.626, -73.784, 40.0, '22R', 40.656, -73.769, 220.0,
                 12079, 200, 'ASP', 0),
                -- in-bbox, but below min_size (small_airport < Medium): excluded.
                ('N51', '09', 40.699, -74.001, 90.0, '27', 40.701, -73.999, 270.0,
                 2500, 50, 'TURF', 0),
                -- out-of-bbox airport: excluded regardless of size.
                ('KLAX', '06L', 33.936, -118.436, 60.0, '24R', 33.949, -118.380, 240.0,
                 10285, 150, 'CON', 0),
                -- in-bbox, above min_size, but closed: excluded even though the airport qualifies.
                ('KJFK', '13L', 40.648, -73.833, 130.0, '31R', 40.630, -73.762, 310.0,
                 3460, 150, 'ASP', 1);",
        )
        .expect("fixture inserts");

        let bbox = BBox::new(40.0, -75.0, 41.0, -73.0).expect("valid bbox");

        let large_only = runways_in_bbox(&conn, bbox, AirportSize::Medium).expect("queries");
        assert_eq!(
            large_only.len(),
            1,
            "only KJFK's open, in-bbox, above-min-size runway should be returned"
        );
        assert_eq!(large_only[0].airport_ident, "KJFK");
        assert_eq!(large_only[0].le_ident.as_deref(), Some("04L"));
        assert_eq!(large_only[0].he_ident.as_deref(), Some("22R"));

        let everything_in_bbox =
            runways_in_bbox(&conn, bbox, AirportSize::Heliport).expect("queries");
        let idents: std::collections::HashSet<&str> = everything_in_bbox
            .iter()
            .map(|runway| runway.airport_ident.as_str())
            .collect();
        assert_eq!(
            idents,
            std::collections::HashSet::from(["KJFK", "N51"]),
            "N51's small-airport runway is in bbox and should appear once min_size allows it; \
             KLAX must still be excluded (out of bbox) and KJFK's closed runway must still be \
             excluded regardless of min_size"
        );
        assert_eq!(
            everything_in_bbox.len(),
            2,
            "KJFK's closed runway must never appear, even at the most permissive min_size"
        );
    }
}
