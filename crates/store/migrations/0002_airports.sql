-- Migration 0002: airports + runways (OurAirports import, M3 item 3.1). Verbatim from
-- docs/08_DATABASE_SCHEMA.md (comments included) — that doc and this file must never drift; a
-- schema change updates both in the same commit. Migrations are append-only: 0001 is untouched.

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
