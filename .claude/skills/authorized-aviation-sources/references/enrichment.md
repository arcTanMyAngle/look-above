# Enrichment

## adsbdb — metadata & routes (no key)
- `GET https://api.adsbdb.com/v0/aircraft/{hex}` / `GET https://api.adsbdb.com/v0/callsign/{callsign}`
- Call **only** from the selection path, **only** when the target's `anonymous == false`
  (privacy rule 2.2 — this gate is code, not convention). LRU cache + 24 h negative cache.
  Never bulk-enumerate.

## aviationweather.gov — METAR/TAF (no key, official NOAA)
- `GET https://aviationweather.gov/api/data/metar?ids={CSV≤100}&format=json` (also `/api/data/taf`).
- Poll visible stations ≥ 10 min apart. METARs are hourly; more polling is waste.
