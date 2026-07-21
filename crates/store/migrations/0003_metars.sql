-- Migration 0003: METAR cache (M3 item 3.3). Verbatim from docs/08_DATABASE_SCHEMA.md — that
-- doc and this file must never drift; a schema change updates both in the same commit.
-- Migrations are append-only: 0001/0002 are untouched.

-- METAR cache. M3.
CREATE TABLE metars (
    station       TEXT NOT NULL,
    observed_at   INTEGER NOT NULL,
    raw           TEXT NOT NULL,
    flight_cat    TEXT,                      -- VFR|MVFR|IFR|LIFR
    wind_dir_deg  INTEGER,
    wind_kt       INTEGER,
    visibility_sm REAL,
    PRIMARY KEY (station, observed_at)
) WITHOUT ROWID;
-- No secondary index: `WITHOUT ROWID`'s clustered primary key is already ordered by
-- (station, observed_at), which is exactly the "latest observation(s) for a station" access
-- pattern `metar::metars_for_stations`/retention need — SQLite can walk it in either direction.
