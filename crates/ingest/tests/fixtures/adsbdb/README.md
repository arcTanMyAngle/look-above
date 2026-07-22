# `api.adsbdb.com` fixtures

Consumed by `ingest::adsbdb`'s tests via `include_str!`.

## Provenance

**`aircraft_nominal.json` and `callsign_nominal.json` are recorded from the live API**
(item 1.10's recorder, extended for this source in M3 item 3.4) — a real response is the
strongest evidence the parser matches the documented shape. Recorded 2026-07-21:

- `aircraft_nominal.json`: `GET /v0/aircraft/a4b213` — a real, publicly registered general
  aviation aircraft (a Cirrus SR22). Holds its ICAO type designator and FAA registration/owner
  fields; all FAA registry data is already public record, the same reasoning
  `aviationweather.gov`'s fixtures rely on for station data.
- `callsign_nominal.json`: `GET /v0/callsign/UAL123` — a real scheduled United Airlines
  flight number, with a route (origin/destination airport) on file.

`aircraft_malformed.json` and `callsign_malformed.json` are hand-authored: each is a JSON array
of four *independent* standalone response bodies (not a batch — adsbdb only ever answers with
one object per request, so the array here is a test fixture convention, not something adsbdb
itself sends) — an empty object, a `response` with no inner key, an explicit `null` for the
inner key, and one well-formed record that deliberately omits an optional field to prove
partial data still parses. The parser tests (`parse_aircraft_response`/`parse_route_response`
in `ingest::adsbdb`) run each array element through the pure parse function independently and
assert only the fourth survives.

## Re-recording

```text
cargo run -p look-above-ingest --bin record-fixture -- adsbdb aircraft a4b213 aircraft_nominal
cargo run -p look-above-ingest --bin record-fixture -- adsbdb callsign UAL123 callsign_nominal
```

Fetches, scrubs, and overwrites the two `*_nominal.json` files in place without printing the
payload (docs/06). Keyless and free; be gentle regardless. The two `*_malformed.json` files are
never re-recorded — they are synthetic by design.

## Shape notes (the traps)

- The whole response is a single JSON object, not an array — unlike every other source this
  crate's fixtures/recorder handle. `record_fixture.rs`'s `records_key()` returns `None` for
  both adsbdb variants and the printed "record count" is computed separately from the shared
  array-truncation logic (see that file's own doc comments).
- adsbdb's real "not found" behavior (an unregistered hex, `000000`) is an HTTP **404** with no
  useful body — `ingest::adsbdb::send_json` maps that to `SourceError::Request { status: 404 }`
  *before* any JSON is decoded, so the adapter never needs to know what a 404 body looks like.
  There is deliberately no `*_not_found.json` fixture: it would test nothing that isn't already
  covered by mocking a bare 404 status.
- `response.aircraft.icao_type` is the short ICAO type designator (`SR22`) `AircraftMeta`'s own
  doc comment wants; the sibling `type` field is a longer, non-ICAO description and is not
  read. Do not conflate the two if the parser is ever revisited.
- `response.flightroute.origin`/`.destination` each carry a full airport record (IATA code,
  name, municipality, coordinates, elevation); only `icao_code` is kept — `Flight`'s schema has
  nowhere else to put the rest.
- `response.flightroute.airline` (callsign, IATA/ICAO carrier code, country) is present live but
  unused: it names the *operator*, which is a separate concern from a route and has no field on
  `Flight` to land in.
