//! The readsb `{ "ac": [...] }` response family — one parser for both keyless fallbacks.
//!
//! airplanes.live and adsb.lol both rebroadcast readsb's JSON, so docs/09 mandates a
//! shared parsing module with separate adapters: the *shape* is one thing, but the two
//! services drift independently, so each adapter keeps its own endpoint, fixtures, and
//! [`SourceId`] stamp and hands the body here. Three things about the shape set every
//! trap in this module:
//!
//! **The units are aviation units, not SI.** `alt_baro` is **feet**, `gs` is **knots**,
//! `baro_rate` is **feet per minute** — where `OpenSky` sent metres and metres per second.
//! [`StateVector`] is SI, so every one of those converts here, through named constants,
//! because a missed conversion compiles and produces plausible-looking numbers in the
//! wrong unit (36,000 "metres" is a real-looking altitude; it is just in space).
//!
//! **`alt_baro` may be the *string* `"ground"`** rather than a number — that is how
//! readsb reports a surface target, and it maps to `on_ground = true` with no altitude.
//!
//! **A record's position is dated by `now − seen_pos`**, not by receipt: `seen_pos` is
//! how many seconds ago the position was last updated, and the top-level `now` is the
//! server's clock. Same reasoning as 1.4's `time_position`-not-`last_contact` — dating a
//! stale fix to now would have M2's dead reckoning advance an aircraft from a place it
//! had already left. `now` arrives in **milliseconds from the APIs** but *seconds* in
//! readsb's own `aircraft.json`, so it is normalized by magnitude before use.
//!
//! Parsing is per-field tolerant and per-record fallible, exactly as in
//! `opensky::states`: a record we cannot use is skipped and counted, never a reason to
//! fail the batch, and the required facts are `hex`, `lat`, `lon`, and `seen_pos` — an
//! identity, a position, and a time of applicability. readsb also lists non-ICAO
//! (TIS-B / ADS-R) targets under a `~`-prefixed hex; [`Icao24::from_hex`] rejects those
//! by design (item 0.3), so they are skipped here rather than tracked under a minted
//! identity.

use look_above_core::types::{CallSign, Icao24, SourceId, StateVector, UnixSeconds};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::normalize::{coordinate, narrow};

/// The international foot, exactly.
pub(crate) const METRES_PER_FOOT: f64 = 0.3048;

/// One nautical mile, exactly — also the unit of the fallbacks' radius parameter.
pub(crate) const METRES_PER_NAUTICAL_MILE: f64 = 1852.0;

/// One knot is one nautical mile per hour.
pub(crate) const METRES_PER_SECOND_PER_KNOT: f64 = METRES_PER_NAUTICAL_MILE / 3600.0;

/// `baro_rate` arrives in feet per minute.
pub(crate) const METRES_PER_SECOND_PER_FOOT_PER_MINUTE: f64 = METRES_PER_FOOT / 60.0;

/// Above this, an epoch value can only be milliseconds: 10¹¹ seconds is the year 5138,
/// while 10¹¹ ms is 1973. readsb's own `aircraft.json` carries `now` in seconds; the API
/// wrappers around it (airplanes.live, adsb.lol) send milliseconds. Deciding by magnitude
/// serves both without a per-source flag, and the live test pins which one actually
/// arrives.
const MILLISECONDS_THRESHOLD: f64 = 1.0e11;

/// A readsb point-query reply: `{ ac: [...], now: ..., msg, total, ... }`.
///
/// Only `ac` and `now` are read; the rest is bookkeeping. `ac` is `Option` so an absent
/// or `null` list reads as an empty sky, and its elements stay [`Value`] so one non-object
/// record cannot fail the whole batch (docs/10 §2). `now` is `Option` too: without it no
/// record can be timestamped, which surfaces as every record skipped — the loud warn —
/// rather than as a parse error that docs/09 says must never kill the poller.
#[derive(Debug, Deserialize)]
pub struct PointResponse {
    #[serde(default)]
    now: Option<f64>,
    #[serde(default)]
    ac: Option<Vec<Value>>,
}

