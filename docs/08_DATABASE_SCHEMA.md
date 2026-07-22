# 08 — Database Schema (SQLite via rusqlite)

Single database file `look_above.db` in the platform data dir. WAL mode, `synchronous=NORMAL`.
Migrations are numbered SQL files embedded in `crates/store` (`migrations/000N_*.sql`),
applied in order at startup; `PRAGMA user_version` tracks the applied version. Migrations are
append-only — never edit a shipped migration.

## Tables

```sql
-- Airframe metadata cache (adsbdb / FAA registry / feed-provided). M1/M3.
CREATE TABLE aircraft (
    icao24        TEXT PRIMARY KEY,          -- lower-case hex, e.g. 'a1b2c3'
    registration  TEXT,
    type_code     TEXT,                      -- ICAO type designator, e.g. 'B738'
    category      TEXT,                      -- jet|turboprop|piston|heli|glider|unknown (glyph selection)
    operator      TEXT,                      -- airline/operator name; NULL for private (rule 4.3)
    is_anonymous  INTEGER NOT NULL DEFAULT 0,-- PIA/anonymized: gates all enrichment (rule 2.2)
    fetched_at    INTEGER,                   -- unix s; NULL = never looked up
    lookup_failed_at INTEGER                 -- negative-cache 404s for 24h
);

-- Position history (M5; M1 keeps only an in-memory live table).
CREATE TABLE positions (
    icao24        TEXT    NOT NULL,
    ts            INTEGER NOT NULL,          -- unix s (source timestamp, not receipt)
    lat           REAL    NOT NULL,
    lon           REAL    NOT NULL,
    baro_alt_m    REAL,                      -- NULL = on ground or unknown
    velocity_ms   REAL,
    heading_deg   REAL,
    vert_rate_ms  REAL,
    on_ground     INTEGER NOT NULL DEFAULT 0,
    source        TEXT    NOT NULL,          -- 'opensky'|'airplaneslive'|'adsblol'
    PRIMARY KEY (icao24, ts)
) WITHOUT ROWID;
CREATE INDEX idx_positions_ts ON positions (ts);   -- pruning + replay range scans

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

-- OurAirports import. M3.
CREATE TABLE airports (
    ident         TEXT PRIMARY KEY,          -- ICAO/GPS code, e.g. 'KJFK'
    name          TEXT NOT NULL,
    type          TEXT NOT NULL,             -- large_airport|medium_airport|small_airport|heliport|...
    lat           REAL NOT NULL,
    lon           REAL NOT NULL,
    elevation_ft  INTEGER,
    iso_country   TEXT,
    iata          TEXT
);
CREATE INDEX idx_airports_type ON airports (type);  -- L1 shows large/medium only

CREATE TABLE runways (
    airport_ident TEXT NOT NULL REFERENCES airports(ident),
    le_ident      TEXT, le_lat REAL, le_lon REAL, le_heading_deg REAL,
    he_ident      TEXT, he_lat REAL, he_lon REAL, he_heading_deg REAL,
    length_ft     INTEGER,
    width_ft      INTEGER,
    surface       TEXT,
    closed        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_runways_airport ON runways (airport_ident);

CREATE TABLE airlines (                      -- openflights, optional (M3+)
    icao          TEXT PRIMARY KEY,          -- 3-letter, e.g. 'DAL'
    iata          TEXT,
    name          TEXT NOT NULL
);

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

-- Per-source health for the debug overlay. M1.
CREATE TABLE source_status (
    source        TEXT PRIMARY KEY,
    last_success  INTEGER,
    last_error    INTEGER,
    last_error_msg TEXT,
    credits_used_today INTEGER NOT NULL DEFAULT 0   -- OpenSky budget tracking
);
```

## Spatial queries

No SQLite spatial extension: live-aircraft "what's in the viewport" queries are served by the
**in-memory spatial index** in `crates/core` (see doc 09), not SQL. SQL spatial filtering
(airports in bbox, history in bbox) uses plain `lat BETWEEN ? AND ? AND lon BETWEEN ? AND ?`
which the airports/positions indexes serve adequately at our scale.

## Retention & pruning (privacy rule 5.1)

- `positions`: default 24 h, hard max 7 days. Pruner runs hourly:
  `DELETE FROM positions WHERE ts < ?` in ≤ 10k-row batches to avoid long write locks.
- `flights`: pruned when their last position is pruned.
- `metars`: keep latest 2 per station, delete older on insert.
- Disk cap: if db file > 1 GB, halve the retention window and log a warning.

## Write patterns

- Position inserts are batched per poll cycle in one transaction (a global cycle is ≤ ~10k
  rows — fine for WAL).
- All writes go through a single writer thread owning the connection; readers use separate
  read-only connections (WAL allows concurrent readers).
