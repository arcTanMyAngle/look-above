//! `record-fixture` — the sanctioned recorder for adapter test fixtures (M1 item 1.10).
//!
//! docs/06 permits exactly two live fetches during development: running the app, and *this*
//! tool. Everything the parser tests read from `crates/ingest/tests/fixtures/` is meant to be
//! recorded here — fetched from an authorized source, trimmed to a handful of records,
//! credential-scrubbed (privacy rule 7.2), and written to a file — **without the payload ever
//! reaching a log or this transcript** (docs/06 network rule). Until now the fixtures have
//! been hand-written to each source's documented shape because this tool did not exist; their
//! READMEs each promise "re-record once item 1.10 lands", and this is that path.
//!
//! It is a bin of the `ingest` crate, not a standalone program, on purpose: a recording must
//! go out exactly as a poll would, so it reuses this crate's allowlist-enforcing
//! [`HttpClient`], the `OpenSky` `OAuth2` client, and the source endpoint constants rather
//! than reimplementing any of them. What it records is the *nominal* case — a real, populated
//! response. The awkward cases each source's fixtures also cover (empty region, nulls in every
//! optional field, a malformed record mid-array) are deliberately hand-authored and stay so:
//! they arrive live only when they happen to, and the point of having them is that they are
//! always present.
//!
//! Usage (region parameters are each source's own native shape):
//!
//! ```text
//! # OpenSky: a bbox (lamin lomin lamax lomax). Spends 1–4 credits; needs credentials in
//! # LOOK_ABOVE_OPENSKY_CLIENT_ID / _SECRET (the env rung of privacy rule 7.1).
//! cargo run -p look-above-ingest --bin record-fixture -- opensky 46 7 48 9 states_nominal
//!
//! # airplanes.live / adsb.lol: a point and radius in nm (≤ 250). Keyless and free.
//! cargo run -p look-above-ingest --bin record-fixture -- airplaneslive 47 8 73 point_nominal
//! cargo run -p look-above-ingest --bin record-fixture -- adsblol       47 8 73 point_nominal
//!
//! # aviationweather.gov: a comma-separated station id list. Keyless and free (NOAA).
//! cargo run -p look-above-ingest --bin record-fixture -- aviationweather KJFK,KLAX,KORD metar_nominal
//! ```
//!
//! The file lands in `crates/ingest/tests/fixtures/<source>/<name>.json`. Re-record a
//! hand-written fixture and its live test (`live_*_match_the_documented_shape`) is what
//! confirms the new bytes still match what the parser believes.

use std::error::Error;
use std::path::{Path, PathBuf};

use look_above_core::secret::SecretString;
use look_above_ingest::http::{HttpClient, send_json};
use look_above_ingest::opensky::states::STATES_ENDPOINT;
use look_above_ingest::opensky::{Credentials, OpenSkyAuth};
use look_above_ingest::point::MAX_RADIUS_NM;
use look_above_ingest::{adsb_lol, airplanes_live};
use serde_json::Value;

/// The trim ceiling docs/10 §2 sets: fixtures carry a handful of records, never a live crowd.
const MAX_FIXTURE_RECORDS: usize = 20;

/// The environment variables `OpenSky` credentials are read from — the highest-precedence
/// rung of privacy rule 7.1. This tool reads only that rung: it cannot reach `app`'s
/// `config.toml`/`credentials.json` loader (that would invert the crate dependency direction),
/// and a manual tool run by the account owner can set two env vars.
const OPENSKY_ID_VAR: &str = "LOOK_ABOVE_OPENSKY_CLIENT_ID";
const OPENSKY_SECRET_VAR: &str = "LOOK_ABOVE_OPENSKY_CLIENT_SECRET";

