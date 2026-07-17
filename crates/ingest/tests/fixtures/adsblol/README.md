# adsb.lol `/v2/point` fixtures (readsb JSON)

Consumed by `ingest::adsb_lol`'s tests via `include_str!`. adsb.lol speaks the same readsb
`{ ac: [...], now }` family as airplanes.live, so the *parser* is shared
(`ingest::readsb`) and is exercised in depth there; these fixtures are adsb.lol's **own
recorded shape**, kept separate because the two services can drift independently — a field
one starts or stops sending is caught only by a fixture set (and a live test) that belong to
that service alone (docs/09, item 1.6).

## Provenance

**Hand-written to readsb's documented shape** — not recorded from the live API, for the same
reason as `../airplaneslive/README.md` and `../opensky/README.md`: the awkward cases are the
point.

These encode what we *believe* adsb.lol sends. The beliefs at risk — that `now` is
**milliseconds**, that `alt_baro`/`gs`/`baro_rate` are **feet/knots/ft-per-min**, and the
field names themselves — are asserted against the real service by
`live_adsb_lol_point_matches_the_documented_shape` in `adsb_lol.rs` (`#[ignore]`d; keyless
and free, but run it once, not in a loop). Run it after any change here.

## Re-recording

The recorder from item 1.10 refreshes a fixture's *shape* from the live API (keyless, free):

```text
cargo run -p look-above-ingest --bin record-fixture -- adsblol 47 8 73 point_nominal
```

Same caveat as the other sources: it fetches, trims to ≤ 20 records, scrubs, and overwrites
without printing the payload, but `point_nominal.json` is crafted so the parser tests assert
*exact* values, which live data will not match — a re-record means updating those assertions,
and the `empty` / `nulls` / `malformed` cases stay hand-authored. Its identities are kept
deliberately distinct from airplanes.live's; a fresh recording will not preserve that, so
prefer editing over re-recording unless the source's shape actually changed.

## Privacy

Privacy rule 7.2: no credential material or account metadata (the source needs neither).
Hex addresses are plausible public allocations (`4b1a1a`/`4b2b2b`/`4b2c2c` Swiss, `a2b3c4`
US); the `~3f7c1d` record is readsb's marker for a non-ICAO TIS-B/ADS-R synthetic target,
which the parser must skip rather than mint an identity for. The identities are deliberately
distinct from the airplanes.live fixtures so a test can never pass by reading the wrong file.

## The files

| File | Case |
|---|---|
| `point_nominal.json` | Four records, three aircraft: airborne with every field (36,000 ft / 450 kt — SI conversions land on exact values), a US target on the ground (`alt_baro: "ground"`, filtered out of the Swiss test box), anonymous (no `flight` key), and a `~`-prefixed TIS-B synthetic that must be dropped. |
| `point_empty.json` | A quiet region: `ac` is `[]`. |
| `point_nulls.json` | Every optional field `null`, and a second record with the optional keys absent entirely; both must still yield positions. |
| `point_malformed.json` | Eleven unusable records around two good ones: non-objects, an empty object, a non-hex address, a `~` hex, a latitude off the globe, missing lat/lon, and a missing `seen_pos`. The parse must skip each and keep the two. |

## Shape notes (the traps)

Identical to airplanes.live's — see `../airplaneslive/README.md`: aviation units (feet /
knots / ft-per-min, or the string `"ground"`), `now` in epoch **milliseconds**, position ts
= `now − seen_pos`, named fields with `lat` before `lon`.
