//! The single writer thread: the one place that owns the write connection (docs/08: "all
//! writes go through a single writer thread owning the connection; readers use separate
//! read-only connections").
//!
//! [`Writer`] is the cheap-to-clone handle every caller holds; it never touches `SQLite`
//! itself, only a `crossbeam` command channel the dedicated thread drains. Migration 0001
//! backs recording a poll cycle's outcome against `source_status`, and reading it back — which
//! is also the other half of 1.7's seam: the `credits_used_today` [`Writer::source_status`]
//! hands back is the raw value `ingest::budget::CreditLedger::restored` rehydrates from. That
//! call itself happens in `ingest`/`app` wiring (a later item); this crate only stores and
//! returns the counter, with no notion of UTC-day rollover — `restored` already tolerates a
//! stale persisted value. Migration 0002 backs [`Writer::airports_in_bbox`] (M3 item 3.1) and
//! [`Writer::runways_in_bbox`] (M3 item 3.2), both seeded once from the bundled `OurAirports`
//! snapshot in `crate::ourairports`.
//!
//! The command set is one enum behind one channel, not one channel per operation, on purpose:
//! a later item can add a variant for `positions` (once that table exists) without changing
//! [`Writer`]'s public shape.

use std::path::Path;
use std::thread;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use look_above_core::contracts::{Airport, AirportSize, Metar, Runway};
use look_above_core::error::StoreError;
use look_above_core::types::{BBox, SourceId, UnixSeconds};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::backend_error;
use crate::metar;
use crate::migrations;
use crate::ourairports;

/// A `source_status` row read back — the poll-health snapshot the debug overlay (M1) and the
/// credit-ledger restore (`ingest::budget::CreditLedger::restored`) both want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    pub source: SourceId,
    pub last_success: Option<UnixSeconds>,
    pub last_error: Option<UnixSeconds>,
    pub last_error_msg: Option<String>,
    pub credits_used_today: u32,
}

/// Commands the writer thread understands, each carrying its own one-shot reply channel.
///
/// A fresh `bounded(1)` reply per call is what keeps every [`Writer`] method synchronous
/// (docs/09: "Sync API; called from the writer thread only" — the *callers* here are sync,
/// the thread itself is the one place `SQLite` is touched), while the one shared
/// `Sender<Command>` is what lets the set grow later without breaking [`Writer`]'s shape.
enum Command {
    RecordSuccess {
        source: SourceId,
        at: UnixSeconds,
        credits_used_today: u32,
        reply: Sender<Result<(), StoreError>>,
    },
    RecordError {
        source: SourceId,
        at: UnixSeconds,
        message: String,
        reply: Sender<Result<(), StoreError>>,
    },
    SourceStatus {
        source: SourceId,
        reply: Sender<Result<Option<SourceStatus>, StoreError>>,
    },
    AirportsInBbox {
        bbox: BBox,
        min_size: AirportSize,
        reply: Sender<Result<Vec<Airport>, StoreError>>,
    },
    RunwaysInBbox {
        bbox: BBox,
        min_size: AirportSize,
        reply: Sender<Result<Vec<Runway>, StoreError>>,
    },
    UpsertMetars {
        batch: Vec<Metar>,
        reply: Sender<Result<(), StoreError>>,
    },
    MetarsForStations {
        stations: Vec<String>,
        reply: Sender<Result<Vec<Metar>, StoreError>>,
    },
}

/// A handle to the writer thread. Cheap to clone (it is just a channel [`Sender`]); every
/// clone talks to the same thread and the same connection.
#[derive(Debug, Clone)]
pub struct Writer {
    commands: Sender<Command>,
}

impl Writer {
    /// Opens (or creates) the database at `path`, runs pending migrations, and spawns the
    /// writer thread that owns the connection for the rest of the process.
    ///
    /// `path` may be a real file or `SQLite`'s own in-memory sentinel, the literal string
    /// `":memory:"` (see [`open_connection`]). Migrations run synchronously here, before the
    /// thread starts, so a broken database is reported to the caller as an `Err` rather than
    /// silently killing a detached thread nobody is watching.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let conn = open_connection(path)?;
        migrations::apply(&conn)?;
        // Data seed, not schema: migrations bring the tables into existence exactly once per
        // `user_version`, but `Writer::open` runs against the same persistent on-disk file on
        // every app start, so the bundled OurAirports snapshot needs its own idempotency check
        // (docs/08 says nothing about *data*, only schema).
        ourairports::seed_if_empty(&conn)?;

