---
name: authorized-aviation-sources
description: The exhaustive allowlist of aviation data sources for Look Above, with endpoints, auth, rate limits, costs, and attribution. Consult before writing or modifying ANY code that makes an HTTP request, choosing a source for a data need, or debugging API errors (401/429/parse failures).
---

# Authorized Aviation Data Sources

This list is **exhaustive**: code may only contact hosts named here. Adding a source
requires owner approval + a `plans/DECISION_LOG.md` entry confirming its terms permit free
programmatic use (privacy rule 1.1). Everything here is free; only OpenSky needs a signup.

## Which source for which need

| Need | Use | Fallback |
|---|---|---|
| Live positions (bbox) | OpenSky (budgeted) | airplanes.live → adsb.lol |
| Aircraft metadata (hex → type/reg/operator) | adsbdb (on selection, anonymity-gated) | FAA registry table (US, offline) |
| Callsign → route | adsbdb (on selection, anonymity-gated) | — (show "—") |
| METAR/TAF | aviationweather.gov | — (hide badge) |
| Airports/runways/navaids | OurAirports CSV (imported) | — (bundled snapshot) |
| Airline names | openflights data file (imported, optional) | callsign prefix shown raw |
| Base map | Natural Earth (bundled at build time) | — |

## Live positions

### OpenSky Network — PRIMARY (free account required)
- **Signup:** free account at https://opensky-network.org → account settings → create an
  **API client** → client id + secret into gitignored `config.toml`.
- **Auth:** OAuth2 client-credentials. Token endpoint:
  `https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token`
  (form: `grant_type=client_credentials&client_id=...&client_secret=...`). Token TTL ~30 min;
  refresh at 80%. Send as `Authorization: Bearer`. (Basic auth was retired in 2025 — if docs
  or old examples show it, they're stale.)
- **Endpoint:** `GET https://opensky-network.org/api/states/all?lamin=&lomin=&lamax=&lomax=`
- **Budget:** credit system — registered accounts get **4,000 credits/day** (anonymous 400;
  active feeders more). A bbox request costs 1–4 credits by area (small < 25°² = 1 … global = 4).
  Track spend in `source_status.credits_used_today`; stay ≤ 80%. Data resolution ~5–10 s.
- **Response:** positional arrays (`states: [[icao24, callsign, origin_country,
  time_position, last_contact, lon, lat, baro_altitude, on_ground, velocity, true_track,
  vertical_rate, sensors, geo_altitude, squawk, spi, position_source, category], ...]`) —
  every field nullable; note **lon before lat**.
- **Attribution:** credit The OpenSky Network in README + About screen; cite their paper if
  ever publishing anything derived.

### airplanes.live — fallback 1 (no key)
- **Endpoint:** `GET https://api.airplanes.live/v2/point/{lat}/{lon}/{radius_nm}` (radius ≤ 250).
- **Limit:** 1 request/second (enforce ≥ 2 s spacing). Community-run: be gentle, credit them.
- **Response:** readsb JSON `{ ac: [...], now }`; `alt_baro` may be the string `"ground"`;
  fields: `hex, flight, lat, lon, alt_baro, gs, track, baro_rate, squawk, category, seen_pos`.
  **Units: feet / knots / ft-per-min** (convert to SI); position ts = `now − seen_pos` with
  `now` in epoch **ms**; `~`-prefixed `hex` = non-ICAO TIS-B synthetic (skip).

### adsb.lol — fallback 2 (no key)
- **Endpoint:** `GET https://api.adsb.lol/v2/point/{lat}/{lon}/{radius_nm}` — same readsb
  family as airplanes.live; share the parser, keep a separate adapter + fixtures (shapes
  can drift independently). Open-data project; credit them.

## Enrichment

### adsbdb — metadata & routes (no key)
- `GET https://api.adsbdb.com/v0/aircraft/{hex}` / `GET https://api.adsbdb.com/v0/callsign/{callsign}`
- Call **only** from the selection path, **only** when the target's `anonymous == false`
  (privacy rule 2.2 — this gate is code, not convention). LRU cache + 24 h negative cache.
  Never bulk-enumerate.

### aviationweather.gov — METAR/TAF (no key, official NOAA)
- `GET https://aviationweather.gov/api/data/metar?ids={CSV≤100}&format=json` (also `/api/data/taf`).
- Poll visible stations ≥ 10 min apart. METARs are hourly; more polling is waste.

### Static downloads (import scripts, not runtime polling)
- **OurAirports** (public domain): `https://davidmegginson.github.io/ourairports-data/{airports,runways,navaids}.csv`
- **FAA registry** (US): `https://registry.faa.gov/database/ReleasableAircraft.zip` — owner
  names are never displayed (privacy 4.3).
- **openflights airlines** (ODbL): `https://raw.githubusercontent.com/jpatokal/openflights/master/data/airlines.dat`
- **Natural Earth** (public domain): fetched at build/setup time from naturalearthdata.com,
  bundled; never at app runtime.
- All honor ETag/Last-Modified; refresh monthly/quarterly, not per-run.

## Explicitly prohibited (privacy rules 1.2–1.3)

- Scraping or calling private/undocumented APIs of **FlightRadar24, FlightAware,
  ADS-B Exchange**, airline sites, or any source whose terms don't permit free programmatic
  use — including "one debugging request".
- Evading limits: IP/key/User-Agent rotation, parallel accounts, cache-busting.
- Any source added "temporarily" without the decision-log entry.

## Cross-cutting client rules (docs/09)

`User-Agent: look-above/<version> (github.com/arcTanMyAngle/look-above)`; 10 s timeouts;
no retry on 4xx; exponential backoff + jitter on 429/5xx honoring Retry-After (base 5 s,
cap 5 min); every source reports into `source_status`.
