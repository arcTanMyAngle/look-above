//! `OpenSky`'s `/states/all` — the adapter that turns a bounding box into aircraft.
//!
//! The response is **positional arrays**, not objects: each aircraft is a JSON array whose
//! meaning is entirely its index (`[icao24, callsign, origin_country, time_position,
//! last_contact, lon, lat, baro_altitude, on_ground, …]`). Two things follow from that, and
//! they shape this whole module.
//!
//! **Longitude comes before latitude.** Every other API here, and every map UI, says "lat,
//! lon". `OpenSky` says lon, lat. A swap compiles, parses, and puts aircraft in the wrong
//! hemisphere, so the indices are named constants ([`field`]) rather than literals, and
//! [`tests::longitude_and_latitude_are_not_swapped`] pins it against real geography.
//!
//! **Every field is nullable, including ones the docs call mandatory.** So parsing is
//! per-field tolerant and per-record fallible: a record we cannot use is skipped and counted,
//! never a reason to fail the batch (docs/09 — adapters never panic on a malformed record,
//! and `Parse` never kills the poller). What we refuse to invent is a *position*: a record
//! without a usable `icao24`, `time_position`, and lon/lat pair is dropped, because the rest
//! of the pipeline treats those four as facts.
//!
//! **Credits.** `OpenSky` bills a bbox query 1–4 credits by area against a 4,000/day
//! allowance. [`credit_cost`] is that table as a pure function of the query — the poller
//! prices a cycle *before* committing to it (docs/09), and the ledger in item 1.7 charges it.

use async_trait::async_trait;
use look_above_core::contracts::{LiveSource, RegionQuery};
use look_above_core::error::SourceError;
use look_above_core::types::{CallSign, Icao24, SourceId, StateVector, UnixSeconds};
use serde::Deserialize;
use serde_json::Value;

use crate::http::{HttpClient, send_json};
use crate::normalize::{coordinate, narrow};
use crate::opensky::auth::OpenSkyAuth;

/// `OpenSky`'s live-state endpoint (authorized-aviation-sources skill).
///
/// A different host from the token endpoint, which is why the allowlist carries both.
pub const STATES_ENDPOINT: &str = "https://opensky-network.org/api/states/all";

/// Where each fact sits in a `states` record.
///
/// The whole reason this module has named constants: `record[5]` and `record[6]` are lon and
/// lat *in that order*, and no compiler will ever notice them swapped. Indices past
/// `VERTICAL_RATE` (`sensors`, `geo_altitude`, `squawk`, `spi`, `position_source`,
/// `category`) are not listed because nothing downstream consumes them yet.
mod field {
    pub const ICAO24: usize = 0;
    pub const CALLSIGN: usize = 1;
    /// Time of applicability of the *position* — not `last_contact` (index 4), which is the
    /// last time `OpenSky` heard anything at all from the aircraft.
    pub const TIME_POSITION: usize = 3;
    pub const LONGITUDE: usize = 5;
    pub const LATITUDE: usize = 6;
    pub const BARO_ALTITUDE: usize = 7;
    pub const ON_GROUND: usize = 8;
    pub const VELOCITY: usize = 9;
    pub const TRUE_TRACK: usize = 10;
    pub const VERTICAL_RATE: usize = 11;
}

/// The `OpenSky` live-position source.
#[derive(Debug)]
pub struct OpenSkySource {
    client: HttpClient,
    auth: OpenSkyAuth,
    endpoint: String,
}

impl OpenSkySource {
    /// The source against the real endpoint.
    ///
    /// Takes an [`OpenSkyAuth`] rather than credentials so that a disabled source is
    /// expressible ([`OpenSkyAuth::disabled`]) — see [`fetch`](Self::fetch).
    pub fn new(client: HttpClient, auth: OpenSkyAuth) -> Self {
        Self::build(client, auth, STATES_ENDPOINT.to_owned())
    }

    /// The one real constructor. Private for the same reason [`OpenSkyAuth::build`] is: the
    /// endpoint override exists so tests can reach a mock, not so callers can retarget the
    /// adapter — and the allowlist would refuse them anyway.
    fn build(client: HttpClient, auth: OpenSkyAuth, endpoint: String) -> Self {
        Self {
            client,
            auth,
            endpoint,
        }
    }

    /// Whether credentials were configured. The poller skips a disabled source rather than
    /// calling [`fetch`](Self::fetch) to be told.
    pub fn is_enabled(&self) -> bool {
        self.auth.is_enabled()
    }
}

#[async_trait]
impl LiveSource for OpenSkySource {
    fn id(&self) -> SourceId {
        SourceId::OpenSky
    }

    fn cost(&self, query: &RegionQuery) -> u32 {
        credit_cost(query)
    }