        let (commands, inbox) = unbounded();
        thread::Builder::new()
            .name("store-writer".to_owned())
            .spawn(move || run(&conn, inbox))
            .map_err(|error| StoreError::Backend {
                message: format!("could not start the store writer thread: {error}"),
            })?;

        Ok(Self { commands })
    }

    /// Records a successful poll cycle: `last_success` and `credits_used_today` for `source`,
    /// upserted — `source` is `source_status`'s primary key, so a later call for the same
    /// source overwrites the row rather than inserting a second one.
    pub fn record_success(
        &self,
        source: SourceId,
        at: UnixSeconds,
        credits_used_today: u32,
    ) -> Result<(), StoreError> {
        self.call(|reply| Command::RecordSuccess {
            source,
            at,
            credits_used_today,
            reply,
        })
    }

    /// Records a failed poll cycle: `last_error`/`last_error_msg` for `source`. Deliberately
    /// leaves `credits_used_today` untouched — a failed cycle spent nothing knowable, and
    /// clobbering the counter here would make a transient failure look like a budget reset.
    pub fn record_error(
        &self,
        source: SourceId,
        at: UnixSeconds,
        message: String,
    ) -> Result<(), StoreError> {
        self.call(|reply| Command::RecordError {
            source,
            at,
            message,
            reply,
        })
    }

    /// Reads `source`'s row back, or `None` if it has never recorded a cycle. The credit
    /// figure here is exactly the `spent` argument `ingest::budget::CreditLedger::restored`
    /// takes.
    pub fn source_status(&self, source: SourceId) -> Result<Option<SourceStatus>, StoreError> {
        self.call(|reply| Command::SourceStatus { source, reply })
    }

    /// Airports within `bbox` at or above `min_size` — same signature as
    /// `core::contracts::Store::airports_in_bbox` (docs/09), backed by migration 0002's
    /// `airports` table, seeded from the bundled `OurAirports` snapshot on first open.
    pub fn airports_in_bbox(
        &self,
        bbox: BBox,
        min_size: AirportSize,
    ) -> Result<Vec<Airport>, StoreError> {
        self.call(|reply| Command::AirportsInBbox {
            bbox,
            min_size,
            reply,
        })
    }

    /// Runways within `bbox`'s airports at or above `min_size` — same signature as
    /// `core::contracts::Store::runways_in_bbox` (docs/09), backed by migration 0002's
    /// `runways` table, seeded from the bundled `OurAirports` snapshot on first open.
    pub fn runways_in_bbox(
        &self,
        bbox: BBox,
        min_size: AirportSize,
    ) -> Result<Vec<Runway>, StoreError> {
        self.call(|reply| Command::RunwaysInBbox {
            bbox,
            min_size,
            reply,
        })
    }

    /// Upserts a batch of METAR observations and prunes each touched station down to its two
    /// most recent (docs/08 retention) — same signature as
    /// `core::contracts::Store::upsert_metars` (M3 item 3.3), backed by migration 0003.
    pub fn upsert_metars(&self, batch: Vec<Metar>) -> Result<(), StoreError> {
        self.call(|reply| Command::UpsertMetars { batch, reply })
    }

    /// The freshest cached METAR for each of `stations` that has one — same signature as
    /// `core::contracts::Store::metars_for_stations`.
    pub fn metars_for_stations(&self, stations: Vec<String>) -> Result<Vec<Metar>, StoreError> {
        self.call(|reply| Command::MetarsForStations { stations, reply })
    }

    /// Builds a [`Command`] around a fresh one-shot reply channel, sends it, and blocks for
    /// the answer.
    ///
    /// Every public method above is this same shape, which is the whole point of one
    /// `Command` enum rather than a method-shaped channel each: a writer thread that is no
    /// longer running (it panicked, or the process is shutting down) surfaces here as one
    /// `StoreError::Backend`, not a different failure per method.
    fn call<T>(
        &self,
        build: impl FnOnce(Sender<Result<T, StoreError>>) -> Command,
    ) -> Result<T, StoreError> {
        let (reply, response) = bounded(1);
        self.commands
            .send(build(reply))
            .map_err(|_send_error| StoreError::Backend {
                message: "store writer thread is no longer running".to_owned(),
            })?;
        response.recv().map_err(|_recv_error| StoreError::Backend {
            message: "store writer thread dropped the reply channel".to_owned(),
        })?
    }
}

