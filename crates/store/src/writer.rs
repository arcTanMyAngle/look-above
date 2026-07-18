//! The single writer thread: the one place that owns the write connection (docs/08: "all
//! writes go through a single writer thread owning the connection; readers use separate
//! read-only connections").
//!
//! [`Writer`] is the cheap-to-clone handle every caller holds; it never touches `SQLite`
//! itself, only a `crossbeam` command channel the dedicated thread drains. Migration 0001
//! backs exactly one capability at this item — recording a poll cycle's outcome against
//! `source_status`, and reading it back — which is also the other half of 1.7's seam: the
//! `credits_used_today` [`Writer::source_status`] hands back is the raw value
//! `ingest::budget::CreditLedger::restored` rehydrates from. That call itself happens in
//! `ingest`/`app` wiring (a later item); this crate only stores and returns the counter, with
//! no notion of UTC-day rollover — `restored` already tolerates a stale persisted value.
//!
//! The command set is one enum behind one channel, not one channel per operation, on purpose:
//! a later item can add a variant for `positions`/`airports` (once those tables exist) without
//! changing [`Writer`]'s public shape.

use std::path::Path;
use std::thread;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use look_above_core::error::StoreError;
use look_above_core::types::{SourceId, UnixSeconds};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::backend_error;
use crate::migrations;

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