/// Object keys scrubbed from a recording before it is written (privacy rule 7.2).
///
/// The authorized responses recorded here carry no credential or account material today — the
/// readsb feeds are anonymous and `OpenSky`'s `/states/all` body is public aircraft data — so
/// this denylist normally removes nothing. It exists so the *tool* is safe the day a source,
/// or a future endpoint, does echo an account field: a recording is never trusted to be clean
/// by inspection, because inspection means reading the payload, which docs/06 forbids.
const SCRUBBED_KEYS: &[&str] = &[
    "access_token",
    "refresh_token",
    "token",
    "api_key",
    "apikey",
    "client_id",
    "client_secret",
    "secret",
    "password",
    "authorization",
    "session",
    "sessionid",
    "user",
    "username",
    "email",
    "account",
];

/// Which source a recording targets — the CLI's first argument.
#[derive(Debug, Clone, Copy)]
enum Source {
    OpenSky,
    AirplanesLive,
    AdsbLol,
    AviationWeather,
}

impl Source {
    /// The fixture subdirectory, matching the existing layout.
    fn dir(self) -> &'static str {
        match self {
            Source::OpenSky => "opensky",
            Source::AirplanesLive => "airplaneslive",
            Source::AdsbLol => "adsblol",
            Source::AviationWeather => "aviationweather",
        }
    }

    /// The top-level array holding the per-record array — `states` for `OpenSky`'s positional
    /// arrays, `ac` for the readsb feeds. `None` means the response body *is* the array
    /// (`aviationweather.gov`'s METAR endpoint returns a bare JSON array, not an object).
    fn records_key(self) -> Option<&'static str> {
        match self {
            Source::OpenSky => Some("states"),
            Source::AirplanesLive | Source::AdsbLol => Some("ac"),
            Source::AviationWeather => None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (source, body) = match args.first().map(String::as_str) {
        Some("opensky") => (Source::OpenSky, fetch_opensky(&args).await?),
        Some("airplaneslive") => (
            Source::AirplanesLive,
            fetch_point(airplanes_live::POINT_ENDPOINT, &args).await?,
        ),
        Some("adsblol") => (
            Source::AdsbLol,
            fetch_point(adsb_lol::POINT_ENDPOINT, &args).await?,
        ),
        Some("aviationweather") => (Source::AviationWeather, fetch_metar(&args).await?),
        _ => return Err(usage().into()),
    };

    // Trim, then scrub — trimming first keeps the scrub off the discarded records.
    let name = out_name(&args)?;
    let prepared = prepare(body, source.records_key());
    let kept = records_len(&prepared, source.records_key());
    let path = write_fixture(source, name, &prepared)?;

    // Counts and a path only — never the payload (docs/06).
    println!("recorded {kept} record(s) to {}", path.display());
    Ok(())
}

/// `<lamin> <lomin> <lamax> <lomax> <name>`, `OpenSky`'s own bbox shape, sent exactly as the
/// adapter sends it (bearer token in a header, never the URL — privacy rule 7.1a).
async fn fetch_opensky(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let bounds = bbox_args(args)?;

    let (id, secret) = opensky_credentials()?;
    let auth = OpenSkyAuth::new(
        HttpClient::new()?,
        Credentials::new(SecretString::from(id), SecretString::from(secret)),
    );
    let token = auth
        .token()
        .await?
        .ok_or("OpenSky reports no credentials — set the two env vars")?;

    // OpenSky's own lamin/lomin/lamax/lomax parameter names, in that order, exactly as
    // `opensky::states` sends them; the bearer token rides in a header, never the URL (7.1a).
    let request = HttpClient::new()?
        .get(STATES_ENDPOINT)?
        .query(&[
            ("lamin", bounds[0]),
            ("lomin", bounds[1]),
            ("lamax", bounds[2]),
            ("lomax", bounds[3]),
        ])
        .bearer_auth(token.expose());
    Ok(send_json(request).await?)
}