/// Opens `path` and applies the docs/08 connection pragmas.
///
/// `journal_mode = WAL` is requested unconditionally; `SQLite` itself falls back to a
/// different mode for `":memory:"` connections (there is no shared file to write a WAL
/// against), so this never asserts the mode actually took — the on-disk smoke test below is
/// where that is real.
fn open_connection<P: AsRef<Path>>(path: P) -> Result<Connection, StoreError> {
    let conn = Connection::open(path).map_err(|error| backend_error(&error))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| backend_error(&error))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| backend_error(&error))?;
    Ok(conn)
}

/// The writer thread's body: drain `inbox` until every [`Writer`] clone (and therefore every
/// `Sender`) has been dropped, at which point iteration ends and the thread exits.
///
/// A reply that fails to send (the caller's `response` half was itself dropped — e.g. a test
/// that stopped waiting) is not this thread's problem: the write already happened or it
/// didn't, and there is no one left to tell.
fn run(conn: &Connection, inbox: Receiver<Command>) {
    for command in inbox {
        match command {
            Command::RecordSuccess {
                source,
                at,
                credits_used_today,
                reply,
            } => {
                let result = record_success(conn, source, at, credits_used_today);
                let _ignored = reply.send(result);
            }
            Command::RecordError {
                source,
                at,
                message,
                reply,
            } => {
                let result = record_error(conn, source, at, &message);
                let _ignored = reply.send(result);
            }
            Command::SourceStatus { source, reply } => {
                let result = read_source_status(conn, source);
                let _ignored = reply.send(result);
            }
            Command::AirportsInBbox {
                bbox,
                min_size,
                reply,
            } => {
                let result = ourairports::airports_in_bbox(conn, bbox, min_size);
                let _ignored = reply.send(result);
            }
            Command::RunwaysInBbox {
                bbox,
                min_size,
                reply,
            } => {
                let result = ourairports::runways_in_bbox(conn, bbox, min_size);
                let _ignored = reply.send(result);
            }
            Command::UpsertMetars { batch, reply } => {
                let result = metar::upsert_metars(conn, &batch);
                let _ignored = reply.send(result);
            }
            Command::MetarsForStations { stations, reply } => {
                let result = metar::metars_for_stations(conn, &stations);
                let _ignored = reply.send(result);
            }
        }
    }
}

/// Upserts `source_status` for a successful cycle. Only the columns a success owns
/// (`last_success`, `credits_used_today`) are written on conflict — an existing
/// `last_error`/`last_error_msg` from a previous failed cycle is left in place, so a success
/// does not erase the record of an earlier problem, only sit next to it.
fn record_success(
    conn: &Connection,
    source: SourceId,
    at: UnixSeconds,
    credits_used_today: u32,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO source_status (source, last_success, credits_used_today)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (source) DO UPDATE SET
             last_success = excluded.last_success,
             credits_used_today = excluded.credits_used_today",
        params![source.as_str(), at.0, credits_used_today],
    )
    .map_err(|error| backend_error(&error))?;
    Ok(())
}

/// Upserts `source_status` for a failed cycle. Only `last_error`/`last_error_msg` are written
/// on conflict — `credits_used_today` is never part of this statement, so an error can never
/// reset or clobber the day's spend.
fn record_error(
    conn: &Connection,
    source: SourceId,
    at: UnixSeconds,
    message: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO source_status (source, last_error, last_error_msg)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (source) DO UPDATE SET
             last_error = excluded.last_error,
             last_error_msg = excluded.last_error_msg",
        params![source.as_str(), at.0, message],
    )
    .map_err(|error| backend_error(&error))?;
    Ok(())
}

