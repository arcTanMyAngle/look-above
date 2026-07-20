//! `import-ourairports` — M3 item 3.1: `OurAirports`' `airports.csv`/`runways.csv`, trimmed to
//! exactly the columns docs/08's `airports`/`runways` tables store and bundled into
//! `crates/store/assets/ourairports/`, the same no-runtime-fetch shape `import-basemap` (M2
//! item 2.2a) already established: `store` has no network dependencies and must stay that way
//! (M0 acceptance line 3; ADR-002's CPU/GPU split doesn't put HTTP in `store`'s lane either).
//!
//! `OurAirports`' `type` column is mapped onto `core::contracts::AirportSize`
//! (`AirportSize::from_ourairports_type`); rows whose type falls outside that ladder
//! (`seaplane_base`, `balloonport`, `closed`, and anything undocumented) are dropped here — the
//! M3 decision `AirportSize`'s own doc comment anticipated ("recorded when the importer
//! lands"). Runways whose airport was dropped are filtered out too, so the bundled snapshot
//! never seeds a `runways` row with no matching `airports` row.
//!
//! Column handling mirrors the storage-agent's import craft standard: parsing is by CSV header
//! name (not position), so upstream column *reordering* is transparent and genuinely unknown
//! *extra* columns are reported (not silently dropped without a trace); a genuinely required
//! column that goes missing fails the whole import cleanly rather than seeding partial data.
//!
//! Usage: `cargo run -p look-above-import --bin import-ourairports`. No arguments, no
//! credentials. Re-run when `OurAirports`' snapshot should be refreshed (the sources skill says
//! monthly/quarterly, not per-run — this is static-download tooling, never a runtime poller).

use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};

use look_above_core::contracts::AirportSize;
use serde::{Deserialize, Serialize};

/// `OurAirports`' static CSV host — no key, no runtime polling (sources skill: "Static
/// downloads (import scripts, not runtime polling)").
const ALLOWED_STATIC_HOSTS: &[&str] = &["davidmegginson.github.io"];

const AIRPORTS_URL: &str = "https://davidmegginson.github.io/ourairports-data/airports.csv";
const RUNWAYS_URL: &str = "https://davidmegginson.github.io/ourairports-data/runways.csv";

/// Coordinates are rounded to this many places (1e-4 degree, ~11 m at the equator) before
/// writing — plenty for a point marker or a runway centerline, mirrors `import_basemap.rs`'s
/// `COORD_PRECISION`.
const COORD_PRECISION: f64 = 10_000.0;

/// Header columns `airports.csv` is documented to carry today (verified live). A header
/// present in the fetched file but absent from this list is unknown to this importer and gets
/// a warning, not a silent drop (craft standard: "unknown columns ignored with a warning").
const KNOWN_AIRPORT_COLUMNS: &[&str] = &[
    "id",
    "ident",
    "type",
    "name",
    "latitude_deg",
    "longitude_deg",
    "elevation_ft",
    "continent",
    "iso_country",
    "iso_region",
    "municipality",
    "scheduled_service",
    "icao_code",
    "iata_code",
    "gps_code",
    "local_code",
    "home_link",
    "wikipedia_link",
    "keywords",
];

/// Header columns `runways.csv` is documented to carry today (verified live).
const KNOWN_RUNWAY_COLUMNS: &[&str] = &[
    "id",
    "airport_ref",
    "airport_ident",
    "length_ft",
    "width_ft",
    "surface",
    "lighted",
    "closed",
    "le_ident",
    "le_latitude_deg",
    "le_longitude_deg",
    "le_elevation_ft",
    "le_heading_degT",
    "le_displaced_threshold_ft",
    "he_ident",
    "he_latitude_deg",
    "he_longitude_deg",
    "he_elevation_ft",
    "he_heading_degT",
    "he_displaced_threshold_ft",
];

/// One upstream `airports.csv` row. `csv`'s Serde support matches struct fields to CSV headers
/// by name (not position) when headers are present, so upstream column reordering or added
/// columns are transparent; `#[serde(default)]` on the non-essential fields additionally
/// tolerates a column vanishing entirely, rather than failing the whole import over a field
/// this importer does not strictly need.
#[derive(Debug, Deserialize)]
struct SourceAirport {
    ident: String,
    #[serde(rename = "type")]
    kind: String,
    name: String,
    latitude_deg: f64,
    longitude_deg: f64,
    #[serde(default)]
    elevation_ft: Option<i32>,
    #[serde(default)]
    iso_country: Option<String>,
    #[serde(default)]
    iata_code: Option<String>,
}

