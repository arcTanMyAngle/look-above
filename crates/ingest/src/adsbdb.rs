//! `api.adsbdb.com`'s aircraft/callsign lookup — the selection-only enrichment source for M3
//! item 3.4's `AircraftMeta`/`Flight` data (registration/type/operator, and callsign→route).
//!
//! Pure and stateless like [`crate::metar::MetarSource`]: [`AdsbdbSource::fetch_aircraft`]/
//! [`AdsbdbSource::fetch_route`] are one request in, one parsed answer out — no LRU, no
//! negative cache, no `Store` write. That orchestration is `app::enrichment`'s job (a
//! different crate: `ingest` and `store` must not depend on each other,
//! `crates/ingest/Cargo.toml`'s own "dependency direction" comment), which is also the only
//! caller allowed to invoke either fetch method, and only after checking [`should_enrich`]
//! (privacy rule 2.2).
//!
//! adsbdb's HTTP 404 for an unknown hex/callsign is the routine "not in the registry" outcome,
//! not a failure: both fetch methods catch it and return `Ok(None)`, distinct from every other
//! error, which still propagates — so the caller's negative cache is never poisoned by a
//! transient network/parse failure, only by a confirmed miss (see each method's own doc
//! comment). Verified live 2026-07-21: an unregistered hex (`000000`) answers HTTP 404 with no
//! body ever reaching [`send_json`]'s JSON decode step, which is why the 404 case is caught at
//! the [`SourceError`] level rather than by inspecting a "not found" response shape.

use look_above_core::contracts::{AircraftCategory, AircraftMeta, Flight};
use look_above_core::error::SourceError;
use look_above_core::types::{CallSign, Icao24, UnixSeconds};
use serde::Deserialize;
use serde_json::Value;

use crate::http::{HttpClient, send_json};

/// `api.adsbdb.com`'s base URL (authorized-aviation-sources skill); `/aircraft/{hex}` and
/// `/callsign/{callsign}` are appended by each lookup.
pub const BASE_URL: &str = "https://api.adsbdb.com/v0";

/// The `api.adsbdb.com` enrichment source.
#[derive(Debug)]
pub struct AdsbdbSource {
    client: HttpClient,
    base_url: String,
}

impl AdsbdbSource {
    /// The source against the real endpoint.
    pub fn new(client: HttpClient) -> Self {
        Self::build(client, BASE_URL.to_owned())
    }

    /// `pub(crate)` so `record_fixture`/tests can point this at a mock; the allowlist refuses
    /// an unauthorized endpoint regardless (see `http::HttpClient::checked_url`).
    pub(crate) fn build(client: HttpClient, base_url: String) -> Self {
        Self { client, base_url }
    }

