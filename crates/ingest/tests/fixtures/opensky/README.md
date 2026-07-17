# OpenSky `/states/all` fixtures

Consumed by `ingest::opensky::states`'s tests via `include_str!`, covering the cases
docs/10 §2 requires of every source: nominal, empty region, nulls in every optional field,
and a malformed record mid-array.

## Provenance

**Hand-written to OpenSky's documented shape** — not recorded from the live API. The awkward
cases are the point, and they are hard to *catch* live: a response with a non-array element
mid-`states`, or an aircraft with every optional field null, arrives when it arrives.
Authoring them is the only way to have them.

The cost of authoring is that these encode what we *believe* OpenSky sends, so a test passing
against them proves the parser matches our belief, not reality. That gap is closed by
`live_opensky_states_match_the_documented_shape` in `states.rs` — an `#[ignore]`d test that
fetches real aircraft and asserts the shape (notably that every one lands inside the
requested bbox, which is what a lon/lat swap fails). Run it after any change here.

## Re-recording

The recorder from item 1.10 can refresh a fixture's *shape* against the live API (it needs
credentials — see the module docs):

```text
cargo run -p look-above-ingest --bin record-fixture -- opensky 46 7 48 9 states_nominal
```

That fetches, trims to ≤ 20 records, credential-scrubs, and **overwrites `states_nominal.json`
in place** without ever printing the payload — if you only meant to inspect the live shape,
record to a scratch name instead, or `git checkout` this file to restore it. It is not a
drop-in: `states_nominal.json` is crafted so the parser
tests assert *exact* values (`DLH9LF` at Frankfurt, the JFK ground record, the anonymous one),
and live data will not match those — a re-record means updating the paired assertions too. The
`empty` / `nulls` / `malformed` cases stay hand-authored; the recorder captures the nominal
sky, not those. Use it to confirm the live shape still parses and to reset a fixture after a
documented source change, not as a routine refresh.

## Privacy

Privacy rule 7.2: fixtures carry no credential material or account metadata. The hex
addresses and callsigns are public data from an authorized feed — real allocations
(`3c6444` is German, `4ca7b6` Irish) so the parse is exercised against plausible values.

## The files

| File | Case |
|---|---|
| `states_nominal.json` | Four aircraft: airborne with every field, on the ground (null altitude), and one with no callsign (privacy 2.2 — anonymous). |
| `states_empty.json` | A quiet region. `states` is `null`, **not** `[]` — OpenSky's real behavior, and a parse error for anyone who assumes an array. |
| `states_nulls.json` | Every optional field null, and a record that stops early (both the 17- and 18-field forms exist). |
| `states_malformed.json` | Six unusable records around two good ones: a string where a record belongs, a non-hex address, a null position, a null `time_position`, coordinates off the globe, and an empty array. The parse must skip each and keep the two. |

## Field order

The record is positional, and **longitude precedes latitude** (index 5, then 6):

```
[icao24, callsign, origin_country, time_position, last_contact, lon, lat,
 baro_altitude, on_ground, velocity, true_track, vertical_rate, sensors,
 geo_altitude, squawk, spi, position_source, category]
```