/// One upstream `runways.csv` row.
#[derive(Debug, Deserialize)]
struct SourceRunway {
    airport_ident: String,
    #[serde(default)]
    le_ident: Option<String>,
    #[serde(default)]
    le_latitude_deg: Option<f64>,
    #[serde(default)]
    le_longitude_deg: Option<f64>,
    #[serde(default, rename = "le_heading_degT")]
    le_heading_deg_t: Option<f64>,
    #[serde(default)]
    he_ident: Option<String>,
    #[serde(default)]
    he_latitude_deg: Option<f64>,
    #[serde(default)]
    he_longitude_deg: Option<f64>,
    #[serde(default, rename = "he_heading_degT")]
    he_heading_deg_t: Option<f64>,
    #[serde(default)]
    length_ft: Option<i32>,
    #[serde(default)]
    width_ft: Option<i32>,
    #[serde(default)]
    surface: Option<String>,
    closed: i64,
}

/// One row of the bundled `airports.csv` asset — column-for-column with the `airports` table
/// (docs/08), read back by `crates/store`'s seed step.
#[derive(Debug, Serialize, PartialEq)]
struct BundledAirport {
    ident: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    lat: f64,
    lon: f64,
    elevation_ft: Option<i32>,
    iso_country: Option<String>,
    iata: Option<String>,
}

/// One row of the bundled `runways.csv` asset — column-for-column with the `runways` table.
#[derive(Debug, Serialize, PartialEq)]
struct BundledRunway {
    airport_ident: String,
    le_ident: Option<String>,
    le_lat: Option<f64>,
    le_lon: Option<f64>,
    le_heading_deg: Option<f64>,
    he_ident: Option<String>,
    he_lat: Option<f64>,
    he_lon: Option<f64>,
    he_heading_deg: Option<f64>,
    length_ft: Option<i32>,
    width_ft: Option<i32>,
    surface: Option<String>,
    closed: i64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let airports_csv = fetch_csv(AIRPORTS_URL).await?;
    warn_unknown_columns(&airports_csv, KNOWN_AIRPORT_COLUMNS, "airports.csv")?;
    let (kept_airports, kept_idents, source_airport_count) = convert_airports(&airports_csv)?;

    let runways_csv = fetch_csv(RUNWAYS_URL).await?;
    warn_unknown_columns(&runways_csv, KNOWN_RUNWAY_COLUMNS, "runways.csv")?;
    let (kept_runways, source_runway_count) = convert_runways(&runways_csv, &kept_idents)?;

    let out_dir = ourairports_dir();
    std::fs::create_dir_all(&out_dir)?;
    let airports_path = write_csv(&out_dir.join("airports.csv"), &kept_airports)?;
    let runways_path = write_csv(&out_dir.join("runways.csv"), &kept_runways)?;

    // Counts and paths only — never rows (docs/06's "no raw payload in context" rule applies
    // just as much to an 80k-row CSV as to an API response).
    println!(
        "airports: {source_airport_count} source row(s) -> {} kept (large/medium/small/heliport, \
         closed/seaplane_base/balloonport dropped) -> {}",
        kept_airports.len(),
        airports_path.display()
    );
    println!(
        "runways: {source_runway_count} source row(s) -> {} kept (airport retained above) -> {}",
        kept_runways.len(),
        runways_path.display()
    );
    Ok(())
}

/// `crates/store/assets/ourairports`, anchored to this crate's manifest rather than the
/// caller's working directory (mirrors `import_basemap.rs`'s `basemap_dir()`).
fn ourairports_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("store")
        .join("assets")
        .join("ourairports")
}

/// Refuses anything not on [`ALLOWED_STATIC_HOSTS`] over plain https, before a request is sent
/// — the same gate `import_basemap.rs` enforces, kept as its own copy per file rather than
/// shared, since each importer's allowlist is deliberately scoped to exactly the one host it
/// needs.
fn assert_authorized(url: &str) -> Result<(), Box<dyn Error>> {
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" {
        return Err(format!("refusing non-https static download: {url}").into());
    }
    match parsed.host_str() {
        Some(host) if ALLOWED_STATIC_HOSTS.contains(&host) => Ok(()),
        Some(host) => {
            Err(format!("{host} is not on ALLOWED_STATIC_HOSTS — add it deliberately").into())
        }
        None => Err(format!("{url} has no host").into()),
    }
}

/// Downloads `url` (checked against [`ALLOWED_STATIC_HOSTS`] first) as text.
async fn fetch_csv(url: &str) -> Result<String, Box<dyn Error>> {
    assert_authorized(url)?;
    let text = reqwest::get(url).await?.error_for_status()?.text().await?;
    Ok(text)
}

