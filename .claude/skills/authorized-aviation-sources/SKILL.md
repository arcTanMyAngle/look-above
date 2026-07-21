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

## Per-source details (read only what you're touching)

Endpoints, auth, budgets, response shapes, and attribution live in `references/`.
Read **only** the file for the source you are working on:

- `references/live-positions.md` — OpenSky (OAuth2, credit budget, response layout), airplanes.live, adsb.lol
- `references/enrichment.md` — adsbdb (anonymity gate, caching), aviationweather.gov METAR/TAF
- `references/static-imports.md` — OurAirports, FAA registry, openflights, Natural Earth

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