/// `<lat> <lon> <radius_nm> <name>`, the readsb feeds' own `/point/{lat}/{lon}/{radius}`
/// shape. Keyless; the shared allowlist and User-Agent still apply through [`HttpClient`].
async fn fetch_point(endpoint: &str, args: &[String]) -> Result<Value, Box<dyn Error>> {
    let lat: f64 = arg(args, 1, "lat")?;
    let lon: f64 = arg(args, 2, "lon")?;
    let radius: u32 = arg(args, 3, "radius_nm")?;
    if radius == 0 || radius > MAX_RADIUS_NM {
        return Err(format!("radius_nm must be 1..={MAX_RADIUS_NM}, got {radius}").into());
    }

    let url = format!("{endpoint}/{lat}/{lon}/{radius}");
    Ok(send_json(HttpClient::new()?.get(&url)?).await?)
}

/// `<station,station,...> <name>`, `aviationweather.gov`'s own comma-separated `ids` shape
/// (`look_above_ingest::metar::MetarSource` sends the same query, ≤ 100 ids per request).
async fn fetch_metar(args: &[String]) -> Result<Value, Box<dyn Error>> {
    let ids = args.get(1).ok_or_else(usage)?;
    let request = HttpClient::new()?
        .get(look_above_ingest::metar::METAR_ENDPOINT)?
        .query(&[("ids", ids.as_str()), ("format", "json")]);
    Ok(send_json(request).await?)
}

/// Trim the record array to [`MAX_FIXTURE_RECORDS`], then scrub credential-shaped keys.
fn prepare(mut body: Value, records_key: Option<&str>) -> Value {
    match records_key {
        Some(key) => {
            if let Some(Value::Array(records)) = body.get_mut(key) {
                records.truncate(MAX_FIXTURE_RECORDS);
            }
        }
        None => {
            if let Value::Array(records) = &mut body {
                records.truncate(MAX_FIXTURE_RECORDS);
            }
        }
    }
    scrub(&mut body);
    body
}

/// Recursively remove any object key in [`SCRUBBED_KEYS`] (case-insensitively).
fn scrub(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !SCRUBBED_KEYS.contains(&key.to_ascii_lowercase().as_str()));
            for child in map.values_mut() {
                scrub(child);
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                scrub(child);
            }
        }
        _ => {}
    }
}

/// How many records the array under `key` holds (or, for `key: None`, the root array itself) —
/// `0` when it is absent, `null`, or not an array, never an error.
fn records_len(body: &Value, key: Option<&str>) -> usize {
    match key {
        Some(key) => body.get(key).and_then(Value::as_array).map_or(0, Vec::len),
        None => body.as_array().map_or(0, Vec::len),
    }
}

/// Pretty-print `body` to `crates/ingest/tests/fixtures/<source>/<name>.json`.
///
/// Anchored to `CARGO_MANIFEST_DIR` so the destination does not depend on the caller's working
/// directory. `name` is rejected if it could escape that directory.
fn write_fixture(source: Source, name: &str, body: &Value) -> Result<PathBuf, Box<dyn Error>> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("fixture name {name:?} must be a bare file stem").into());
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(source.dir());
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(format!("{name}.json"));
    let mut json = serde_json::to_string_pretty(body)?;
    json.push('\n');
    std::fs::write(&path, json)?;
    Ok(path)
}

/// The four bbox bounds, parsed and positioned as `[lamin, lomin, lamax, lomax]`.
fn bbox_args(args: &[String]) -> Result<[f64; 4], Box<dyn Error>> {
    Ok([
        arg(args, 1, "lamin")?,
        arg(args, 2, "lomin")?,
        arg(args, 3, "lamax")?,
        arg(args, 4, "lomax")?,
    ])
}

/// The output name is always the argument after the region parameters — index 5 for the bbox
/// source, index 4 for the point sources, index 2 for the METAR source (just a station list).
fn out_name(args: &[String]) -> Result<&str, Box<dyn Error>> {
    let index = match args.first().map(String::as_str) {
        Some("opensky") => 5,
        Some("aviationweather") => 2,
        _ => 4,
    };
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage().into())
}

/// Parse positional argument `index` as `T`, naming it on failure.
fn arg<T>(args: &[String], index: usize, name: &str) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = args
        .get(index)
        .ok_or_else(|| format!("missing argument <{name}>\n{}", usage()))?;
    raw.parse::<T>()
        .map_err(|error| format!("argument <{name}> = {raw:?} is not valid: {error}").into())
}

