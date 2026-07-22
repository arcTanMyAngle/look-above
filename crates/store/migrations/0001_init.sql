-- Migration 0001: the two tables M1 item 1.11 needs. Verbatim from docs/08_DATABASE_SCHEMA.md
-- (comments included) — that doc and this file must never drift; a schema change updates both
-- in the same commit. The rest of docs/08's eventual schema (positions, flights, airports,
-- runways, airlines, metars) lands in later numbered migrations at the milestones that need
-- them (M3 for flights/airports/runways/metars, M5 for positions) — migrations are append-only,
-- so nothing here is created ahead of its milestone.

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

-- Per-source health for the debug overlay. M1.
CREATE TABLE source_status (
    source        TEXT PRIMARY KEY,
    last_success  INTEGER,
    last_error    INTEGER,
    last_error_msg TEXT,
    credits_used_today INTEGER NOT NULL DEFAULT 0   -- OpenSky budget tracking
);