    /// Looks up airframe metadata for `hex` (`GET /v0/aircraft/{hex}`).
    ///
    /// `now` is the caller's clock reading, not read from the system here — the same reasoning
    /// as [`crate::metar::MetarSource`]'s pollers taking a clock/timestamp explicitly, so
    /// parsing stays deterministic under test. `Ok(None)` covers both adsbdb's 404 (hex not in
    /// its registry — routine, not an error, see this module's own doc comment) and a 200 whose
    /// body does not actually carry an `aircraft` record (defensive: nothing observed live does
    /// this, but nothing in the shape documents that it never will).
    pub async fn fetch_aircraft(
        &self,
        hex: Icao24,
        now: UnixSeconds,
    ) -> Result<Option<AircraftMeta>, SourceError> {
        let url = format!("{}/aircraft/{hex}", self.base_url);
        let raw = match send_json::<Value>(self.client.get(&url)?).await {
            Ok(value) => value,
            Err(SourceError::Request { status: 404 }) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(parse_aircraft_response(&raw, hex, now))
    }

    /// Looks up the route on file for `callsign` (`GET /v0/callsign/{callsign}`), cached
    /// against `icao24` — the aircraft that was selected when the lookup was made, not
    /// anything adsbdb's callsign endpoint itself reports (it answers purely from the
    /// callsign, so the icao24 must come from the caller).
    ///
    /// `first_seen`/`last_seen` are both `now`: a fresh, single lookup, not a merged session
    /// (that's M5's job — `DECISION_LOG` 2026-07-21, M3 3.4). Same `Ok(None)` shape as
    /// [`fetch_aircraft`](Self::fetch_aircraft) for a 404 or a body with no route on file.
    pub async fn fetch_route(
        &self,
        callsign: &CallSign,
        icao24: Icao24,
        now: UnixSeconds,
    ) -> Result<Option<Flight>, SourceError> {
        let url = format!("{}/callsign/{}", self.base_url, callsign.as_str());
        let raw = match send_json::<Value>(self.client.get(&url)?).await {
            Ok(value) => value,
            Err(SourceError::Request { status: 404 }) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(parse_route_response(&raw, icao24, callsign, now))
    }
}

/// The wire shape of `response.aircraft` — only the fields [`AircraftMeta`] actually keeps.
/// Verified live 2026-07-21 against a real registered hex
/// (`tests/fixtures/adsbdb/aircraft_nominal.json`): `icao_type` is the short ICAO type
/// designator (e.g. `SR22`) `AircraftMeta::type_code`'s own doc comment wants; the sibling
/// `type` field is a separate, non-ICAO description and is deliberately not read.
/// `registered_owner_operator_flag_code`, `mode_s`, `url_photo`, `url_photo_thumbnail` are also
/// present live but nothing in `AircraftMeta` keeps them.
#[derive(Debug, Deserialize)]
struct RawAircraft {
    icao_type: Option<String>,
    registration: Option<String>,
    registered_owner: Option<String>,
}

/// Reads `response.aircraft` out of a whole `/v0/aircraft/{hex}` body, or `None` for any shape
/// that is not a populated aircraft record — an absent `response`/`aircraft` key, an explicit
/// `null`, or a record whose fields don't even deserialize. Never a panic or a propagated
/// error: the same "malformed maps to skip" rule [`crate::metar::RawMetar`] follows for one
/// element of a batch, applied here to the lone record this endpoint ever returns.
fn parse_aircraft_response(raw: &Value, hex: Icao24, now: UnixSeconds) -> Option<AircraftMeta> {
    let aircraft = raw.get("response")?.get("aircraft")?;
    if aircraft.is_null() {
        return None;
    }
    let raw_aircraft: RawAircraft = serde_json::from_value(aircraft.clone()).ok()?;
    Some(AircraftMeta {
        icao24: hex,
        registration: non_empty(raw_aircraft.registration),
        type_code: non_empty(raw_aircraft.icao_type),
        // adsbdb gives no jet/turboprop/piston/heli/glider signal — stays Unknown until a
        // classifier exists to derive one (not this item's job).
        category: AircraftCategory::Unknown,
        operator: non_empty(raw_aircraft.registered_owner),
        is_anonymous: false,
        fetched_at: Some(now),
        lookup_failed_at: None,
    })
}

/// The wire shape of `response.flightroute.origin`/`.destination` — only the ICAO code
/// [`Flight`] keeps. `iata_code`, `name`, `municipality`, `country_name`/`_iso`, `elevation`,
/// `latitude`/`longitude` are also present live
/// (`tests/fixtures/adsbdb/callsign_nominal.json`) but nothing downstream reads them; neither
/// does the sibling `response.flightroute.airline` object, which names the operator but is a
/// separate concern from a route.
#[derive(Debug, Deserialize)]
struct RawAirportRef {
    icao_code: Option<String>,
}

/// Reads `response.flightroute.{origin,destination}` out of a whole `/v0/callsign/{callsign}`
/// body. `None` for an absent/null `flightroute` (no route on file for this callsign); either
/// leg being missing or unparseable maps to that leg being `None` rather than a failed lookup —
/// half a route is still worth showing.
fn parse_route_response(
    raw: &Value,
    icao24: Icao24,
    callsign: &CallSign,
    now: UnixSeconds,
) -> Option<Flight> {
    let flightroute = raw.get("response")?.get("flightroute")?;
    if flightroute.is_null() {
        return None;
    }
    Some(Flight {
        icao24,
        callsign: Some(callsign.clone()),
        origin: airport_code(flightroute.get("origin")),
        destination: airport_code(flightroute.get("destination")),
        first_seen: now,
        last_seen: now,
    })
}

/// `value` is `flightroute.origin`/`.destination` itself (already looked up), or `None` when
/// that key was absent — both map to no ICAO code for that leg.
fn airport_code(value: Option<&Value>) -> Option<String> {
    let raw_ref: RawAirportRef = serde_json::from_value(value?.clone()).ok()?;
    non_empty(raw_ref.icao_code)
}

/// Trims and maps an empty string to `None`. adsbdb has not been observed live sending an
/// empty string in place of `null`, but nothing documents that it never will, and the cost of
/// guarding for it is one trim.
fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

/// Privacy rule 2.2's enrichment gate, in code — never call adsbdb for a target that is
/// anonymous. `app::enrichment` is the actual call site that enforces this end to end; this
/// is the gate itself, kept next to the adapter it protects and unit-tested in isolation
/// (mirrors `core::merge`'s sticky-anonymity tests, M1 item 1.4).
pub const fn should_enrich(anonymous: bool) -> bool {
    !anonymous
}

#[cfg(test)]
mod tests {
    use look_above_core::types::UnixSeconds;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::allowlist::{HostPolicy, is_authorized_host};
    use crate::http::REQUEST_TIMEOUT;

    const AIRCRAFT_NOMINAL: &str = include_str!("../tests/fixtures/adsbdb/aircraft_nominal.json");
    const AIRCRAFT_MALFORMED: &str =
        include_str!("../tests/fixtures/adsbdb/aircraft_malformed.json");
    const CALLSIGN_NOMINAL: &str = include_str!("../tests/fixtures/adsbdb/callsign_nominal.json");
    const CALLSIGN_MALFORMED: &str =
        include_str!("../tests/fixtures/adsbdb/callsign_malformed.json");

    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn source_against(server: &MockServer) -> AdsbdbSource {
        AdsbdbSource::build(client(), server.uri())
    }

    async fn mock_body(server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    async fn mock_status(server: &MockServer, status: u16) {
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status))
            .mount(server)
            .await;
    }

    fn hex(text: &str) -> Icao24 {
        Icao24::from_hex(text).expect("valid ICAO24 in test")
    }

    fn callsign(text: &str) -> CallSign {
        CallSign::new(text).expect("valid callsign in test")
    }

    // ---- Nominal parsing, via the fetch methods (end to end through a mock) -----------------

    #[tokio::test]
    async fn a_fetch_aircraft_parses_the_recorded_nominal_fixture() {
        let server = MockServer::start().await;
        mock_body(&server, AIRCRAFT_NOMINAL).await;
        let target = hex("a4b213");
        let now = UnixSeconds(1_721_000_000);

        let meta = source_against(&server)
            .fetch_aircraft(target, now)
            .await
            .expect("fetch succeeds")
            .expect("the fixture is a populated aircraft record");

        assert_eq!(meta.icao24, target);
        assert_eq!(meta.type_code.as_deref(), Some("SR22"));
        assert!(meta.registration.is_some());
        assert!(meta.operator.is_some());
        assert!(
            !meta.is_anonymous,
            "only non-anonymous targets ever reach this adapter"
        );
        assert_eq!(meta.fetched_at, Some(now));
        assert_eq!(meta.lookup_failed_at, None);
    }

    #[tokio::test]
    async fn a_fetch_route_parses_the_recorded_nominal_fixture() {
        let server = MockServer::start().await;
        mock_body(&server, CALLSIGN_NOMINAL).await;
        let cs = callsign("UAL123");
        let target = hex("a4b213");
        let now = UnixSeconds(1_721_000_000);

        let flight = source_against(&server)
            .fetch_route(&cs, target, now)
            .await
            .expect("fetch succeeds")
            .expect("the fixture carries a route");

        assert_eq!(flight.icao24, target);
        assert_eq!(flight.callsign, Some(cs));
        assert_eq!(flight.origin.as_deref(), Some("PANC"));
        assert_eq!(flight.destination.as_deref(), Some("KORD"));
        assert_eq!(flight.first_seen, now);
        assert_eq!(flight.last_seen, now);
    }

    // ---- Malformed/partial parsing, at the pure-function level (no HTTP involved) -----------

    #[test]
    fn parse_aircraft_response_skips_every_malformed_case_and_keeps_the_partial_good_one() {
        let cases: Vec<Value> =
            serde_json::from_str(AIRCRAFT_MALFORMED).expect("fixture is valid JSON");
        let target = hex("a4b213");
        let now = UnixSeconds(1_721_000_000);

        let results: Vec<Option<AircraftMeta>> = cases
            .iter()
            .map(|case| parse_aircraft_response(case, target, now))
            .collect();
        assert_eq!(
            results.iter().filter(|r| r.is_some()).count(),
            1,
            "only the well-formed case survives"
        );

        let meta = results
            .into_iter()
            .flatten()
            .next()
            .expect("the good case parsed");
        assert_eq!(meta.registration.as_deref(), Some("N401TT"));
        assert_eq!(
            meta.type_code, None,
            "the good case in the fixture omits icao_type on purpose, to prove a partial \
             record still parses"
        );
        assert_eq!(meta.operator, None);
    }

    #[test]
    fn parse_route_response_skips_every_malformed_case_and_keeps_the_partial_good_one() {
        let cases: Vec<Value> =
            serde_json::from_str(CALLSIGN_MALFORMED).expect("fixture is valid JSON");
        let target = hex("a4b213");
        let cs = callsign("UAL123");
        let now = UnixSeconds(1_721_000_000);

        let results: Vec<Option<Flight>> = cases
            .iter()
            .map(|case| parse_route_response(case, target, &cs, now))
            .collect();
        assert_eq!(
            results.iter().filter(|r| r.is_some()).count(),
            1,
            "only the well-formed case survives"
        );

        let flight = results
            .into_iter()
            .flatten()
            .next()
            .expect("the good case parsed");
        assert_eq!(flight.origin.as_deref(), Some("PANC"));
        assert_eq!(
            flight.destination, None,
            "the good case in the fixture omits destination on purpose, to prove half a \
             route still parses"
        );
    }

    // ---- The 404-is-not-an-error contract ----------------------------------------------------

    #[tokio::test]
    async fn a_404_from_the_aircraft_endpoint_is_not_found_not_an_error() {
        let server = MockServer::start().await;
        mock_status(&server, 404).await;

        let result = source_against(&server)
            .fetch_aircraft(hex("000000"), UnixSeconds(0))
            .await;
        assert_eq!(result, Ok(None));
    }

    #[tokio::test]
    async fn a_404_from_the_callsign_endpoint_is_not_found_not_an_error() {
        let server = MockServer::start().await;
        mock_status(&server, 404).await;

        let result = source_against(&server)
            .fetch_route(&callsign("ZZZ9999"), hex("000000"), UnixSeconds(0))
            .await;
        assert_eq!(result, Ok(None));
    }

    #[tokio::test]
    async fn a_non_404_error_still_propagates_from_fetch_aircraft() {
        let server = MockServer::start().await;
        mock_status(&server, 500).await;

        let error = source_against(&server)
            .fetch_aircraft(hex("a4b213"), UnixSeconds(0))
            .await
            .expect_err("a 500 must not be swallowed like a 404");
        assert_eq!(error, SourceError::Server { status: 500 });
    }

    #[tokio::test]
    async fn a_non_404_error_still_propagates_from_fetch_route() {
        let server = MockServer::start().await;
        mock_status(&server, 500).await;

        let error = source_against(&server)
            .fetch_route(&callsign("UAL123"), hex("a4b213"), UnixSeconds(0))
            .await
            .expect_err("a 500 must not be swallowed like a 404");
        assert_eq!(error, SourceError::Server { status: 500 });
    }

    // ---- The allowlist ------------------------------------------------------------------------

    #[test]
    fn the_adsbdb_base_url_is_the_documented_one_and_is_authorized() {
        assert_eq!(BASE_URL, "https://api.adsbdb.com/v0");
        let host = reqwest::Url::parse(BASE_URL)
            .expect("the base url parses")
            .host_str()
            .expect("the base url has a host")
            .to_owned();
        assert!(is_authorized_host(&host), "{host} must be on the allowlist");
    }

    // ---- The privacy-rule-2.2 gate ------------------------------------------------------------

    #[test]
    fn should_enrich_refuses_an_anonymous_target() {
        assert!(!should_enrich(true));
    }

    #[test]
    fn should_enrich_permits_a_non_anonymous_target() {
        assert!(should_enrich(false));
    }

    // ---- The real api.adsbdb.com --------------------------------------------------------------

    /// The one test that fetches a real aircraft record. Keyless and free, but be gentle:
    ///
    /// ```text
    /// cargo test -p look-above-ingest -- --ignored live_adsbdb_aircraft
    /// ```
    ///
    /// Nothing here prints a payload — only field presence (docs/06).
    #[tokio::test]
    #[ignore = "hits the real api.adsbdb.com API; keyless and free, but be gentle"]
    async fn live_adsbdb_aircraft_matches_the_documented_shape() {
        let source = AdsbdbSource::new(HttpClient::new().expect("client builds"));
        let target = hex("a4b213");
        let meta = source
            .fetch_aircraft(target, UnixSeconds(0))
            .await
            .expect("api.adsbdb.com answers")
            .expect("a4b213 is a real, registered aircraft");

        assert_eq!(meta.icao24, target);
        eprintln!(
            "live adsbdb aircraft: has_type_code={} has_registration={} has_operator={}",
            meta.type_code.is_some(),
            meta.registration.is_some(),
            meta.operator.is_some()
        );
    }

    /// The one test that fetches a real route. Keyless and free, but be gentle:
    ///
    /// ```text
    /// cargo test -p look-above-ingest -- --ignored live_adsbdb_route
    /// ```
    #[tokio::test]
    #[ignore = "hits the real api.adsbdb.com API; keyless and free, but be gentle"]
    async fn live_adsbdb_route_matches_the_documented_shape() {
        let source = AdsbdbSource::new(HttpClient::new().expect("client builds"));
        let cs = callsign("UAL123");
        let target = hex("a4b213");
        let flight = source
            .fetch_route(&cs, target, UnixSeconds(0))
            .await
            .expect("api.adsbdb.com answers")
            .expect("UAL123 has a route on file");

        assert_eq!(flight.icao24, target);
        eprintln!(
            "live adsbdb route: has_origin={} has_destination={}",
            flight.origin.is_some(),
            flight.destination.is_some()
        );
    }
}
