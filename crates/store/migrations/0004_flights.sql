-- Migration 0004: observed flights (M3 item 3.4). Verbatim from docs/08_DATABASE_SCHEMA.md
-- (comments included) — that doc and this file must never drift; a schema change updates both
-- in the same commit. Migrations are append-only: 0001-0003 are untouched. This table was
-- originally planned for M5 (docs/08's own "session-boundary merge" note below is the leftover
-- of that plan); pulled forward to M3 to back on-selection adsbdb route caching
-- (DECISION_LOG 2026-07-21, M3 3.4).

-- Observed flights (callsign sessions), for the info card and replay grouping. Originally
-- tagged M5; pulled forward to M3 item 3.4 to back on-selection adsbdb route caching
-- (DECISION_LOG 2026-07-21, M3 3.4). M3 only ever inserts one row per resolved lookup —
-- the session-boundary merge (extending last_seen, detecting gaps via `positions`) this
-- table's shape implies is still M5's job, once `positions` exists.
CREATE TABLE flights (
    id            INTEGER PRIMARY KEY,
    icao24        TEXT    NOT NULL,
    callsign      TEXT,                      -- NULL when anonymous (rule 2.2)
    origin        TEXT,                      -- ICAO airport, from adsbdb; best-effort
    destination   TEXT,
    first_seen    INTEGER NOT NULL,
    last_seen     INTEGER NOT NULL
);
CREATE INDEX idx_flights_icao_seen ON flights (icao24, last_seen);