    async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError> {
        // A disabled source is not an error *state* (item 1.3), but `fetch` has no way to say
        // "skip me" — so asking a source we cannot authenticate is a caller bug, reported as
        // one. `Auth` is not transient, so the poller fails over to the keyless fallbacks
        // (1.5–1.6) instead of retrying. Note what this deliberately does *not* do: OpenSky
        // also serves anonymous callers at 400 credits/day, and silently dropping to that
        // would turn a missing credential into a tenth of the budget and no clue why.
        let Some(token) = self.auth.token().await? else {
            return Err(SourceError::Auth {
                message: "OpenSky has no credentials configured, so the source is disabled"
                    .to_owned(),
            });
        };

        let mut request = self.client.get(&self.endpoint)?;
        if let Some(bbox) = query.bbox {
            // `OpenSky`'s own parameter names and its lamin/lomin/lamax/lomax order. A global
            // query sends none of them: the endpoint's default *is* the world, and sending a
            // ±180° box would be a different (4-credit) question with a worse answer.
            request = request.query(&[
                ("lamin", bbox.lat_min()),
                ("lomin", bbox.lon_min()),
                ("lamax", bbox.lat_max()),
                ("lomax", bbox.lon_max()),
            ]);
        }
        // `expose` at the point of use, never before (privacy rule 7.1a). `bearer_auth` puts
        // it in a header rather than the query string, so it stays out of proxy logs.
        request = request.bearer_auth(token.expose());

        let response: StatesResponse = send_json(request).await?;
        Ok(response.into_state_vectors())
    }
}

/// What `query` costs against the daily credit allowance, in credits.
///
/// `OpenSky` prices a bbox by its area in square degrees: 0–25 → 1, 25–100 → 2, 100–400 → 3,
/// anything larger (and the global query) → 4. Free registered accounts get 4,000/day and
/// privacy rule 1.3 asks us to stay under 80% of that, which is what makes this worth being a
/// pure function: item 1.7's ledger prices a cycle before spending on it.
///
/// A `pub fn` beside the trait method because the budget planner needs the price without
/// holding a source, and the alternative — a `cost` that only exists on an instance — would
/// have the ledger construct an adapter to ask a question about arithmetic.
pub fn credit_cost(query: &RegionQuery) -> u32 {
    let Some(bbox) = query.bbox else {
        return GLOBAL_COST;
    };
    let area = (bbox.lat_max() - bbox.lat_min()) * (bbox.lon_max() - bbox.lon_min());

    // Strict `<`, so a boundary area lands in the *dearer* tier. OpenSky documents the bands
    // as "0–25", "25–100", "100–400", which leaves each edge in two of them. Guessing cheap
    // is the one direction that hurts: the ledger would believe it holds credits it has
    // already spent, and overrunning a documented allowance is the thing rule 1.3 forbids.
    // Guessing dear costs us a slightly wider poll interval.
    if area < 25.0 {
        1
    } else if area < 100.0 {
        2
    } else if area < 400.0 {
        3
    } else {
        GLOBAL_COST
    }
}

/// The dearest tier: a global query, or a bbox over 400 square degrees.
const GLOBAL_COST: u32 = 4;

/// The `/states/all` reply.
///
/// `states` is `Option` because an empty region yields `null`, not `[]` — a distinction that
/// would otherwise be a parse error on every quiet bbox. The elements are `Value`, not
/// `Vec<Value>`, on purpose: typing them as arrays would make one non-array record fail the
/// whole batch, and docs/10 §2 requires a malformed record mid-array to be skipped instead.
/// `time` and every other top-level field are ignored — we take each record's own
/// `time_position` as its time of applicability.
#[derive(Debug, Deserialize)]
struct StatesResponse {
    #[serde(default)]
    states: Option<Vec<Value>>,
}

impl StatesResponse {
    fn into_state_vectors(self) -> Vec<StateVector> {
        let records = self.states.unwrap_or_default();
        let vectors: Vec<StateVector> = records.iter().filter_map(state_vector).collect();
        let skipped = records.len() - vectors.len();

        if skipped > 0 {
            if vectors.is_empty() {
                // Losing every record is what a changed field order would look like, and an
                // empty sky does not explain itself. Routine skips are not warn-worthy;
                // this is.
                tracing::warn!(
                    skipped,
                    "every OpenSky record was unusable — the response shape may have changed"
                );
            } else {
                // Routine: OpenSky lists aircraft it has heard from but has no position for.
                tracing::debug!(
                    kept = vectors.len(),
                    skipped,
                    "skipped OpenSky records with no usable position"
                );
            }
        }
        vectors
    }
}