/// Prints a warning (not an error — the craft standard is "ignored with a warning", and this
/// importer only reads columns it names explicitly) for every CSV header not in `known`.
fn warn_unknown_columns(
    csv_text: &str,
    known: &[&str],
    file_label: &str,
) -> Result<(), Box<dyn Error>> {
    let mut reader = csv::Reader::from_reader(csv_text.as_bytes());
    let headers = reader.headers()?.clone();
    for header in &headers {
        if !known.contains(&header) {
            eprintln!("warning: {file_label} has an unrecognized column {header:?} (ignored)");
        }
    }
    Ok(())
}

/// Rounds one coordinate to [`COORD_PRECISION`].
fn round_coord(value: f64) -> f64 {
    (value * COORD_PRECISION).round() / COORD_PRECISION
}

/// Kept rows, the set of idents kept (so `convert_runways` can drop orphans), and the total
/// source row count (for the console summary).
type ConvertedAirports = (Vec<BundledAirport>, HashSet<String>, usize);

/// Parses `airports.csv`, drops any row whose `type` falls outside [`AirportSize`]'s ladder,
/// and returns the kept rows, the set of idents kept (so `convert_runways` can drop orphans),
/// and the total source row count (for the console summary).
fn convert_airports(csv_text: &str) -> Result<ConvertedAirports, Box<dyn Error>> {
    let mut reader = csv::Reader::from_reader(csv_text.as_bytes());
    let mut kept = Vec::new();
    let mut kept_idents = HashSet::new();
    let mut source_count = 0usize;

    for result in reader.deserialize::<SourceAirport>() {
        let row = result?;
        source_count += 1;
        if AirportSize::from_ourairports_type(&row.kind).is_none() {
            // seaplane_base/balloonport/closed/unknown — dropped per the M3 mapping decision.
            continue;
        }
        kept_idents.insert(row.ident.clone());
        kept.push(BundledAirport {
            ident: row.ident,
            name: row.name,
            kind: row.kind,
            lat: round_coord(row.latitude_deg),
            lon: round_coord(row.longitude_deg),
            elevation_ft: row.elevation_ft,
            iso_country: row.iso_country,
            iata: row.iata_code,
        });
    }
    Ok((kept, kept_idents, source_count))
}

/// Parses `runways.csv`, keeping only rows whose `airport_ident` survived
/// [`convert_airports`]'s type-drop — otherwise the bundled `runways` snapshot would seed rows
/// with no matching `airports` row.
fn convert_runways(
    csv_text: &str,
    kept_airport_idents: &HashSet<String>,
) -> Result<(Vec<BundledRunway>, usize), Box<dyn Error>> {
    let mut reader = csv::Reader::from_reader(csv_text.as_bytes());
    let mut kept = Vec::new();
    let mut source_count = 0usize;

    for result in reader.deserialize::<SourceRunway>() {
        let row = result?;
        source_count += 1;
        if !kept_airport_idents.contains(&row.airport_ident) {
            continue;
        }
        kept.push(BundledRunway {
            airport_ident: row.airport_ident,
            le_ident: row.le_ident,
            le_lat: row.le_latitude_deg.map(round_coord),
            le_lon: row.le_longitude_deg.map(round_coord),
            le_heading_deg: row.le_heading_deg_t,
            he_ident: row.he_ident,
            he_lat: row.he_latitude_deg.map(round_coord),
            he_lon: row.he_longitude_deg.map(round_coord),
            he_heading_deg: row.he_heading_deg_t,
            length_ft: row.length_ft,
            width_ft: row.width_ft,
            surface: row.surface,
            closed: row.closed,
        });
    }
    Ok((kept, source_count))
}

