# airplanes.live `/v2/point` fixtures (readsb JSON)

Consumed by `ingest::readsb`'s and `ingest::airplanes_live`'s tests via `include_str!`,
covering the cases docs/10 §2 requires of every source: nominal, empty region, nulls in
every optional field, the `"ground"` altitude string, and a malformed record mid-array.

## Provenance

**Hand-written to readsb's documented shape** — not recorded from the live API, for the
same two reasons as `../opensky/README.md`: the recording script (item 1.10) does not
exist yet, and the awkward cases are the point — a TIS-B record or an all-null aircraft
arrives when it arrives; authoring them is the only way to have them.

These therefore encode what we *believe* airplanes.live sends. The beliefs at risk — that
`now` is **milliseconds**, that `alt_baro`/`gs`/`baro_rate` are **feet/knots/ft-per-min**,
and the field names themselves — are asserted against the real service by
`live_airplanes_live_point_matches_the_documented_shape` in `airplanes_live.rs`
(`#[ignore]`d; keyless and free, but run it once, not in a loop). Run it after any change
here. Re-record these once item 1.10 lands.

## Privacy

Privacy rule 7.2: no credential material or account metadata (the source needs neither).
Hex addresses are plausible public allocations (`3c6444` German, `4ca7b6` Irish, `a1b2c3`
US); the `~2e6a4b` record is readsb's marker for a non-ICAO TIS-B/ADS-R synthetic target,
which the parser must skip rather than mint an identity for.

## The files

| File | Case |
|---|---|
| `point_nominal.json` | Four records, three aircraft: airborne with every field (36,000 ft / 450 kt — SI conversions land on exact values), on the ground (`alt_baro: "ground"`, no altitude), anonymous (no `flight` key), and a `~`-prefixed TIS-B synthetic that must be dropped. |
| `point_empty.json` | A quiet region: `ac` is `[]`. (`ac: null` and a missing `ac` are unit-tested inline — all three mean an empty sky.) |
| `point_nulls.json` | Every optional field `null`, and a second record with the optional keys absent entirely; both must still yield positions. |
| `point_malformed.json` | Eleven unusable records around two good ones: non-objects, an empty object, a non-hex address, a `~` hex, a latitude off the globe, missing lat/lon, and a missing `seen_pos`. The parse must skip each and keep the two. |

## Shape notes (the traps)

- **Units are aviation units:** `alt_baro` in feet (or the string `"ground"`), `gs` in
  knots, `baro_rate` in feet/minute. The parser converts to SI; a missed conversion
  produces plausible-looking numbers in the wrong unit.
- **`now` is epoch milliseconds** in the API responses (readsb's own `aircraft.json`
  uses seconds; the parser normalizes by magnitude). A record's position is dated
  `now − seen_pos`, never receipt time.
- Fields are named, not positional — `lat` before `lon`, unlike OpenSky.