/// Reads the `OpenSky` credential pair from the environment, or explains what is missing.
fn opensky_credentials() -> Result<(String, String), Box<dyn Error>> {
    match (
        std::env::var(OPENSKY_ID_VAR),
        std::env::var(OPENSKY_SECRET_VAR),
    ) {
        (Ok(id), Ok(secret)) => Ok((id, secret)),
        _ => Err(format!(
            "recording from OpenSky needs {OPENSKY_ID_VAR} and {OPENSKY_SECRET_VAR} set"
        )
        .into()),
    }
}

fn usage() -> String {
    "usage:\n  \
     record-fixture opensky <lamin> <lomin> <lamax> <lomax> <name>\n  \
     record-fixture airplaneslive <lat> <lon> <radius_nm> <name>\n  \
     record-fixture adsblol <lat> <lon> <radius_nm> <name>\n  \
     record-fixture aviationweather <station,station,...> <name>"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ---- Trimming ---------------------------------------------------------------------------

    /// docs/10 §2: a recording carries at most [`MAX_FIXTURE_RECORDS`], so a busy live sky is
    /// cut down. Top-level fields around the array are left intact — `now` is what readsb
    /// records are timestamped against.
    #[test]
    fn a_long_record_list_is_trimmed_to_the_ceiling() {
        let records: Vec<Value> = (0..50)
            .map(|i| json!({ "hex": format!("3c64{i:02}"), "lat": 50.0, "lon": 8.0 }))
            .collect();
        let body = json!({ "now": 1_721_000_000_000_i64, "total": 50, "ac": records });

        let prepared = prepare(body, Some("ac"));
        assert_eq!(records_len(&prepared, Some("ac")), MAX_FIXTURE_RECORDS);
        assert_eq!(
            prepared.get("now").and_then(Value::as_i64),
            Some(1_721_000_000_000),
            "the timestamp the records depend on must survive the trim"
        );
    }

    /// A list already within the ceiling is untouched — re-recording a small fixture must not
    /// silently drop records.
    #[test]
    fn a_short_record_list_is_left_alone() {
        let body = json!({ "states": [["3c6444"], ["4ca7b6"]] });
        let prepared = prepare(body, Some("states"));
        assert_eq!(records_len(&prepared, Some("states")), 2);
    }

    /// An empty region: `OpenSky` sends `states: null`, and that is neither a crash nor a
    /// record — `records_len` reports zero and the file still writes.
    #[test]
    fn a_null_record_array_is_zero_records_not_an_error() {
        let body = json!({ "time": 1_721_000_000, "states": Value::Null });
        let prepared = prepare(body, Some("states"));
        assert_eq!(records_len(&prepared, Some("states")), 0);
    }

    /// `aviationweather.gov`'s METAR endpoint returns a bare array, not an object — `records_key:
    /// None` means "the root is the array", and trimming/counting must work against it directly.
    #[test]
    fn a_root_level_array_is_trimmed_and_counted_with_no_records_key() {
        let records: Vec<Value> = (0..50)
            .map(|i| json!({ "icaoId": format!("K{i:03}") }))
            .collect();
        let body = Value::Array(records);

        let prepared = prepare(body, None);
        assert_eq!(records_len(&prepared, None), MAX_FIXTURE_RECORDS);
    }

    // ---- Scrubbing --------------------------------------------------------------------------

    /// Privacy rule 7.2: any credential- or account-shaped key is removed, at any depth and
    /// regardless of case, while public aircraft fields are preserved untouched.
    #[test]
    fn credential_shaped_keys_are_scrubbed_at_every_depth() {
        let mut body = json!({
            "Access_Token": "secret-abc",
            "now": 1_721_000_000_000_i64,
            "ac": [
                {
                    "hex": "3c6444",
                    "flight": "DLH9LF  ",
                    "lat": 50.0,
                    "lon": 8.0,
                    "session": "should-vanish",
                    "meta": { "api_key": "nested-secret", "gs": 450.0 }
                }
            ]
        });
        scrub(&mut body);

        // The account-shaped keys are gone, top-level and nested.
        assert!(body.get("Access_Token").is_none(), "case-insensitive scrub");
        let record = &body["ac"][0];
        assert!(record.get("session").is_none());
        assert!(record["meta"].get("api_key").is_none());

        // Public aircraft data is untouched.
        assert_eq!(record["hex"], json!("3c6444"));
        assert_eq!(record["flight"], json!("DLH9LF  "));
        assert_eq!(record["meta"]["gs"], json!(450.0));
        assert_eq!(body["now"], json!(1_721_000_000_000_i64));
    }

    /// A real readsb-shaped body has nothing to scrub — the common case must be a no-op, not a
    /// mangling.
    #[test]
    fn an_ordinary_readsb_body_is_unchanged_by_scrubbing() {
        let original = json!({
            "now": 1_721_000_000_000_i64,
            "ac": [{ "hex": "3c6444", "flight": "DLH9LF  ", "lat": 50.0, "lon": 8.0, "gs": 450.0 }]
        });
        let mut scrubbed = original.clone();
        scrub(&mut scrubbed);
        assert_eq!(scrubbed, original);
    }

    // ---- Naming and layout ------------------------------------------------------------------

    #[test]
    fn each_source_maps_to_its_fixture_dir_and_record_key() {
        assert_eq!(Source::OpenSky.dir(), "opensky");
        assert_eq!(Source::OpenSky.records_key(), Some("states"));
        assert_eq!(Source::AirplanesLive.dir(), "airplaneslive");
        assert_eq!(Source::AdsbLol.dir(), "adsblol");
        assert_eq!(Source::AirplanesLive.records_key(), Some("ac"));
        assert_eq!(Source::AdsbLol.records_key(), Some("ac"));
        assert_eq!(Source::AviationWeather.dir(), "aviationweather");
        assert_eq!(Source::AviationWeather.records_key(), None);
    }

    /// A name that could escape the fixtures directory is refused before any write.
    #[test]
    fn an_unsafe_fixture_name_is_refused() {
        let body = json!({ "ac": [] });
        for bad in ["../evil", "sub/dir", "back\\slash", ""] {
            assert!(
                write_fixture(Source::AdsbLol, bad, &body).is_err(),
                "name {bad:?} must be refused"
            );
        }
    }

    /// The output-name index tracks the region arity: the bbox source puts it fifth, the point
    /// sources fourth, and the METAR source (just a station list, no lat/lon/radius) second.
    #[test]
    fn the_output_name_follows_the_region_arguments() {
        let opensky = strings(&["opensky", "46", "7", "48", "9", "states_nominal"]);
        assert_eq!(out_name(&opensky).expect("named"), "states_nominal");

        let point = strings(&["adsblol", "47", "8", "73", "point_nominal"]);
        assert_eq!(out_name(&point).expect("named"), "point_nominal");

        let metar = strings(&["aviationweather", "KJFK,KLAX", "metar_nominal"]);
        assert_eq!(out_name(&metar).expect("named"), "metar_nominal");

        let missing = strings(&["adsblol", "47", "8", "73"]);
        assert!(
            out_name(&missing).is_err(),
            "a missing name is a usage error"
        );
    }

    /// The bbox bounds parse in `OpenSky`'s lamin/lomin/lamax/lomax order; a non-number names
    /// the offending argument.
    #[test]
    fn bbox_arguments_parse_in_order_and_report_bad_input() {
        let args = strings(&["opensky", "46", "7", "48", "9", "states_nominal"]);
        assert_eq!(bbox_args(&args).expect("parses"), [46.0, 7.0, 48.0, 9.0]);

        let bad = strings(&["opensky", "46", "east", "48", "9", "states_nominal"]);
        let error = bbox_args(&bad).expect_err("not a number").to_string();
        assert!(error.contains("lomin"), "names the argument: {error}");
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }
}
