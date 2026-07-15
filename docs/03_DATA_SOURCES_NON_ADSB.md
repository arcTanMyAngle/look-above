# 03 — Data Sources: Non-ADS-B (Enrichment)

Live aircraft *positions* (ADS-B-derived state vectors) are covered in
[../plans/M1_AUTHORIZED_DATA_INGESTION.md](../plans/M1_AUTHORIZED_DATA_INGESTION.md) and the
[authorized-aviation-sources skill](../.claude/skills/authorized-aviation-sources/SKILL.md).
This doc covers everything *around* those positions: airports, weather, routes, aircraft
metadata. All sources below are **free and require no signup**.

## Airports, runways, navaids — OurAirports

- **What:** ~80k airports worldwide, runways with endpoints/headings, navaids. Public domain.
- **Where:** static CSVs, refreshed daily upstream:
  - `https://davidmegginson.github.io/ourairports-data/airports.csv`
  - `https://davidmegginson.github.io/ourairports-data/runways.csv`
  - `https://davidmegginson.github.io/ourairports-data/navaids.csv`
- **Cadence:** download at first run + manual/monthly refresh; import into SQLite
  (`airports`, `runways` tables — see [08_DATABASE_SCHEMA.md](08_DATABASE_SCHEMA.md)).
- **Used for:** airport dots (L1), runway outlines (L2), region search, METAR station lookup.

## Aviation weather — NOAA Aviation Weather Center Data API

- **What:** METARs, TAFs, PIREPs. Official US government source, global METAR coverage, no key.
- **Where:** `https://aviationweather.gov/api/data/metar?ids={ICAO}&format=json`
  (also `/api/data/taf`). Batch up to ~100 stations per call with comma-separated ids.
- **Cadence:** poll visible-region stations every 10 min (METARs update hourly; this is
  already generous). Cache in `metars` table with `observed_at`.
- **Used for:** METAR badge on airports in regional mode (wind, visibility, flight category
  VFR/MVFR/IFR/LIFR color dot).

## Callsign → route, hex → aircraft — adsbdb

- **What:** community API mapping callsigns to origin/destination and ICAO 24-bit hex to
  airframe (type, registration, operator). Free, no key.
- **Where:** `https://api.adsbdb.com/v0/callsign/{callsign}`,
  `https://api.adsbdb.com/v0/aircraft/{hex}`.
- **Cadence:** on-demand when the user selects an aircraft; LRU-cache responses in the
  `aircraft` table. Never bulk-enumerate. Treat 404 as "unknown" and cache that too (24 h)
  to avoid re-asking.
- **Used for:** selection info card (route, type, operator). **Privacy rule:** if the live
  feed anonymizes an aircraft, do not use adsbdb to re-identify it — see
  [04_PRIVACY_AND_SAFETY_RULES.md](04_PRIVACY_AND_SAFETY_RULES.md).

## US aircraft registry — FAA Releasable Database

- **What:** official US registration database (N-numbers → type, owner class). Free download.
- **Where:** `https://registry.faa.gov/database/ReleasableAircraft.zip` (CSV inside).
- **Cadence:** optional import at M3; refresh quarterly. US-only; non-US airframes rely on adsbdb.
- **Used for:** offline type lookup fallback. Owner *names* are ingested but never displayed —
  only operator/airline and type (privacy rule 4.3).

## Airlines & routes reference — openflights (optional, M3+)

- **What:** airline names/IATA/ICAO codes; public data files (Open Database License).
- **Where:** `https://raw.githubusercontent.com/jpatokal/openflights/master/data/airlines.dat`.
- **Used for:** mapping callsign prefixes (e.g. `DAL` → Delta) for labels. Low priority;
  adsbdb usually suffices.

## Base map geometry — Natural Earth

- **What:** public-domain coastlines, land polygons, country borders (1:10m / 1:50m / 1:110m).
- **Where:** `https://www.naturalearthdata.com/downloads/` (bundle simplified GeoJSON with the
  app; no runtime fetch).
- **Used for:** the map itself. Vector data is tessellated once at startup (CPU, `lyon`),
  cached as vertex buffers per LOD tier.

## Refresh & failure policy (all enrichment sources)

- Enrichment is always **best-effort**: the tracker must run fully with zero enrichment
  (positions only). Any enrichment fetch failure degrades silently to cached/absent data.
- All downloads honor `ETag`/`Last-Modified` where offered; never re-download unchanged files.
- Every source gets a `source_status` row (last success, last error) surfaced in a debug overlay.
