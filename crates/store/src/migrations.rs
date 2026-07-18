//! Numbered, embedded SQL migrations, and the runner that brings a connection up to date.
//!
//! Migrations are `include_str!`-embedded rather than read from disk at runtime (docs/08:
//! "numbered SQL files embedded in `crates/store`"), so the compiled binary is self-contained
//! — no `migrations/` directory needs to ship alongside it. Progress is tracked in `SQLite`'s
//! own `PRAGMA user_version`; each migration commits its DDL and its version bump in one
//! transaction, so a crash mid-migration cannot leave the version ahead of the schema it
//! claims. Migrations are append-only ("never edit a shipped migration" — docs/08), so the
//! only change this file should ever need is another entry appended to [`MIGRATIONS`].

use look_above_core::error::StoreError;
use rusqlite::Connection;

use crate::error::backend_error;

/// One numbered migration: the `user_version` it brings the database to, and the SQL that
/// gets it there.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// Every migration, oldest first.
///
/// Migration 0001 creates only `aircraft` and `source_status` — the pair item 1.11's writer
/// thread needs. The rest of docs/08's eventual schema (`positions`, `flights`, `airports`,
/// `runways`, `airlines`, `metars`) is tagged there with its own milestone (M3/M5) and lands
/// as its own numbered migration when that milestone needs it, rather than being created
/// ahead of time with nothing to do.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("../migrations/0001_init.sql"),
}];

/// Brings `conn` from whatever `user_version` it is already at up to [`MIGRATIONS`]'s latest.
///
/// A migration whose version is `<=` the connection's current `user_version` is skipped —
/// which is what makes a second call against an already-migrated database a no-op (docs/10
/// §3: "idempotent-by-version"), and what makes a fresh (`user_version = 0`) database walk
/// every entry from the start.
pub fn apply(conn: &Connection) -> Result<(), StoreError> {
    let current = user_version(conn)?;
    for migration in MIGRATIONS {
        if migration.version <= current {
            continue;
        }
        apply_one(conn, migration)?;
    }
    Ok(())
}

/// Runs one migration's DDL and bumps `user_version` to it, both inside a single transaction.
///
/// `BEGIN IMMEDIATE` claims the write lock up front rather than on the first write inside the
/// batch, so a concurrent reader can never observe a half-applied migration. Interpolating
/// `migration.version` into the SQL string (rather than binding it) is safe here because it
/// comes from the `const` [`MIGRATIONS`] table above, never from external input — and `PRAGMA`
/// statements do not accept bound parameters in `SQLite` regardless.
fn apply_one(conn: &Connection, migration: &Migration) -> Result<(), StoreError> {
    let script = format!(
        "BEGIN IMMEDIATE;\n{sql}\nPRAGMA user_version = {version};\nCOMMIT;",
        sql = migration.sql,
        version = migration.version,
    );
    conn.execute_batch(&script)
        .map_err(|error| StoreError::Migration {
            version: migration.version,
            message: error.to_string(),
        })
}

/// The schema version `conn` has already reached.
fn user_version(conn: &Connection) -> Result<u32, StoreError> {
    conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
        .map_err(|error| backend_error(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_conn() -> Connection {
        Connection::open_in_memory().expect("in-memory connection opens")
    }

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, bool>(0),
        )
        .expect("sqlite_master query succeeds")
    }

    fn table_count(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("sqlite_master query succeeds")
    }

    #[test]
    fn a_fresh_database_starts_at_version_zero() {
        let conn = memory_conn();
        assert_eq!(user_version(&conn).expect("reads user_version"), 0);
    }

    #[test]
    fn applying_migrations_advances_user_version_to_the_latest() {
        let conn = memory_conn();
        apply(&conn).expect("migrations apply");
        let latest = MIGRATIONS.last().expect("at least one migration").version;
        assert_eq!(user_version(&conn).expect("reads user_version"), latest);
    }

    #[test]
    fn applying_migrations_creates_exactly_the_tables_migration_0001_owns() {
        let conn = memory_conn();
        apply(&conn).expect("migrations apply");
        assert!(table_exists(&conn, "aircraft"), "aircraft was not created");
        assert!(
            table_exists(&conn, "source_status"),
            "source_status was not created"
        );
        // No other table (positions/airports/...) is created ahead of its milestone.
        assert_eq!(table_count(&conn), 2);
    }

    #[test]
    fn re_applying_against_an_already_migrated_database_is_a_no_op() {
        let conn = memory_conn();
        apply(&conn).expect("first apply succeeds");
        let after_first = user_version(&conn).expect("reads user_version");

        // A second call must not re-run migration 0001's `CREATE TABLE` (which would error
        // against an existing table) and must leave the version exactly where it was.
        apply(&conn).expect("second apply is a no-op, not an error");
        assert_eq!(
            user_version(&conn).expect("reads user_version"),
            after_first
        );
        assert_eq!(table_count(&conn), 2, "tables were not re-created");
    }

    #[test]
    fn a_connection_already_marked_up_to_date_has_nothing_re_run() {
        let conn = memory_conn();
        // Simulate a connection whose `user_version` already claims the latest migration,
        // without the tables actually existing — `apply` must trust `user_version` and skip
        // migration 0001 entirely rather than re-running it.
        conn.pragma_update(None, "user_version", 1u32)
            .expect("sets user_version");
        apply(&conn).expect("apply with nothing pending succeeds");
        assert_eq!(table_count(&conn), 0, "nothing should have run");
    }
}