fn write_csv<T: Serialize>(path: &Path, rows: &[T]) -> Result<PathBuf, Box<dyn Error>> {
    let mut writer = csv::Writer::from_path(path)?;
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Host gate ----------------------------------------------------------------------

    #[test]
    fn the_real_download_host_is_authorized_over_https() {
        assert!(assert_authorized(AIRPORTS_URL).is_ok());
        assert!(assert_authorized(RUNWAYS_URL).is_ok());
    }

    #[test]
    fn an_unlisted_host_is_refused() {
        let error = assert_authorized("https://evil-davidmegginson.github.io/x.csv")
            .expect_err("lookalike host must be refused")
            .to_string();
        assert!(error.contains("evil-davidmegginson.github.io"), "{error}");
    }

    #[test]
    fn plain_http_is_refused_even_for_an_authorized_host() {
        assert!(
            assert_authorized("http://davidmegginson.github.io/ourairports-data/airports.csv")
                .is_err()
        );
    }

    // ---- Coordinate rounding --------------------------------------------------------------

    #[test]
    fn coordinates_round_to_the_configured_precision() {
        assert!((round_coord(12.345_678_9) - 12.3457).abs() < 1e-9);
        assert!((round_coord(-8.000_04) - (-8.0)).abs() < 1e-9);
    }

    // ---- Airport conversion / type-drop -----------------------------------------------------

    #[test]
    fn kept_airport_types_survive_conversion_with_rounded_coordinates() {
        let csv = "ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,iata_code\n\
                    KJFK,large_airport,John F Kennedy Intl,40.639801,-73.778900,13,US,JFK\n";
        let (kept, idents, source_count) = convert_airports(csv).expect("converts");
        assert_eq!(source_count, 1);
        assert_eq!(kept.len(), 1);
        assert!(idents.contains("KJFK"));
        assert_eq!(kept[0].ident, "KJFK");
        assert_eq!(kept[0].kind, "large_airport");
        assert!((kept[0].lat - 40.6398).abs() < 1e-9);
        assert!((kept[0].lon - (-73.7789)).abs() < 1e-9);
    }

    #[test]
    fn dropped_airport_types_are_excluded_but_still_counted_as_source_rows() {
        let csv = "ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,iata_code\n\
                    00XX,seaplane_base,Some Seaplane Base,10.0,20.0,,US,\n\
                    00YY,balloonport,Some Balloonport,11.0,21.0,,US,\n\
                    00ZZ,closed,Some Closed Field,12.0,22.0,,US,\n\
                    KJFK,large_airport,John F Kennedy Intl,40.6398,-73.7789,13,US,JFK\n";
        let (kept, idents, source_count) = convert_airports(csv).expect("converts");
        assert_eq!(source_count, 4, "every source row is counted");
        assert_eq!(kept.len(), 1, "only the large_airport row is kept");
        assert!(!idents.contains("00XX"));
        assert!(!idents.contains("00YY"));
        assert!(!idents.contains("00ZZ"));
        assert!(idents.contains("KJFK"));
    }

    #[test]
    fn missing_elevation_and_iata_are_tolerated_as_blank_optional_fields() {
        let csv = "ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,iata_code\n\
                    00A,heliport,Total RF Heliport,40.070985,-74.933689,,,\n";
        let (kept, _idents, _source_count) = convert_airports(csv).expect("converts");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].elevation_ft, None);
        assert_eq!(kept[0].iso_country, None);
        assert_eq!(kept[0].iata, None);
    }

    #[test]
    fn reordered_and_extra_upstream_columns_do_not_break_parsing() {
        // `keywords` and `continent` are real upstream columns this importer does not consume;
        // `type`/`ident` are also reordered relative to the struct's own field order.
        let csv = "keywords,type,continent,ident,name,latitude_deg,longitude_deg\n\
                    JFK NYC,large_airport,NA,KJFK,John F Kennedy Intl,40.6398,-73.7789\n";
        let (kept, idents, source_count) = convert_airports(csv).expect("converts");
        assert_eq!(source_count, 1);
        assert_eq!(kept.len(), 1);
        assert!(idents.contains("KJFK"));
    }

    #[test]
    fn a_missing_required_column_fails_the_import_cleanly() {
        // No `latitude_deg`/`longitude_deg` at all — a required column, not merely a blank
        // value, so this must be a clean `Err`, not a panic or a silently-zeroed row.
        let csv = "ident,type,name\nKJFK,large_airport,John F Kennedy Intl\n";
        assert!(convert_airports(csv).is_err());
    }

    // ---- Runway conversion / orphan-drop ----------------------------------------------------

    #[test]
    fn runways_for_dropped_airports_are_excluded() {
        let mut kept_idents = HashSet::new();
        kept_idents.insert("KJFK".to_owned());

        let csv = "airport_ident,length_ft,width_ft,surface,closed,le_ident,le_latitude_deg,\
                    le_longitude_deg,le_heading_degT,he_ident,he_latitude_deg,he_longitude_deg,\
                    he_heading_degT\n\
                    KJFK,14511,150,Concrete,0,04L,40.63,-73.78,40,22R,40.65,-73.76,220\n\
                    00XX,3000,60,Turf,0,09,10.0,20.0,90,27,10.02,20.02,270\n";
        let (kept, source_count) = convert_runways(csv, &kept_idents).expect("converts");
        assert_eq!(source_count, 2, "every source row is counted");
        assert_eq!(kept.len(), 1, "only the KJFK runway survives");
        assert_eq!(kept[0].airport_ident, "KJFK");
        assert_eq!(kept[0].le_heading_deg, Some(40.0));
        assert_eq!(kept[0].he_heading_deg, Some(220.0));
    }
}
