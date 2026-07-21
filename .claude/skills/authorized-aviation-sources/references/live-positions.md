# Live positions

## OpenSky Network — PRIMARY (free account required)
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

## airplanes.live — fallback 1 (no key)
- **Endpoint:** `GET https://api.airplanes.live/v2/point/{lat}/{lon}/{radius_nm}` (radius ≤ 250).
- **Limit:** 1 request/second (enforce ≥ 2 s spacing). Community-run: be gentle, credit them.
- **Response:** readsb JSON `{ ac: [...], now }`; `alt_baro` may be the string `"ground"`;
  fields: `hex, flight, lat, lon, alt_baro, gs, track, baro_rate, squawk, category, seen_pos`.
  **Units: feet / knots / ft-per-min** (convert to SI); position ts = `now − seen_pos` with
  `now` in epoch **ms**; `~`-prefixed `hex` = non-ICAO TIS-B synthetic (skip).

## adsb.lol — fallback 2 (no key)
- **Endpoint:** `GET https://api.adsb.lol/v2/point/{lat}/{lon}/{radius_nm}` — same readsb
  family as airplanes.live; share the parser, keep a separate adapter + fixtures (shapes
  can drift independently). Open-data project; credit them.