/// One record → one [`StateVector`], or `None` if it cannot be trusted.
///
/// `None` covers everything from "not even an array" to "latitude 91": the caller counts and
/// logs, and the batch carries on. The four facts required here — address, time, longitude,
/// latitude — are the ones the pipeline treats as given; the rest are genuinely optional
/// upstream and stay `Option` all the way to the renderer.
fn state_vector(record: &Value) -> Option<StateVector> {
    let record = record.as_array()?;

    let icao24 = Icao24::from_hex(str_at(record, field::ICAO24)?).ok()?;

    // `time_position`, not `last_contact`. They differ when OpenSky has heard the aircraft
    // recently but not its position, and using the newer one would date a stale fix to now —
    // M2's dead reckoning would then advance it from a place it had already left, drawing a
    // confidently wrong aircraft. `StateVector::ts` is defined as the source's time of
    // applicability; when there isn't one, we have no position to report.
    let ts = UnixSeconds(i64_at(record, field::TIME_POSITION)?);

    // Longitude first — see the module docs.
    let lon_deg = coordinate(f64_at(record, field::LONGITUDE)?, 180.0)?;
    let lat_deg = coordinate(f64_at(record, field::LATITUDE)?, 90.0)?;

    let callsign = str_at(record, field::CALLSIGN).and_then(CallSign::new);

    Some(StateVector {
        icao24,
        // Privacy rule 2.2: a position with no identity is an unidentified target, and this
        // flag is what stops M3's enrichment lookup from trying to give it one. Note the
        // limit of what is claimed here — this catches the "no identity" half of 2.2. A PIA
        // hex that *does* broadcast a callsign is not detected: that needs the FAA's assigned
        // address ranges, which we do not have, and rule 2.1 notes our feeds already honor
        // the program. Recorded as a known gap in DECISION_LOG; the enrichment gate (M3) is
        // where it binds and where the range data will have to land.
        anonymous: callsign.is_none(),
        callsign,
        ts,
        lat_deg,
        lon_deg,
        baro_alt_m: f64_at(record, field::BARO_ALTITUDE).map(narrow),
        velocity_ms: f64_at(record, field::VELOCITY).map(narrow),
        heading_deg: f64_at(record, field::TRUE_TRACK).map(narrow),
        vert_rate_ms: f64_at(record, field::VERTICAL_RATE).map(narrow),
        // Documented as non-null, so absence means the shape drifted. Airborne is the
        // assumption that loses least: it costs a glyph, where skipping would cost the
        // aircraft.
        on_ground: bool_at(record, field::ON_GROUND).unwrap_or(false),
        source: SourceId::OpenSky,
    })
}

/// Field `index`, or `None` when it is absent, JSON `null`, or the wrong type.
///
/// Folding "wrong type" into "absent" is the tolerance docs/09 asks for: a string where a
/// float belongs is a source bug, and treating one bad optional field as a missing optional
/// field keeps the aircraft on screen. It does not weaken the required fields — those use
/// `?`, so absent still drops the record.
fn field(record: &[Value], index: usize) -> Option<&Value> {
    record.get(index).filter(|value| !value.is_null())
}

fn str_at(record: &[Value], index: usize) -> Option<&str> {
    field(record, index)?.as_str()
}

fn f64_at(record: &[Value], index: usize) -> Option<f64> {
    field(record, index)?.as_f64()
}

fn i64_at(record: &[Value], index: usize) -> Option<i64> {
    field(record, index)?.as_i64()
}