/// Reads `source`'s row, or `None` if it has never recorded a cycle.
fn read_source_status(
    conn: &Connection,
    source: SourceId,
) -> Result<Option<SourceStatus>, StoreError> {
    conn.query_row(
        "SELECT last_success, last_error, last_error_msg, credits_used_today
         FROM source_status WHERE source = ?1",
        [source.as_str()],
        |row| {
            Ok(SourceStatus {
                source,
                last_success: row.get::<_, Option<i64>>(0)?.map(UnixSeconds),
                last_error: row.get::<_, Option<i64>>(1)?.map(UnixSeconds),
                last_error_msg: row.get(2)?,
                credits_used_today: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|error| backend_error(&error))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn migrated_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory connection opens");
        migrations::apply(&conn).expect("migrations apply");
        conn
    }

    // ---- Direct function tests: the upsert semantics, without the thread in the way ------

    #[test]
    fn reading_an_unrecorded_source_returns_none() {
        let conn = migrated_conn();
        assert_eq!(
            read_source_status(&conn, SourceId::AdsbLol).expect("reads"),
            None
        );
    }

    #[test]
    fn record_success_round_trips_through_read_source_status() {
        let conn = migrated_conn();
        record_success(&conn, SourceId::OpenSky, UnixSeconds(100), 10).expect("records");

        let status = read_source_status(&conn, SourceId::OpenSky)
            .expect("reads")
            .expect("row exists");
        assert_eq!(status.source, SourceId::OpenSky);
        assert_eq!(status.last_success, Some(UnixSeconds(100)));
        assert_eq!(status.credits_used_today, 10);
        assert_eq!(status.last_error, None);
        assert_eq!(status.last_error_msg, None);
    }

    #[test]
    fn record_error_round_trips_and_never_touches_credits_used_today() {
        let conn = migrated_conn();
        record_success(&conn, SourceId::OpenSky, UnixSeconds(100), 42).expect("success first");
        record_error(&conn, SourceId::OpenSky, UnixSeconds(200), "network blip")
            .expect("then an error");

        let status = read_source_status(&conn, SourceId::OpenSky)
            .expect("reads")
            .expect("row exists");
        assert_eq!(status.last_error, Some(UnixSeconds(200)));
        assert_eq!(status.last_error_msg.as_deref(), Some("network blip"));
        assert_eq!(
            status.credits_used_today, 42,
            "an error write must not reset the day's spend"
        );
    }

    #[test]
    fn a_later_success_does_not_erase_an_earlier_recorded_error() {
        let conn = migrated_conn();
        record_error(&conn, SourceId::OpenSky, UnixSeconds(150), "blip").expect("error first");
        record_success(&conn, SourceId::OpenSky, UnixSeconds(200), 25).expect("then a success");

        let status = read_source_status(&conn, SourceId::OpenSky)
            .expect("reads")
            .expect("row exists");
        assert_eq!(status.last_success, Some(UnixSeconds(200)));
        assert_eq!(status.credits_used_today, 25);
        assert_eq!(
            status.last_error,
            Some(UnixSeconds(150)),
            "success only owns its own columns"
        );
        assert_eq!(status.last_error_msg.as_deref(), Some("blip"));
    }

    #[test]
    fn a_second_success_for_the_same_source_overwrites_rather_than_inserting_a_second_row() {
        let conn = migrated_conn();
        record_success(&conn, SourceId::OpenSky, UnixSeconds(1), 5).expect("first");
        record_success(&conn, SourceId::OpenSky, UnixSeconds(2), 9).expect("second");

        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM source_status WHERE source = ?1",
                [SourceId::OpenSky.as_str()],
                |row| row.get(0),
            )
            .expect("counts rows");
        assert_eq!(
            row_count, 1,
            "source is the primary key: a repeat write must overwrite, not duplicate"
        );

        let status = read_source_status(&conn, SourceId::OpenSky)
            .expect("reads")
            .expect("row exists");
        assert_eq!(status.last_success, Some(UnixSeconds(2)));
        assert_eq!(status.credits_used_today, 9);
    }

    #[test]
    fn different_sources_get_independent_rows() {
        let conn = migrated_conn();
        record_success(&conn, SourceId::OpenSky, UnixSeconds(1), 5).expect("opensky");
        record_success(&conn, SourceId::AdsbLol, UnixSeconds(2), 0).expect("adsblol");

        assert_eq!(
            read_source_status(&conn, SourceId::OpenSky)
                .expect("reads")
                .map(|status| status.credits_used_today),
            Some(5)
        );
        assert_eq!(
            read_source_status(&conn, SourceId::AdsbLol)
                .expect("reads")
                .map(|status| status.credits_used_today),
            Some(0)
        );
    }

    // ---- Writer-level tests: through the channel, over the real thread --------------------

    #[test]
    fn writer_open_runs_migrations_and_is_immediately_usable() {
        let writer = Writer::open(":memory:").expect("writer starts");
        assert_eq!(
            writer
                .source_status(SourceId::OpenSky)
                .expect("reads through the channel"),
            None
        );
    }

    #[test]
    fn writer_records_a_success_cycle_end_to_end() {
        let writer = Writer::open(":memory:").expect("writer starts");
        writer
            .record_success(SourceId::OpenSky, UnixSeconds(1_700_000_000), 42)
            .expect("records");

        let status = writer
            .source_status(SourceId::OpenSky)
            .expect("reads")
            .expect("row exists");
        assert_eq!(status.last_success, Some(UnixSeconds(1_700_000_000)));
        assert_eq!(status.credits_used_today, 42);
    }

    #[test]
    fn writer_records_an_error_cycle_end_to_end() {
        let writer = Writer::open(":memory:").expect("writer starts");
        writer
            .record_error(
                SourceId::AirplanesLive,
                UnixSeconds(5),
                "timed out".to_owned(),
            )
            .expect("records");

        let status = writer
            .source_status(SourceId::AirplanesLive)
            .expect("reads")
            .expect("row exists");
        assert_eq!(status.last_error, Some(UnixSeconds(5)));
        assert_eq!(status.last_error_msg.as_deref(), Some("timed out"));
        assert_eq!(status.credits_used_today, 0);
    }

    #[test]
    fn cloned_handles_share_the_same_writer_thread_and_database() {
        let writer = Writer::open(":memory:").expect("writer starts");
        let clone = writer.clone();
        clone
            .record_success(SourceId::OpenSky, UnixSeconds(9), 3)
            .expect("records via the clone");

        let status = writer
            .source_status(SourceId::OpenSky)
            .expect("reads via the original handle")
            .expect("row exists");
        assert_eq!(status.credits_used_today, 3);
    }

    // ---- Airports (M3 item 3.1): seed-on-open, and the bbox/min_size query ----------------

    #[test]
    fn writer_open_seeds_airports_and_runways_from_the_bundled_snapshot() {
        let writer = Writer::open(":memory:").expect("writer starts");
        // A tiny, permissive bbox covering the whole planet at the lowest tier: if the seed
        // ran, this returns rows; if it didn't, it returns none.
        let whole_world = BBox::new(-90.0, -180.0, 90.0, 180.0).expect("valid bbox");
        let airports = writer
            .airports_in_bbox(whole_world, AirportSize::Heliport)
            .expect("queries");
        assert!(
            !airports.is_empty(),
            "Writer::open should have seeded the bundled OurAirports snapshot"
        );
    }

    #[test]
    fn writer_airports_in_bbox_returns_only_the_requested_region_and_size() {
        let writer = Writer::open(":memory:").expect("writer starts");
        // JFK's coordinates, a real `large_airport` in the bundled snapshot.
        let nyc_area = BBox::new(40.0, -75.0, 41.0, -73.0).expect("valid bbox");
        let large_only = writer
            .airports_in_bbox(nyc_area, AirportSize::Large)
            .expect("queries");
        assert!(
            large_only.iter().any(|airport| airport.ident == "KJFK"),
            "JFK should be among the large airports in the NYC-area bbox"
        );
        assert!(
            large_only
                .iter()
                .all(|airport| airport.size == AirportSize::Large),
            "AirportSize::Large must exclude medium/small/heliport results"
        );

        let elsewhere = BBox::new(-1.0, -1.0, 0.0, 0.0).expect("valid bbox"); // Gulf of Guinea
        let far_away = writer
            .airports_in_bbox(elsewhere, AirportSize::Heliport)
            .expect("queries");
        assert!(
            far_away.iter().all(|airport| airport.ident != "KJFK"),
            "a bbox nowhere near JFK must not return it"
        );
    }

    #[test]
    fn writer_runways_in_bbox_returns_at_least_one_runway_for_a_real_bundled_airport() {
        let writer = Writer::open(":memory:").expect("writer starts");
        // JFK's coordinates, a real `large_airport` in the bundled snapshot with real runways.
        let nyc_area = BBox::new(40.0, -75.0, 41.0, -73.0).expect("valid bbox");
        let runways = writer
            .runways_in_bbox(nyc_area, AirportSize::Large)
            .expect("queries");
        assert!(
            runways.iter().any(|runway| runway.airport_ident == "KJFK"),
            "JFK should have at least one runway in the NYC-area bbox at AirportSize::Large"
        );

        let elsewhere = BBox::new(-1.0, -1.0, 0.0, 0.0).expect("valid bbox"); // Gulf of Guinea
        let far_away = writer
            .runways_in_bbox(elsewhere, AirportSize::Heliport)
            .expect("queries");
        assert!(
            far_away.iter().all(|runway| runway.airport_ident != "KJFK"),
            "a bbox nowhere near JFK must not return any of its runways"
        );
    }

    #[test]
    fn re_opening_a_writer_against_the_same_on_disk_file_does_not_duplicate_seeded_airports() {
        let path = unique_temp_db_path();
        let _cleanup = TempDbFile(path.clone());

        let first_count = {
            let writer = Writer::open(&path).expect("first open seeds");
            let whole_world = BBox::new(-90.0, -180.0, 90.0, 180.0).expect("valid bbox");
            writer
                .airports_in_bbox(whole_world, AirportSize::Heliport)
                .expect("queries")
                .len()
        };
        // The first `Writer` (and its thread) is dropped here; a second `Writer::open` against
        // the same file is exactly the "app restarts" scenario the idempotency check targets.
        let second_count = {
            let writer = Writer::open(&path).expect("second open must not re-seed");
            let whole_world = BBox::new(-90.0, -180.0, 90.0, 180.0).expect("valid bbox");
            writer
                .airports_in_bbox(whole_world, AirportSize::Heliport)
                .expect("queries")
                .len()
        };

        assert_eq!(
            first_count, second_count,
            "re-opening the same on-disk database must not duplicate seeded rows"
        );
    }

    // ---- METARs (M3 item 3.3): upsert through the channel, query back --------------------

    #[test]
    fn writer_upserts_and_queries_metars_end_to_end() {
        use look_above_core::contracts::{FlightCategory, Metar};

        let writer = Writer::open(":memory:").expect("writer starts");
        let observation = Metar {
            station: "KJFK".to_owned(),
            observed_at: UnixSeconds(1_700_000_000),
            raw: "KJFK 010000Z 28012KT 10SM FEW250 05/01 A3024".to_owned(),
            flight_category: Some(FlightCategory::Vfr),
            wind_dir_deg: Some(280),
            wind_kt: Some(12),
            visibility_sm: Some(10.0),
        };
        writer
            .upsert_metars(vec![observation.clone()])
            .expect("upserts");

        let found = writer
            .metars_for_stations(vec!["KJFK".to_owned(), "KLAX".to_owned()])
            .expect("queries");
        assert_eq!(found, vec![observation], "only KJFK has a cached METAR");
    }

    // ---- On-disk smoke test: WAL is real, not just requested ------------------------------

    /// A `Drop` guard that removes a `SQLite` database file and its WAL/SHM/rollback-journal
    /// side files, so a failed assertion still leaves the temp directory clean.
    struct TempDbFile(PathBuf);

    impl Drop for TempDbFile {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let candidate = PathBuf::from(format!("{}{suffix}", self.0.display()));
                let _ignored = std::fs::remove_file(candidate);
            }
        }
    }

    fn unique_temp_db_path() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "look-above-store-wal-smoke-{}-{nanos}.db",
            std::process::id()
        ))
    }

    #[test]
    fn on_disk_connections_actually_use_wal() {
        let path = unique_temp_db_path();
        let _cleanup = TempDbFile(path.clone());

        let conn = open_connection(&path).expect("opens an on-disk database");
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .expect("reads journal_mode");
        assert_eq!(
            mode.to_lowercase(),
            "wal",
            "on-disk databases must use WAL (docs/08); :memory: cannot, so this is the one \
             place it is actually checked"
        );
    }
}