impl PointResponse {
    /// Every usable record, stamped as coming from `source`.
    pub fn into_state_vectors(self, source: SourceId) -> Vec<StateVector> {
        let records = self.ac.unwrap_or_default();
        // No usable `now` means no record has a time of applicability, so none parses —
        // which is the warn below, not a panic and not a batch dated to receipt time.
        let vectors: Vec<StateVector> = self
            .now
            .map(epoch_seconds)
            .filter(|now_s| now_s.is_finite())
            .map_or_else(Vec::new, |now_s| {
                records
                    .iter()
                    .filter_map(|record| state_vector(record, now_s, source))
                    .collect()
            });
        let skipped = records.len() - vectors.len();

        if skipped > 0 {
            if vectors.is_empty() {
                // Losing every record is what a changed field name or a moved `now`
                // would look like, and an empty sky does not explain itself.
                tracing::warn!(
                    %source,
                    skipped,
                    "every readsb record was unusable — the response shape may have changed"
                );
            } else {
                // Routine: targets heard but not yet positioned, and `~`-prefixed
                // TIS-B/ADS-R synthetics.
                tracing::debug!(
                    %source,
                    kept = vectors.len(),
                    skipped,
                    "skipped readsb records with no usable identity or position"
                );
            }
        }
        vectors
    }
}

/// Normalizes a readsb `now` to seconds — see [`MILLISECONDS_THRESHOLD`].
fn epoch_seconds(now: f64) -> f64 {
    if now > MILLISECONDS_THRESHOLD {
        now / 1000.0
    } else {
        now
    }
}

/// One `ac` record → one [`StateVector`], or `None` if it cannot be trusted.
///
/// `None` covers everything from "not even an object" to "latitude 91" to "a `~` hex":
/// the caller counts and logs, and the batch carries on.
fn state_vector(record: &Value, now_s: f64, source: SourceId) -> Option<StateVector> {
    let record = record.as_object()?;

    let icao24 = Icao24::from_hex(str_field(record, "hex")?).ok()?;

    // Named fields, and lat before lon — the opposite habit from OpenSky's positional
    // arrays, and the reason this parser cannot swap them silently.
    let lat_deg = coordinate(f64_field(record, "lat")?, 90.0)?;
    let lon_deg = coordinate(f64_field(record, "lon")?, 180.0)?;

    let ts = UnixSeconds(whole_seconds(now_s - f64_field(record, "seen_pos")?)?);

    let callsign = str_field(record, "flight").and_then(CallSign::new);

    // The one polymorphic field: a number is an altitude in feet, the string "ground" is
    // a surface report with no altitude, and anything else reads as absent-and-airborne
    // (the assumption that loses least — it costs a glyph, where skipping would cost the
    // aircraft).
    let (baro_alt_m, on_ground) = match field(record, "alt_baro") {
        Some(value) if value.as_str() == Some("ground") => (None, true),
        Some(value) => (
            value.as_f64().map(|feet| narrow(feet * METRES_PER_FOOT)),
            false,
        ),
        None => (None, false),
    };

    Some(StateVector {
        icao24,
        // Privacy rule 2.2, same as 1.4: a position with no identity is flagged so
        // enrichment never looks it up. And the same known limit, recorded in
        // DECISION_LOG 1.4: a PIA hex that *does* broadcast a callsign is not detected
        // here — that needs FAA range data, and M3's enrichment gate is where it binds.
        anonymous: callsign.is_none(),
        callsign,
        ts,
        lat_deg,
        lon_deg,
        baro_alt_m,
        velocity_ms: f64_field(record, "gs")
            .map(|knots| narrow(knots * METRES_PER_SECOND_PER_KNOT)),
        heading_deg: f64_field(record, "track").map(narrow),
        vert_rate_ms: f64_field(record, "baro_rate")
            .map(|fpm| narrow(fpm * METRES_PER_SECOND_PER_FOOT_PER_MINUTE)),
        on_ground,
        source,
    })
}