fn bool_at(record: &[Value], index: usize) -> Option<bool> {
    field(record, index)?.as_bool()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use look_above_core::secret::SecretString;
    use look_above_core::types::BBox;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::*;
    use crate::allowlist::{HostPolicy, is_authorized_host};
    use crate::http::{REQUEST_TIMEOUT, USER_AGENT};
    use crate::opensky::auth::{Credentials, SystemClock};

    // ---- Fixtures ---------------------------------------------------------------------------
    //
    // `include_str!` rather than a runtime read: a deleted or renamed fixture is then a build
    // failure rather than a test that fails with a path in the message. Provenance is in
    // tests/fixtures/opensky/README.md — these are hand-written to the *documented* shape,
    // which is exactly the belief `live_opensky_states_match_the_documented_shape` exists to
    // check against the real thing.

    const NOMINAL: &str = include_str!("../../tests/fixtures/opensky/states_nominal.json");
    const EMPTY: &str = include_str!("../../tests/fixtures/opensky/states_empty.json");
    const NULLS: &str = include_str!("../../tests/fixtures/opensky/states_nulls.json");
    const MALFORMED: &str = include_str!("../../tests/fixtures/opensky/states_malformed.json");

    fn parse(fixture: &str) -> Vec<StateVector> {
        serde_json::from_str::<StatesResponse>(fixture)
            .expect("fixture is a valid states response")
            .into_state_vectors()
    }

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    fn bbox(lat_min: f64, lon_min: f64, lat_max: f64, lon_max: f64) -> BBox {
        BBox::new(lat_min, lon_min, lat_max, lon_max).expect("valid bbox in test")
    }

    /// A region well inside the 1-credit tier, used wherever the box itself is not the point.
    fn a_region() -> RegionQuery {
        RegionQuery::region(bbox(48.0, 4.0, 52.0, 10.0))
    }

    /// The real client, widened to reach a loopback mock — the same one-line escape hatch the
    /// `http` and `auth` tests use, so the shipping User-Agent, timeout, and allowlist are
    /// what these assertions run through.
    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn credentials() -> Credentials {
        Credentials::new(
            SecretString::from("test-client-id"),
            SecretString::from("test-client-secret"),
        )
    }

    /// Mounts a token endpoint on `server` handing out `tok-1`, and returns auth wired to it.
    ///
    /// The real clock: at a 1,800 s TTL nothing here lives long enough to refresh, and the
    /// schedule is `auth`'s test to run, not this module's.
    async fn auth_against(server: &MockServer) -> OpenSkyAuth {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-1",
                "expires_in": 1800,
                "token_type": "Bearer",
            })))
            .mount(server)
            .await;

        OpenSkyAuth::build(
            client(),
            credentials(),
            format!("{}/token", server.uri()),
            Arc::new(SystemClock),
        )
    }

    /// A fully wired source — token endpoint and states endpoint both on `server`.
    async fn source_against(server: &MockServer) -> OpenSkySource {
        let auth = auth_against(server).await;
        OpenSkySource::build(client(), auth, format!("{}/api/states/all", server.uri()))
    }

    /// Mounts `/api/states/all` returning `body`.
    async fn mock_states(server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .and(path("/api/states/all"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// The `/states/all` request the source actually sent.
    ///
    /// Found by path rather than by position: the token POST shares this server, and an
    /// index would silently start asserting against the wrong request the day the cache
    /// changes when it fetches.
    async fn states_request(server: &MockServer) -> Request {
        server
            .received_requests()
            .await
            .expect("requests are recorded")
            .into_iter()
            .find(|request| request.url.path() == "/api/states/all")
            .expect("the states request was made")
    }

    fn query_value(request: &Request, name: &str) -> Option<String> {
        request
            .url
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }

    // ---- Parsing: the nominal case -----------------------------------------------------------

    #[test]
    fn the_nominal_fixture_yields_every_aircraft() {
        let states = parse(NOMINAL);
        assert_eq!(states.len(), 4);
        assert!(states.iter().all(|s| s.source == SourceId::OpenSky));
    }

    #[test]
    fn a_full_record_maps_every_field_it_carries() {
        let first = &parse(NOMINAL)[0];
        assert_eq!(
            *first,
            StateVector {
                icao24: hex("3c6444"),
                callsign: CallSign::new("DLH9LF"),
                // time_position (index 3), not last_contact.
                ts: UnixSeconds(1_721_000_000),
                lat_deg: 50.0379,
                lon_deg: 8.5624,
                baro_alt_m: Some(10972.8),
                velocity_ms: Some(231.45),
                heading_deg: Some(87.3),
                vert_rate_ms: Some(0.0),
                on_ground: false,
                anonymous: false,
                source: SourceId::OpenSky,
            }
        );
    }

    /// The single most consequential thing in this module: `OpenSky` sends lon before lat.
    ///
    /// Asserted against geography rather than against the fixture's numbers, because a test
    /// that reads the same two indices the parser does would agree with a swap. Frankfurt is
    /// at ~50°N ~8.6°E and JFK at ~40.6°N ~-73.8°E: both have |lat| > |lon| in one case and
    /// opposite signs in the other, so a swap cannot survive either.
    #[test]
    fn longitude_and_latitude_are_not_swapped() {
        let states = parse(NOMINAL);

        let frankfurt = &states[0];
        assert!(
            (49.0..51.0).contains(&frankfurt.lat_deg),
            "3c6444 should be near Frankfurt (50°N), got lat {}",
            frankfurt.lat_deg
        );
        assert!(
            (8.0..9.0).contains(&frankfurt.lon_deg),
            "3c6444 should be near Frankfurt (8.6°E), got lon {}",
            frankfurt.lon_deg
        );

        // Western hemisphere: a swap here flips the sign onto the latitude.
        let jfk = &states[2];
        assert!(jfk.lat_deg > 0.0 && jfk.lon_deg < 0.0, "{jfk:?}");
    }

    #[test]
    fn a_callsign_is_trimmed_of_opensky_s_padding() {
        // OpenSky right-pads callsigns to 8 characters.
        assert_eq!(
            parse(NOMINAL)[0].callsign.as_ref().map(CallSign::as_str),
            Some("DLH9LF")
        );
    }

    #[test]
    fn an_aircraft_on_the_ground_is_marked_and_has_no_barometric_altitude() {
        let jfk = &parse(NOMINAL)[2];
        assert!(jfk.on_ground);
        assert_eq!(
            jfk.baro_alt_m, None,
            "OpenSky nulls baro_altitude on the ground; that must stay None, not become 0"
        );
    }

    /// Privacy rule 2.2, at the point the flag is set.
    #[test]
    fn a_record_without_a_callsign_is_anonymous() {
        let states = parse(NOMINAL);

        let unidentified = &states[3];
        assert_eq!(unidentified.callsign, None);
        assert!(
            unidentified.anonymous,
            "a position with no identity must be flagged so enrichment never looks it up"
        );

        assert!(
            !states[0].anonymous,
            "an identified aircraft must not be flagged anonymous"
        );
    }

    // ---- Parsing: the awkward cases -----------------------------------------------------------

    /// docs/10 §2: the empty region. `null`, not `[]` — the case that would otherwise be a
    /// parse error over every quiet bbox.
    #[test]
    fn an_empty_region_yields_no_aircraft_rather_than_an_error() {
        assert!(parse(EMPTY).is_empty());
    }

    #[test]
    fn a_response_without_a_states_key_at_all_is_an_empty_sky() {
        assert!(parse(r#"{"time": 1721000000}"#).is_empty());
    }

    /// docs/10 §2: nulls in every optional field.
    #[test]
    fn nulls_in_every_optional_field_still_yield_a_position() {
        let all_null = &parse(NULLS)[0];
        assert_eq!(
            *all_null,
            StateVector {
                icao24: hex("3c6444"),
                callsign: None,
                ts: UnixSeconds(1_721_000_000),
                lat_deg: 50.0379,
                lon_deg: 8.5624,
                baro_alt_m: None,
                velocity_ms: None,
                heading_deg: None,
                vert_rate_ms: None,
                // Documented non-null, sent null here: airborne is the assumption.
                on_ground: false,
                anonymous: true,
                source: SourceId::OpenSky,
            },
            "an unknown heading is None, never 0 — a zero heading is a different fact"
        );
    }

    /// A record can simply stop early: the 18-field form (with `category`) and the 17-field
    /// form both exist in the wild, and a future field would extend it again.
    #[test]
    fn a_record_shorter_than_the_documented_array_is_tolerated() {
        let truncated = &parse(NULLS)[1];
        assert_eq!(truncated.icao24, hex("4ca7b6"));
        assert_eq!(truncated.heading_deg, Some(271.9), "the last field present");
        assert_eq!(
            truncated.vert_rate_ms, None,
            "a field past the end of the array is absent, not a failure"
        );
    }

    /// docs/10 §2: a malformed record mid-array must be skipped, not fail the batch.
    #[test]
    fn malformed_records_are_skipped_and_the_good_ones_survive() {
        let states = parse(MALFORMED);
        assert_eq!(
            states.iter().map(|s| s.icao24).collect::<Vec<_>>(),
            [hex("3c6444"), hex("4ca7b7")],
            "the two well-formed records must survive six broken neighbours"
        );
    }

    /// Names each way the fixture above is broken, so a regression says which one.
    #[test]
    fn every_kind_of_unusable_record_is_rejected() {
        let good = json!([
            "3c6444",
            "DLH9LF  ",
            "Germany",
            1_721_000_000,
            1_721_000_000,
            8.5624,
            50.0379,
            10972.8,
            false,
            231.45,
            87.3,
            0.0
        ]);
        assert!(state_vector(&good).is_some(), "the control must parse");

        let cases = [
            (json!("not-an-array"), "a string where a record belongs"),
            (json!([]), "an empty record"),
            (json!(null), "a null record"),
            (
                json!([
                    "zzzzzz",
                    "BAD1",
                    "Nowhere",
                    1_721_000_000,
                    1_721_000_000,
                    8.0,
                    50.0
                ]),
                "a non-hex address",
            ),
            (
                json!([
                    "3c6444",
                    "X",
                    "Germany",
                    1_721_000_000,
                    1_721_000_000,
                    null,
                    50.0379
                ]),
                "no longitude",
            ),
            (
                json!([
                    "3c6444",
                    "X",
                    "Germany",
                    1_721_000_000,
                    1_721_000_000,
                    8.5624,
                    null
                ]),
                "no latitude",
            ),
            (
                json!([
                    "3c6444",
                    "X",
                    "Germany",
                    null,
                    1_721_000_000,
                    8.5624,
                    50.0379
                ]),
                "no time_position — the position has no time of applicability",
            ),
            (
                json!([
                    "3c6444",
                    "X",
                    "Germany",
                    1_721_000_000,
                    1_721_000_000,
                    500.0,
                    91.0
                ]),
                "coordinates outside the globe",
            ),
            (
                json!([
                    "3c6444",
                    "X",
                    "Germany",
                    1_721_000_000,
                    1_721_000_000,
                    8.5624
                ]),
                "a record that stops before the latitude",
            ),
        ];
        for (record, why) in cases {
            assert!(state_vector(&record).is_none(), "must be skipped: {why}");
        }
    }

    /// The parser is handed bytes from the network; nothing it can be handed may panic.
    #[test]
    fn arbitrary_json_never_panics_the_parser() {
        let shapes = [
            json!({"states": "not-an-array"}),
            json!({"states": []}),
            json!({"states": [[{}, [], 0, false]]}),
            json!({"states": [[12345, 6789]]}),
            json!({"states": [["3c6444", 42, "Germany", "not-a-number", 0, "8.5", "50.0"]]}),
            json!({"states": [[null, null, null, null, null, null, null]]}),
        ];
        for shape in shapes {
            // A `states` that is not an array is a shape we cannot read at all — that is a
            // Parse error by contract, not an empty sky. Everything else must yield records.
            let parsed = serde_json::from_value::<StatesResponse>(shape.clone());
            if let Ok(response) = parsed {
                let _ = response.into_state_vectors();
            }
        }
    }

    /// A `Value` that is a number where the parser wants a string, and vice versa: the
    /// tolerance rule is "wrong type on an optional field reads as absent".
    #[test]
    fn a_wrongly_typed_optional_field_reads_as_absent() {
        let record = json!([
            "3c6444",
            42,
            "Germany",
            1_721_000_000,
            1_721_000_000,
            8.5624,
            50.0379,
            "high",
            false,
            "fast",
            87.3
        ]);
        let state = state_vector(&record).expect("the required fields are all good");
        assert_eq!(state.callsign, None, "a numeric callsign is no callsign");
        assert!(state.anonymous);
        assert_eq!(state.baro_alt_m, None, "a string altitude is no altitude");
        assert_eq!(state.velocity_ms, None);
        assert_eq!(
            state.heading_deg,
            Some(87.3),
            "a good field after a bad one must still be read"
        );
    }

    // ---- Credits -------------------------------------------------------------------------------

    /// The skill's table: 0–25 → 1, 25–100 → 2, 100–400 → 3, larger → 4.
    #[test]
    fn credit_cost_follows_opensky_s_published_tiers() {
        let cases = [
            // (bbox, area in square degrees, credits)
            (bbox(50.0, 8.0, 51.0, 9.0), 1.0, 1),
            (bbox(50.0, 8.0, 54.0, 14.0), 24.0, 1),
            (bbox(40.0, 0.0, 45.0, 10.0), 50.0, 2),
            (bbox(40.0, 0.0, 49.0, 10.0), 90.0, 2),
            (bbox(30.0, 0.0, 45.0, 20.0), 300.0, 3),
            (bbox(0.0, 0.0, 30.0, 30.0), 900.0, 4),
            (bbox(-90.0, -180.0, 90.0, 180.0), 64800.0, 4),
        ];
        for (region, area, expected) in cases {
            assert_eq!(
                credit_cost(&RegionQuery::region(region)),
                expected,
                "a {area}°² bbox should cost {expected} credits"
            );
        }
    }

    /// Every tier boundary sits in the dearer band. Under-pricing is the failure that
    /// overruns the allowance; over-pricing only widens the poll interval.
    #[test]
    fn a_tier_boundary_is_charged_at_the_dearer_rate() {
        let cases = [
            (bbox(50.0, 8.0, 55.0, 13.0), 25.0, 2),
            (bbox(40.0, 0.0, 50.0, 10.0), 100.0, 3),
            (bbox(20.0, 0.0, 40.0, 20.0), 400.0, 4),
        ];
        for (region, area, expected) in cases {
            assert_eq!(
                credit_cost(&RegionQuery::region(region)),
                expected,
                "exactly {area}°² must be charged as the dearer tier"
            );
        }
    }

    #[test]
    fn a_global_query_costs_the_maximum() {
        assert_eq!(credit_cost(&RegionQuery::GLOBAL), 4);
    }

    #[test]
    fn a_degenerate_bbox_is_still_priced() {
        // A zero-area box is a legal BBox (min == max). It must price, not divide by zero.
        assert_eq!(
            credit_cost(&RegionQuery::region(bbox(50.0, 8.0, 50.0, 8.0))),
            1
        );
    }

    #[test]
    fn the_trait_method_and_the_free_function_agree() {
        let source = OpenSkySource::new(
            HttpClient::new().expect("client builds"),
            OpenSkyAuth::disabled(),
        );
        assert_eq!(source.id(), SourceId::OpenSky);
        for query in [
            RegionQuery::GLOBAL,
            RegionQuery::region(bbox(50.0, 8.0, 51.0, 9.0)),
            RegionQuery::region(bbox(0.0, 0.0, 30.0, 30.0)),
        ] {
            assert_eq!(source.cost(&query), credit_cost(&query));
        }
    }

    /// M1's regions are bboxes ≤ ~1,000 km across (the plan's design notes). At 50°N that is
    /// well inside the cheapest tier — worth pinning, because it is the number the budget in
    /// 1.7 will be built on.
    #[test]
    fn a_typical_regional_view_costs_a_single_credit() {
        // ~9° of longitude at 50°N ≈ 640 km; 4° of latitude ≈ 445 km.
        assert_eq!(
            credit_cost(&RegionQuery::region(bbox(48.0, 4.0, 52.0, 10.0))),
            1
        );
    }

    // ---- The request on the wire -----------------------------------------------------------------

    /// docs/10 §2: assert the request shape, not just the parse.
    ///
    /// Checks the parameters' *values*, not their spelling: `48.5`, `48.50` and `4.85e1` are
    /// the same question, and pinning one of them would be a test of `serde_urlencoded`'s
    /// float formatter rather than of this adapter. The fractional coordinates are the point —
    /// a formatter that rounded would snap every bbox to whole degrees, and that is a fact
    /// about the value, which this sees.
    #[tokio::test]
    async fn a_bbox_query_sends_opensky_s_four_parameters() {
        let server = MockServer::start().await;
        mock_states(&server, EMPTY).await;

        let source = source_against(&server).await;
        source
            .fetch(&RegionQuery::region(bbox(48.5, -4.25, 52.0, 10.0)))
            .await
            .expect("the fetch succeeds");

        let request = states_request(&server).await;
        for (name, expected) in [
            ("lamin", 48.5),
            ("lomin", -4.25),
            ("lamax", 52.0),
            ("lomax", 10.0),
        ] {
            let value =
                query_value(&request, name).unwrap_or_else(|| panic!("no {name} parameter sent"));
            let sent: f64 = value
                .parse()
                .unwrap_or_else(|_| panic!("{name}={value} is not a number"));
            assert_eq!(sent, expected, "{name} carried the wrong bound");
        }
        assert_eq!(
            request.headers["user-agent"], USER_AGENT,
            "the adapter must not bypass the shared client"
        );
    }

    /// A global query is the endpoint's default: sending a ±180° box would be a different
    /// question, and `RegionQuery` keeps the distinction alive precisely so this can hold.
    #[tokio::test]
    async fn a_global_query_sends_no_bbox_parameters() {
        let server = MockServer::start().await;
        mock_states(&server, EMPTY).await;

        let source = source_against(&server).await;
        source
            .fetch(&RegionQuery::GLOBAL)
            .await
            .expect("the fetch succeeds");

        assert_eq!(
            states_request(&server).await.url.query(),
            None,
            "a global query must not carry bbox parameters"
        );
    }

    /// Privacy rule 7.1a: the token rides in a header, never in the URL, because a query
    /// string reaches proxy logs and error messages.
    #[tokio::test]
    async fn the_request_carries_the_bearer_token_in_a_header_and_not_the_url() {
        let server = MockServer::start().await;
        mock_states(&server, EMPTY).await;

        let source = source_against(&server).await;
        source.fetch(&a_region()).await.expect("the fetch succeeds");

        let request = states_request(&server).await;
        assert_eq!(request.headers["authorization"], "Bearer tok-1");
        assert!(
            !request.url.as_str().contains("tok-1"),
            "the token leaked into the URL: {}",
            request.url
        );
    }

    /// The whole batch comes back through a real `HttpClient` and `send_json`, not just the
    /// parser: this is the test that would catch the endpoint, the header, and the decode
    /// disagreeing with each other.
    #[tokio::test]
    async fn a_fetch_returns_the_parsed_batch() {
        let server = MockServer::start().await;
        mock_states(&server, NOMINAL).await;

        let source = source_against(&server).await;
        let states = source.fetch(&a_region()).await.expect("the fetch succeeds");

        assert_eq!(states.len(), 4);
        assert_eq!(states[0].icao24, hex("3c6444"));
        assert!(states[3].anonymous);
    }

    #[tokio::test]
    async fn a_fetch_over_a_malformed_batch_still_returns_the_good_records() {
        let server = MockServer::start().await;
        mock_states(&server, MALFORMED).await;

        let source = source_against(&server).await;
        let states = source
            .fetch(&a_region())
            .await
            .expect("a malformed record must never fail the fetch");
        assert_eq!(states.len(), 2);
    }

    // ---- Failure paths ------------------------------------------------------------------------------

    /// The disabled source must not send anything at all — not even to be told no.
    #[tokio::test]
    async fn a_disabled_source_fails_without_sending_a_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/all"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(NOMINAL, "application/json"))
            .expect(0)
            .mount(&server)
            .await;

        let source = OpenSkySource::build(
            client(),
            OpenSkyAuth::disabled(),
            format!("{}/api/states/all", server.uri()),
        );
        assert!(!source.is_enabled());

        let error = source
            .fetch(&a_region())
            .await
            .expect_err("a disabled source cannot fetch");
        assert!(matches!(error, SourceError::Auth { .. }), "{error:?}");
        assert!(
            !error.is_transient(),
            "no credentials will not appear by retrying — the poller must fail over"
        );
        // expect(0): asserted on drop.
    }

    /// docs/10 §2 asks for a 429 case. The status mapping itself belongs to `http` and is
    /// tested there; what matters here is that it survives the adapter intact, including
    /// `OpenSky`'s own non-standard retry header.
    #[tokio::test]
    async fn a_rate_limited_fetch_surfaces_opensky_s_retry_hint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/all"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("x-rate-limit-retry-after-seconds", "42"),
            )
            .mount(&server)
            .await;

        let source = source_against(&server).await;
        let error = source
            .fetch(&a_region())
            .await
            .expect_err("429 is an error");
        assert_eq!(
            error,
            SourceError::RateLimited {
                retry_after: Some(std::time::Duration::from_secs(42))
            }
        );
        assert!(error.is_transient());
    }

    /// docs/10 §2 asks for a 5xx case.
    #[tokio::test]
    async fn an_upstream_failure_surfaces_as_server_and_is_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/all"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let source = source_against(&server).await;
        let error = source
            .fetch(&a_region())
            .await
            .expect_err("503 is an error");
        assert_eq!(error, SourceError::Server { status: 503 });
        assert!(error.is_transient());
    }

    /// A body we cannot read is a `Parse`, and by contract that never kills the poller.
    #[tokio::test]
    async fn an_unreadable_body_surfaces_as_parse() {
        let server = MockServer::start().await;
        mock_states(&server, "<html>maintenance</html>").await;

        let source = source_against(&server).await;
        let error = source
            .fetch(&a_region())
            .await
            .expect_err("html is not a states response");
        assert!(matches!(error, SourceError::Parse { .. }), "{error:?}");
        assert!(!error.is_transient());
    }

    /// A rejected token must not be reported as anything but an auth failure.
    #[tokio::test]
    async fn a_rejected_token_surfaces_as_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/all"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let source = source_against(&server).await;
        let error = source
            .fetch(&a_region())
            .await
            .expect_err("401 is an error");
        assert!(matches!(error, SourceError::Auth { .. }), "{error:?}");
        assert!(!error.is_transient());
    }

    // ---- The allowlist ---------------------------------------------------------------------------------

    #[test]
    fn the_states_endpoint_is_the_documented_one_and_is_authorized() {
        assert_eq!(
            STATES_ENDPOINT,
            "https://opensky-network.org/api/states/all"
        );
        let host = reqwest::Url::parse(STATES_ENDPOINT)
            .expect("the endpoint parses")
            .host_str()
            .expect("the endpoint has a host")
            .to_owned();
        assert!(is_authorized_host(&host), "{host} must be on the allowlist");
    }

    /// Privacy rule 1.1 on the shipping client: whatever endpoint a bug names, the aircraft
    /// request goes to `OpenSky` or nowhere.
    ///
    /// Auth runs over the loopback-widened client and succeeds, so the source holds a valid
    /// token and the only thing left to stop the request is the allowlist. That ordering is
    /// the point — a test where auth failed first would pass without the gate existing.
    #[tokio::test]
    async fn the_real_client_will_not_fetch_states_from_an_unauthorized_host() {
        let server = MockServer::start().await;
        let auth = auth_against(&server).await;
        assert!(
            auth.token()
                .await
                .expect("the token fetch succeeds")
                .is_some(),
            "the refusal below must be about the states host, not about auth"
        );

        let source = OpenSkySource::build(
            HttpClient::new().expect("client builds"),
            auth,
            "https://www.flightradar24.com/api/states/all".to_owned(),
        );
        let error = source
            .fetch(&a_region())
            .await
            .expect_err("a prohibited host is refused");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(reason.contains("flightradar24"), "{reason}");
        assert!(!error.is_transient());
    }

    // ---- The real OpenSky -------------------------------------------------------------------------------

    /// The one test here that fetches real aircraft, and the reason the rest can be trusted.
    ///
    /// Every fixture in this module is **hand-written from `OpenSky`'s documentation**, so the
    /// mocks above prove only that we parse what we *believe* `OpenSky` sends. The belief is the
    /// risky part: field order in a positional array is invisible to the compiler, and lon/lat
    /// being backwards from every other source is exactly the kind of thing docs get wrong.
    /// This asserts the shape against the live endpoint.
    ///
    /// It costs **1 credit** of the 4,000/day allowance (a ~2°² bbox, the cheapest tier), so
    /// it is `#[ignore]`d — CI never runs it, and it needs credentials and a network anyway:
    ///
    /// ```text
    /// LOOK_ABOVE_OPENSKY_CLIENT_ID=… LOOK_ABOVE_OPENSKY_CLIENT_SECRET=… \
    ///     cargo test -p look-above-ingest -- --ignored live_opensky_states
    /// ```
    ///
    /// Nothing here prints a payload — only counts and ranges (docs/06: never paste raw API
    /// responses into a log or a transcript).
    #[tokio::test]
    #[ignore = "hits the real OpenSky /states/all and spends 1 credit; needs credentials"]
    async fn live_opensky_states_match_the_documented_shape() {
        let (Ok(client_id), Ok(client_secret)) = (
            std::env::var("LOOK_ABOVE_OPENSKY_CLIENT_ID"),
            std::env::var("LOOK_ABOVE_OPENSKY_CLIENT_SECRET"),
        ) else {
            panic!(
                "set LOOK_ABOVE_OPENSKY_CLIENT_ID and LOOK_ABOVE_OPENSKY_CLIENT_SECRET to \
                 run this test"
            );
        };

        // The real client, the real endpoint, no loopback widening.
        let source = OpenSkySource::new(
            HttpClient::new().expect("client builds"),
            OpenSkyAuth::new(
                HttpClient::new().expect("client builds"),
                Credentials::new(
                    SecretString::from(client_id),
                    SecretString::from(client_secret),
                ),
            ),
        );

        // Switzerland and the approaches to Zurich: reliably busy, and 2°×2° = 4°² keeps this
        // in the 1-credit tier.
        let region = bbox(46.0, 7.0, 48.0, 9.0);
        let query = RegionQuery::region(region);
        assert_eq!(
            credit_cost(&query),
            1,
            "this test must stay in the cheap tier"
        );

        let states = source.fetch(&query).await.expect("OpenSky answers");
        assert!(
            !states.is_empty(),
            "no aircraft over Switzerland — either the sky is empty or the parse is wrong"
        );

        for state in &states {
            // The headline: every aircraft must actually be inside the box we asked for. This
            // is what a lon/lat swap fails — swapped, these would be near (8°N, 47°E), which
            // is in Somalia, and `contains` would reject every one of them.
            assert!(
                region.contains(state.lat_deg, state.lon_deg),
                "{} is outside the requested bbox at lat {} lon {} — swapped coordinates?",
                state.icao24,
                state.lat_deg,
                state.lon_deg
            );
            assert!(state.ts.0 > 0, "{} has no timestamp", state.icao24);
            if let Some(altitude) = state.baro_alt_m {
                assert!(
                    (-500.0..=20000.0).contains(&altitude),
                    "{} reports {altitude} m — that is not an altitude",
                    state.icao24
                );
            }
            if let Some(heading) = state.heading_deg {
                assert!(
                    (0.0..=360.0).contains(&heading),
                    "{} reports heading {heading}",
                    state.icao24
                );
            }
        }

        // The optional fields must be populated for *someone*, or we are silently reading the
        // wrong indices and calling every field absent.
        assert!(
            states.iter().any(|s| s.callsign.is_some()),
            "not one aircraft had a callsign — index 1 is probably not the callsign"
        );
        assert!(
            states.iter().any(|s| s.velocity_ms.is_some()),
            "not one aircraft had a velocity — index 9 is probably not the velocity"
        );

        let anonymous = states.iter().filter(|s| s.anonymous).count();
        eprintln!(
            "live OpenSky /states/all: {} aircraft in {:?}, {} anonymous, {} on the ground, \
             1 credit spent",
            states.len(),
            region,
            anonymous,
            states.iter().filter(|s| s.on_ground).count(),
        );
    }
}
