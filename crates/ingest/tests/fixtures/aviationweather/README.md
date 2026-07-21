# aviationweather.gov `/api/data/metar` fixtures

Consumed by `ingest::metar`'s tests via `include_str!`.

## Provenance

**`metar_nominal.json` is recorded from the live API** (item 1.10's recorder, extended for
this source in M3 item 3.3) — unlike the readsb fixtures, there is no awkward-case reason to
hand-author the nominal case here, and a real batch response is the strongest evidence the
parser matches the documented shape. Recorded 2026-07-21 against ten major US/int'l stations
(`KJFK,KLAX,KORD,KDEN,KATL,PANC,PHNL,KBOI,KSEA,KMIA`); all ten happened to be VFR at record
time, so `metar.rs`'s own tests do not assert a specific flight category out of this fixture,
only that parsing succeeds and every record has a station and raw text.

`metar_malformed.json` is hand-authored: a non-object, an empty object, and three records each
missing one of the three `NOT NULL` fields (`icaoId`/`obsTime`/`rawOb`), around one good KJFK
record — the parser must skip the first five and keep the one.

## Re-recording

```text
cargo run -p look-above-ingest --bin record-fixture -- aviationweather KJFK,KLAX,KORD,KDEN,KATL,PANC,PHNL,KBOI,KSEA,KMIA metar_nominal
```

Fetches, trims to ≤ 20 records, scrubs, and overwrites `metar_nominal.json` in place without
printing the payload (docs/06). Keyless and free (NOAA); be gentle regardless.

## Shape notes (the traps)

- `fltCat` is absent/`null` on an observation with no computable ceiling/visibility, not just
  one of the four category strings.
- `wdir` is a plain number in every observation seen live, but the documented format also
  allows the string `"VRB"` for variable wind — no single heading to store, so it (and any
  other non-numeric value) maps to `None`, not a guessed degree.
- `visib` is sometimes a plain number (`6`) and sometimes a qualified string (`"10+"`,
  `"1/2"`, `"M1/4"`); the qualifier ("at least" / "less than") has nowhere to live in the
  schema's plain `REAL` column, so it is dropped and only the numeric value is kept.
- `obsTime` is already Unix seconds (unlike airplanes.live's epoch-millisecond `now` — do not
  assume every source's timestamp needs the same conversion).