/// Floors a fractional epoch to whole seconds, or `None` if it is not a real time.
///
/// Floor, not round: a position's time of applicability rounded *up* would date it after
/// the fix, which is the direction dead reckoning cannot forgive. The range guard keeps a
/// garbage `now`/`seen_pos` pair from becoming a confidently absurd timestamp.
#[allow(clippy::cast_possible_truncation)]
fn whole_seconds(value: f64) -> Option<i64> {
    // 10¹⁵ seconds is ~31 million years — anything past it is garbage, not a time.
    const LIMIT: f64 = 1.0e15;
    (value.is_finite() && value.abs() < LIMIT).then(|| value.floor() as i64)
}

/// Field `key`, or `None` when it is absent or JSON `null`.
///
/// Folding "wrong type" into "absent" (in the typed helpers below) is the tolerance
/// docs/09 asks for: one bad optional field keeps the aircraft on screen. It does not
/// weaken the required fields — those use `?`, so absent still drops the record.
fn field<'a>(record: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    record.get(key).filter(|value| !value.is_null())
}

fn str_field<'a>(record: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    field(record, key)?.as_str()
}

fn f64_field(record: &Map<String, Value>, key: &str) -> Option<f64> {
    field(record, key)?.as_f64()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ---- Fixtures ---------------------------------------------------------------------------
    //
    // `include_str!` rather than a runtime read, as in `opensky::states`: a deleted fixture
    // is a build failure. Provenance is in tests/fixtures/airplaneslive/README.md — these
    // are hand-written to readsb's documented shape, and the belief is checked against the
    // real service by `airplanes_live`'s `#[ignore]`d live test.

    const NOMINAL: &str = include_str!("../tests/fixtures/airplaneslive/point_nominal.json");
    const EMPTY: &str = include_str!("../tests/fixtures/airplaneslive/point_empty.json");
    const NULLS: &str = include_str!("../tests/fixtures/airplaneslive/point_nulls.json");
    const MALFORMED: &str = include_str!("../tests/fixtures/airplaneslive/point_malformed.json");

    fn parse(fixture: &str) -> Vec<StateVector> {
        serde_json::from_str::<PointResponse>(fixture)
            .expect("fixture is a valid point response")
            .into_state_vectors(SourceId::AirplanesLive)
    }

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    // ---- The nominal case ---------------------------------------------------------------------

    /// Four records, one of them a `~`-prefixed TIS-B synthetic: three aircraft.
    #[test]
    fn the_nominal_fixture_yields_every_real_aircraft() {
        let states = parse(NOMINAL);
        assert_eq!(states.len(), 3);
        assert!(states.iter().all(|s| s.source == SourceId::AirplanesLive));
    }

    /// The full mapping, units converted. The values are chosen so the SI side is exact:
    /// 36,000 ft = 10,972.8 m, 450 kt = 231.5 m/s — the same altitude the `OpenSky` nominal
    /// fixture uses, so the two adapters' outputs are directly comparable.
    #[test]
    fn a_full_record_maps_and_converts_every_field() {
        let first = &parse(NOMINAL)[0];
        assert_eq!(
            *first,
            StateVector {
                icao24: hex("3c6444"),
                callsign: CallSign::new("DLH9LF"),
                // now (1721000000000 ms → 1721000000 s) minus seen_pos 0.
                ts: UnixSeconds(1_721_000_000),
                lat_deg: 50.0379,
                lon_deg: 8.5624,
                baro_alt_m: Some(10972.8),
                velocity_ms: Some(231.5),
                heading_deg: Some(87.3),
                vert_rate_ms: Some(0.0),
                on_ground: false,
                anonymous: false,
                source: SourceId::AirplanesLive,
            }
        );
    }

    /// The unit conversions, pinned one by one so a regression names the unit it broke.
    #[test]
    fn aviation_units_convert_to_si() {
        let climber = &parse(NOMINAL)[2];
        assert_eq!(
            climber.baro_alt_m,
            Some(3657.6),
            "12,000 ft is 3,657.6 m — a raw 12000 here means feet leaked through"
        );
        assert_eq!(
            climber.velocity_ms,
            Some(185.2),
            "360 kt is 185.2 m/s — a raw 360 here means knots leaked through"
        );
        assert_eq!(
            climber.vert_rate_ms,
            Some(-7.62),
            "-1,500 ft/min is -7.62 m/s — a raw -1500 here means ft/min leaked through"
        );
    }

    /// The checklist item's named case: `alt_baro` as the string `"ground"`.
    #[test]
    fn the_ground_altitude_string_marks_a_surface_target_with_no_altitude() {
        let taxiing = &parse(NOMINAL)[1];
        assert!(taxiing.on_ground);
        assert_eq!(
            taxiing.baro_alt_m, None,
            "\"ground\" is a surface flag, not an altitude of zero"
        );
        // The rest of the record is still read: a ground target keeps its identity and
        // taxi speed (9 kt = 4.63 m/s exactly).
        assert_eq!(taxiing.callsign, CallSign::new("UAL123"));
        assert_eq!(taxiing.velocity_ms, Some(4.63));
        // Western hemisphere (JFK): the sign check a lat/lon slip cannot survive.
        assert!(
            taxiing.lat_deg > 0.0 && taxiing.lon_deg < 0.0,
            "{taxiing:?}"
        );
    }

    /// Privacy rule 2.2 at the point the flag is set, as in 1.4.
    #[test]
    fn a_record_without_a_flight_field_is_anonymous() {
        let states = parse(NOMINAL);
        assert_eq!(states[2].callsign, None);
        assert!(states[2].anonymous);
        assert!(!states[0].anonymous);
    }

    /// readsb marks TIS-B / ADS-R synthetics with a `~` hex; they are not aircraft
    /// addresses and must be skipped, not tracked under a minted identity.
    #[test]
    fn a_tilde_prefixed_hex_is_skipped() {
        assert!(
            !parse(NOMINAL)
                .iter()
                .any(|s| s.callsign == CallSign::new("N123AB")),
            "the TIS-B record slipped through"
        );
    }

    /// `seen_pos` dates the position: `ts` is `now` minus the position's age, floored.
    #[test]
    fn ts_is_now_minus_seen_pos() {
        let states = parse(NOMINAL);
        assert_eq!(states[0].ts, UnixSeconds(1_721_000_000), "seen_pos 0");
        assert_eq!(states[1].ts, UnixSeconds(1_720_999_998), "seen_pos 2.0");
        assert_eq!(
            states[2].ts,
            UnixSeconds(1_720_999_998),
            "seen_pos 1.5 floors, never rounds toward the future"
        );
    }

    // ---- `now` normalization --------------------------------------------------------------------

    /// The APIs send `now` in milliseconds; readsb's own file sends seconds. Both must
    /// yield the same timestamps.
    #[test]
    fn now_in_seconds_and_in_milliseconds_agree() {
        let record = json!({ "hex": "3c6444", "lat": 50.0, "lon": 8.0, "seen_pos": 5.0 });
        for now in [json!(1_721_000_000.0), json!(1_721_000_000_000.0_f64)] {
            let response: PointResponse =
                serde_json::from_value(json!({ "ac": [record.clone()], "now": now }))
                    .expect("valid response");
            let states = response.into_state_vectors(SourceId::AdsbLol);
            assert_eq!(states[0].ts, UnixSeconds(1_720_999_995), "now = {now}");
            // Also the proof the parser stamps whatever source it is told — 1.6's reuse.
            assert_eq!(states[0].source, SourceId::AdsbLol);
        }
    }

    /// Without `now` nothing can be timestamped: every record is skipped (the warn path),
    /// never a panic, never a batch dated to receipt time.
    #[test]
    fn a_response_without_now_yields_no_records() {
        let response: PointResponse = serde_json::from_value(json!({
            "ac": [{ "hex": "3c6444", "lat": 50.0, "lon": 8.0, "seen_pos": 1.0 }],
        }))
        .expect("valid response");
        assert!(
            response
                .into_state_vectors(SourceId::AirplanesLive)
                .is_empty()
        );
    }

    // ---- The awkward cases -----------------------------------------------------------------------

    #[test]
    fn an_empty_region_yields_no_aircraft_rather_than_an_error() {
        assert!(parse(EMPTY).is_empty());
    }

    #[test]
    fn a_missing_or_null_ac_list_is_an_empty_sky() {
        assert!(parse(r#"{"now": 1721000000000, "msg": "No error"}"#).is_empty());
        assert!(parse(r#"{"now": 1721000000000, "ac": null}"#).is_empty());
    }

    /// docs/10 §2: nulls in every optional field still yield a position.
    #[test]
    fn nulls_in_every_optional_field_still_yield_a_position() {
        let states = parse(NULLS);
        let all_null = &states[0];
        assert_eq!(
            *all_null,
            StateVector {
                icao24: hex("3c6444"),
                callsign: None,
                ts: UnixSeconds(1_720_999_997),
                lat_deg: 50.0379,
                lon_deg: 8.5624,
                baro_alt_m: None,
                velocity_ms: None,
                heading_deg: None,
                vert_rate_ms: None,
                on_ground: false,
                anonymous: true,
                source: SourceId::AirplanesLive,
            },
            "an unknown heading is None, never 0 — a zero heading is a different fact"
        );

        // The same tolerance for fields that are absent rather than null.
        let sparse = &states[1];
        assert_eq!(sparse.icao24, hex("4ca7b6"));
        assert_eq!(sparse.baro_alt_m, None);
        assert!(
            !sparse.on_ground,
            "no alt_baro at all is airborne, not ground"
        );
    }

    /// docs/10 §2: a malformed record mid-array must be skipped, not fail the batch.
    #[test]
    fn malformed_records_are_skipped_and_the_good_ones_survive() {
        let states = parse(MALFORMED);
        assert_eq!(
            states.iter().map(|s| s.icao24).collect::<Vec<_>>(),
            [hex("3c6444"), hex("4ca7b6")],
            "the two well-formed records must survive eleven broken neighbours"
        );
    }

    /// Names each way a record can be unusable, so a regression says which one.
    #[test]
    fn every_kind_of_unusable_record_is_rejected() {
        let good = json!({
            "hex": "3c6444", "flight": "DLH9LF  ",
            "lat": 50.0379, "lon": 8.5624, "seen_pos": 1.0,
        });
        assert!(
            state_vector(&good, 1_721_000_000.0, SourceId::AirplanesLive).is_some(),
            "the control must parse"
        );

        let cases = [
            (json!("not-an-object"), "a string where a record belongs"),
            (json!([]), "an array where a record belongs"),
            (json!(null), "a null record"),
            (json!({}), "an empty record"),
            (
                json!({ "lat": 50.0, "lon": 8.0, "seen_pos": 1.0 }),
                "no hex — no identity",
            ),
            (
                json!({ "hex": "~2e6a4b", "lat": 50.0, "lon": 8.0, "seen_pos": 1.0 }),
                "a ~-prefixed non-ICAO (TIS-B) hex",
            ),
            (
                json!({ "hex": "zzzzzz", "lat": 50.0, "lon": 8.0, "seen_pos": 1.0 }),
                "a non-hex address",
            ),
            (
                json!({ "hex": "3c6444", "lon": 8.0, "seen_pos": 1.0 }),
                "no latitude",
            ),
            (
                json!({ "hex": "3c6444", "lat": 50.0, "seen_pos": 1.0 }),
                "no longitude",
            ),
            (
                json!({ "hex": "3c6444", "lat": 91.0, "lon": 8.0, "seen_pos": 1.0 }),
                "a latitude off the globe",
            ),
            (
                json!({ "hex": "3c6444", "lat": 50.0, "lon": 8.0 }),
                "no seen_pos — the position has no time of applicability",
            ),
            (
                json!({ "hex": "3c6444", "lat": "50.0", "lon": 8.0, "seen_pos": 1.0 }),
                "a latitude that is a string — required fields are never coerced",
            ),
        ];
        for (record, why) in cases {
            assert!(
                state_vector(&record, 1_721_000_000.0, SourceId::AirplanesLive).is_none(),
                "must be skipped: {why}"
            );
        }
    }

    /// The tolerance rule on optional fields: wrong type reads as absent, and a good
    /// field after a bad one is still read.
    #[test]
    fn a_wrongly_typed_optional_field_reads_as_absent() {
        let record = json!({
            "hex": "3c6444",
            "flight": 42,
            "lat": 50.0379, "lon": 8.5624, "seen_pos": 1.0,
            "alt_baro": true,
            "gs": "fast",
            "track": 87.3,
        });
        let state =
            state_vector(&record, 1_721_000_000.0, SourceId::AirplanesLive).expect("parses");
        assert_eq!(state.callsign, None, "a numeric callsign is no callsign");
        assert!(state.anonymous);
        assert_eq!(state.baro_alt_m, None, "a boolean altitude is no altitude");
        assert!(!state.on_ground, "only the exact string \"ground\" grounds");
        assert_eq!(state.velocity_ms, None);
        assert_eq!(state.heading_deg, Some(87.3));
    }

    /// The parser is handed bytes from the network; nothing it can be handed may panic.
    #[test]
    fn arbitrary_json_never_panics_the_parser() {
        let shapes = [
            json!({"ac": "not-a-list", "now": 1_721_000_000_000.0_f64}),
            json!({"ac": [], "now": "yesterday"}),
            json!({"ac": [[]], "now": 1_721_000_000_000.0_f64}),
            json!({"ac": [{"hex": 12345, "lat": {}, "lon": [], "seen_pos": false}], "now": 0}),
            json!({"ac": [{"hex": "3c6444", "lat": 50.0, "lon": 8.0, "seen_pos": 1.0}], "now": -1.0e300}),
        ];
        for shape in shapes {
            // An `ac` that is not a list at all is a Parse error by contract; everything
            // else must yield records or skip them, never panic.
            if let Ok(response) = serde_json::from_value::<PointResponse>(shape.clone()) {
                let _ = response.into_state_vectors(SourceId::AirplanesLive);
            }
        }
    }

    // ---- The unit and time helpers -------------------------------------------------------------

    /// The definitions, and the conversions as the pipeline stores them: exactness is
    /// asserted at `f32` — where [`StateVector`] keeps the value — because 1852/3600 is
    /// not a dyadic rational and the `f64` product carries one trailing ulp.
    #[test]
    fn the_conversion_constants_are_the_defined_values() {
        assert_eq!(METRES_PER_FOOT, 0.3048, "international foot");
        assert_eq!(METRES_PER_NAUTICAL_MILE, 1852.0, "nautical mile");
        assert_eq!(narrow(450.0 * METRES_PER_SECOND_PER_KNOT), 231.5, "450 kt");
        assert_eq!(
            narrow(-1500.0 * METRES_PER_SECOND_PER_FOOT_PER_MINUTE),
            -7.62,
            "-1,500 ft/min"
        );
    }

    #[test]
    fn epoch_seconds_passes_seconds_through_and_scales_milliseconds() {
        assert_eq!(epoch_seconds(1_721_000_000.0), 1_721_000_000.0);
        assert_eq!(epoch_seconds(1_721_000_000_000.0), 1_721_000_000.0);
    }

    #[test]
    fn whole_seconds_floors_and_refuses_non_times() {
        assert_eq!(whole_seconds(1_721_000_000.9), Some(1_721_000_000));
        assert_eq!(whole_seconds(-0.5), Some(-1), "floor, not truncate");
        assert_eq!(whole_seconds(f64::NAN), None);
        assert_eq!(whole_seconds(f64::INFINITY), None);
        assert_eq!(whole_seconds(1.0e300), None, "not a time anyone flew at");
    }
}
