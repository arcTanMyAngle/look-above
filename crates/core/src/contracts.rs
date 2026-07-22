//! The seams the crates agree on (docs/09). Changing one requires a decision-log entry.
//!
//! `core` owns the traits; `ingest` and `store` implement them. M0 defines the
//! shapes only — there are no implementations yet.

use async_trait::async_trait;

use crate::error::{SourceError, StoreError};
use crate::types::{BBox, CallSign, Icao24, SourceId, StateVector, UnixSeconds};

/// What to ask a source for: a bounding box, or the whole world.
///
/// `None` means global rather than a ±180° box — sources bill global and regional
/// queries differently, so the distinction must survive to the adapter (docs/09).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RegionQuery {
    pub bbox: Option<BBox>,
}

impl RegionQuery {
    /// A query covering the entire globe.
    pub const GLOBAL: Self = Self { bbox: None };

    /// A query restricted to `bbox`.
    pub const fn region(bbox: BBox) -> Self {
        Self { bbox: Some(bbox) }
    }
}

/// One external live-data source (implemented in `crates/ingest`).
///
/// Adapters own auth, pagination, and parsing, and must be side-effect free
/// beyond HTTP and their token cache. Only sources on the authorized allowlist
/// may be implemented (privacy rules 1.1–1.2).
///
/// `#[async_trait]` rather than a native `async fn`: the poller holds its sources
/// as trait objects to fail over between them, and native async-fn-in-trait is
/// not dyn-compatible.
#[async_trait]
pub trait LiveSource: Send + Sync {
    /// Which source this is — also the value written to `positions.source`.
    fn id(&self) -> SourceId;

    /// Cost of `query` in this source's own budget units; 0 when unmetered.
    ///
    /// Pure and cheap: the poller calls this to plan a cycle *before* committing
    /// to a request (`OpenSky` bills 1–4 credits per bbox by area, docs/09).
    fn cost(&self, query: &RegionQuery) -> u32;

    /// Fetches and normalizes the current state of `query`'s region.
    ///
    /// Must never panic on a malformed record: skip it and carry on (docs/09).
    async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError>;
}

/// Local persistence (implemented in `crates/store`).
///
/// Synchronous by design and called only from the single writer thread that owns
/// the connection (docs/08); readers use separate read-only connections.
pub trait Store {
    /// Inserts a poll cycle's positions — one transaction per batch.
    fn insert_positions(&mut self, batch: &[StateVector]) -> Result<(), StoreError>;

    /// Inserts or updates cached airframe metadata.
    ///
    /// Callers must not reach here for an anonymous target carrying identity:
    /// enrichment is gated before lookup (privacy rule 2.2).
    fn upsert_aircraft_meta(&mut self, meta: &AircraftMeta) -> Result<(), StoreError>;

    /// The cached airframe metadata row for `icao24`, or `None` if it has never been looked
    /// up. Read side of [`upsert_aircraft_meta`](Self::upsert_aircraft_meta) — callers use
    /// `fetched_at`/`lookup_failed_at` to decide whether a fresh adsbdb lookup is warranted
    /// (M3 item 3.4's 24 h negative-cache gate).
    fn aircraft_meta(&self, icao24: Icao24) -> Result<Option<AircraftMeta>, StoreError>;

    /// Inserts one resolved flight/route observation (`flights` table, docs/08).
    ///
    /// A plain insert, not an upsert: M3 item 3.4 pulled this table forward from its
    /// originally planned M5 milestone to back on-selection route caching (`DECISION_LOG`
    /// 2026-07-21, M3 3.4), but the session-boundary merge M5's own "observed flights"
    /// design implies (extending `last_seen`, detecting gaps via `positions`) still awaits
    /// the `positions` table M5 brings. Each successful, non-cached adsbdb route lookup is
    /// its own row.
    fn insert_flight(&mut self, flight: &Flight) -> Result<(), StoreError>;

    /// The most recently observed [`Flight`] row for `icao24` (highest `last_seen`), or
    /// `None` if none has ever been recorded.
    fn latest_flight(&self, icao24: Icao24) -> Result<Option<Flight>, StoreError>;

    /// Airports within `bbox` at or above `min_size` (see [`AirportSize`]).
    fn airports_in_bbox(
        &self,
        bbox: BBox,
        min_size: AirportSize,
    ) -> Result<Vec<Airport>, StoreError>;

    /// Runways within `bbox`'s airports at or above `min_size` (see [`AirportSize`]).
    fn runways_in_bbox(&self, bbox: BBox, min_size: AirportSize)
    -> Result<Vec<Runway>, StoreError>;

    /// Inserts or updates cached METAR observations. Retention (keep the two most recent per
    /// station, docs/08) is the implementation's job, not the caller's. M3 item 3.3.
    fn upsert_metars(&mut self, batch: &[Metar]) -> Result<(), StoreError>;

    /// The freshest cached METAR for each of `stations` that has one — stations with no cached
    /// observation are simply absent from the result, never an error.
    fn metars_for_stations(&self, stations: &[String]) -> Result<Vec<Metar>, StoreError>;

    /// Deletes positions older than `keep_after`, returning the row count removed
    /// (privacy rule 5.1). Batched internally to avoid long write locks.
    fn prune(&mut self, keep_after: UnixSeconds) -> Result<u64, StoreError>;
}

/// Cached airframe metadata (`aircraft` table, docs/08). Populated in M1/M3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AircraftMeta {
    pub icao24: Icao24,
    pub registration: Option<String>,
    /// ICAO type designator, e.g. `B738`.
    pub type_code: Option<String>,
    pub category: AircraftCategory,
    /// Airline/operator name. `None` for private aircraft (privacy rule 4.3).
    pub operator: Option<String>,
    /// PIA/anonymized: gates all enrichment (privacy rule 2.2).
    pub is_anonymous: bool,
    /// `None` = never looked up.
    pub fetched_at: Option<UnixSeconds>,
    /// Negative cache for 404s (24 h).
    pub lookup_failed_at: Option<UnixSeconds>,
}

/// Airframe class — selects the glyph the renderer draws (docs/01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AircraftCategory {
    Jet,
    Turboprop,
    Piston,
    Heli,
    Glider,
    /// Not yet looked up, or looked up and unclassified — both draw the fallback glyph.
    #[default]
    Unknown,
}

impl AircraftCategory {
    /// The stable wire/DB spelling stored in `aircraft.category` (docs/08) — the inverse of
    /// [`from_store_str`](Self::from_store_str). Same shape as
    /// [`FlightCategory::as_str`](FlightCategory::as_str).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jet => "jet",
            Self::Turboprop => "turboprop",
            Self::Piston => "piston",
            Self::Heli => "heli",
            Self::Glider => "glider",
            Self::Unknown => "unknown",
        }
    }

    /// Maps `aircraft.category` (docs/08) back to a category. Unlike
    /// [`FlightCategory::from_metar_str`](FlightCategory::from_metar_str), this never fails to
    /// produce a value: `Unknown` is already this type's own documented catch-all for "not yet
    /// looked up, or looked up and unclassified", so an unrecognized or foreign string (upstream
    /// drift, or a value written by a future version) maps to `Unknown` rather than requiring
    /// callers to handle a second "absent" case on top of it.
    pub fn from_store_str(raw: &str) -> Self {
        match raw {
            "jet" => Self::Jet,
            "turboprop" => Self::Turboprop,
            "piston" => Self::Piston,
            "heli" => Self::Heli,
            "glider" => Self::Glider,
            _ => Self::Unknown,
        }
    }
}

/// An airport from the `OurAirports` import (`airports` table, docs/08). M3.
#[derive(Debug, Clone, PartialEq)]
pub struct Airport {
    /// ICAO/GPS code, e.g. `KJFK`.
    pub ident: String,
    pub name: String,
    pub size: AirportSize,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub elevation_ft: Option<i32>,
    pub iso_country: Option<String>,
    pub iata: Option<String>,
}

/// A runway from the `OurAirports` import (`runways` table, docs/08). M3.
///
/// `runways.closed` is deliberately not carried here: closed runways are filtered out in SQL
/// at query time (`store::ourairports::runways_in_bbox`), so a caller reading this type never
/// has a flag to check. A runway with a `None` `le_*`/`he_*` end is still returned — the
/// bundled CSV has some incomplete rows, and it's the render side's job to decide what to draw
/// (or not) for a partial runway, not this type or its query.
#[derive(Debug, Clone, PartialEq)]
pub struct Runway {
    /// The `airports.ident` this runway belongs to.
    pub airport_ident: String,
    pub le_ident: Option<String>,
    pub le_lat_deg: Option<f64>,
    pub le_lon_deg: Option<f64>,
    pub le_heading_deg: Option<f64>,
    pub he_ident: Option<String>,
    pub he_lat_deg: Option<f64>,
    pub he_lon_deg: Option<f64>,
    pub he_heading_deg: Option<f64>,
    pub length_ft: Option<i32>,
    pub width_ft: Option<i32>,
    pub surface: Option<String>,
}

/// Airport prominence, ordered so LOD tiers can filter with `size >= min_size`.
///
/// Ordering is the point of this type: `Store::airports_in_bbox` takes a minimum,
/// and L1 shows large and medium only (docs/08). `OurAirports` types outside this
/// ladder (`seaplane_base`, `balloonport`, `closed`) are dropped at import — that
/// mapping is an M3 decision, recorded when the importer lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AirportSize {
    Heliport,
    Small,
    Medium,
    Large,
}

impl AirportSize {
    /// Maps an `OurAirports` `airports.type` value (docs/08) to its tier, or `None` for
    /// anything outside the ladder this type models.
    ///
    /// `seaplane_base`, `balloonport`, and `closed` are deliberately excluded — those rows are
    /// dropped entirely at import time (M3 item 3.1) rather than folded into `Heliport`/`Small`,
    /// which is the mapping decision this enum's own doc comment anticipated. Any other,
    /// undocumented value (upstream column drift) also maps to `None` so the importer's
    /// drop-unless-recognized behavior stays a single `match`, not two separate checks.
    pub fn from_ourairports_type(raw: &str) -> Option<Self> {
        match raw {
            "heliport" => Some(Self::Heliport),
            "small_airport" => Some(Self::Small),
            "medium_airport" => Some(Self::Medium),
            "large_airport" => Some(Self::Large),
            _ => None,
        }
    }
}

/// Flight-category classification from a METAR (`metars.flight_cat`, docs/08) — also selects
/// the badge color docs/13 assigns (VFR green / MVFR blue / IFR red / LIFR magenta). M3 3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlightCategory {
    Vfr,
    Mvfr,
    Ifr,
    Lifr,
}

impl FlightCategory {
    /// Maps `aviationweather.gov`'s `fltCat` field to a category, or `None` for anything it
    /// doesn't report (some observations lack a computable ceiling/visibility) or a value this
    /// closed set doesn't recognize — never a parse failure, the same "unknown maps to None"
    /// shape as [`AirportSize::from_ourairports_type`].
    pub fn from_metar_str(raw: &str) -> Option<Self> {
        match raw {
            "VFR" => Some(Self::Vfr),
            "MVFR" => Some(Self::Mvfr),
            "IFR" => Some(Self::Ifr),
            "LIFR" => Some(Self::Lifr),
            _ => None,
        }
    }

    /// The stable wire/DB spelling stored in `metars.flight_cat` (docs/08) — the inverse of
    /// [`from_metar_str`](Self::from_metar_str).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vfr => "VFR",
            Self::Mvfr => "MVFR",
            Self::Ifr => "IFR",
            Self::Lifr => "LIFR",
        }
    }
}

/// A cached METAR observation (`metars` table, docs/08). M3 item 3.3.
#[derive(Debug, Clone, PartialEq)]
pub struct Metar {
    /// ICAO station id, e.g. `KJFK` — joins to `airports.ident`.
    pub station: String,
    pub observed_at: UnixSeconds,
    /// The raw METAR text, kept for the info-card/debug overlay even though every parsed field
    /// below is also derived from it.
    pub raw: String,
    /// `None` when the source did not report a computable category.
    pub flight_category: Option<FlightCategory>,
    /// `None` for a calm/variable report with no single heading (e.g. `VRB`).
    pub wind_dir_deg: Option<i32>,
    pub wind_kt: Option<i32>,
    pub visibility_sm: Option<f64>,
}

/// A flight-category badge resolved to a position (M3 item 3.3) — the join of a queried
/// [`Airport`] with its cached [`Metar`] (by `station` == `ident`), done once by the caller
/// (`app::window`) rather than re-joined every render frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetarBadge {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub category: FlightCategory,
}

/// One observed flight/route (`flights` table, docs/08) — adsbdb's callsign→route answer,
/// cached against the icao24 that was selected when it was resolved. M3 item 3.4; see
/// [`Store::insert_flight`]'s doc comment for why this is a plain insert per lookup rather
/// than a merged session, and `DECISION_LOG` 2026-07-21 (M3 3.4) for why the table exists this
/// early at all (docs/08 originally tagged it M5).
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub icao24: Icao24,
    /// `None` when the feed reports no identity — mirrors [`StateVector::callsign`]
    /// (`crate::types`), never populated for an anonymous target (privacy rule 2.2).
    pub callsign: Option<CallSign>,
    /// ICAO airport code, from adsbdb; best-effort, absent when adsbdb has no route on file.
    pub origin: Option<String>,
    pub destination: Option<String>,
    pub first_seen: UnixSeconds,
    pub last_seen: UnixSeconds,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airport_size_orders_from_heliport_up_to_large() {
        assert!(AirportSize::Large > AirportSize::Medium);
        assert!(AirportSize::Medium > AirportSize::Small);
        assert!(AirportSize::Small > AirportSize::Heliport);
    }

    #[test]
    fn filtering_by_min_size_keeps_larger_airports() {
        // The L1 tier: large and medium only.
        let visible_at_l1: Vec<AirportSize> = [
            AirportSize::Heliport,
            AirportSize::Small,
            AirportSize::Medium,
            AirportSize::Large,
        ]
        .into_iter()
        .filter(|size| *size >= AirportSize::Medium)
        .collect();
        assert_eq!(visible_at_l1, [AirportSize::Medium, AirportSize::Large]);
    }

    #[test]
    fn ourairports_type_mapping_keeps_the_four_size_tiers() {
        assert_eq!(
            AirportSize::from_ourairports_type("heliport"),
            Some(AirportSize::Heliport)
        );
        assert_eq!(
            AirportSize::from_ourairports_type("small_airport"),
            Some(AirportSize::Small)
        );
        assert_eq!(
            AirportSize::from_ourairports_type("medium_airport"),
            Some(AirportSize::Medium)
        );
        assert_eq!(
            AirportSize::from_ourairports_type("large_airport"),
            Some(AirportSize::Large)
        );
    }

    #[test]
    fn ourairports_type_mapping_drops_non_airport_and_closed_types() {
        for dropped in ["seaplane_base", "balloonport", "closed"] {
            assert_eq!(
                AirportSize::from_ourairports_type(dropped),
                None,
                "{dropped} must be dropped at import, not silently mapped"
            );
        }
    }

    #[test]
    fn ourairports_type_mapping_drops_unrecognized_values_too() {
        // Upstream column drift (a future OurAirports `type` we don't know yet) must fail
        // closed the same way as the documented drop list, not panic or default to a tier.
        assert_eq!(AirportSize::from_ourairports_type("space_elevator"), None);
    }

    #[test]
    fn global_query_is_distinct_from_a_region() {
        assert_eq!(RegionQuery::GLOBAL.bbox, None);
        assert_eq!(RegionQuery::default(), RegionQuery::GLOBAL);

        let bbox = BBox::new(40.0, -75.0, 41.0, -73.0).expect("valid bbox in test");
        assert_eq!(RegionQuery::region(bbox).bbox, Some(bbox));
    }

    #[test]
    fn unlooked_up_aircraft_default_to_the_fallback_glyph() {
        assert_eq!(AircraftCategory::default(), AircraftCategory::Unknown);
    }

    #[test]
    fn aircraft_category_as_str_round_trips_through_from_store_str() {
        for category in [
            AircraftCategory::Jet,
            AircraftCategory::Turboprop,
            AircraftCategory::Piston,
            AircraftCategory::Heli,
            AircraftCategory::Glider,
            AircraftCategory::Unknown,
        ] {
            assert_eq!(
                AircraftCategory::from_store_str(category.as_str()),
                category
            );
        }
    }

    #[test]
    fn aircraft_category_from_store_str_falls_back_to_unknown_for_unrecognized_values() {
        for unrecognized in ["", "JET", "rocket", "N/A"] {
            assert_eq!(
                AircraftCategory::from_store_str(unrecognized),
                AircraftCategory::Unknown
            );
        }
    }

    /// The traits must be usable as trait objects: the poller keeps a failover
    /// list of sources, and the app swaps store backends behind a `Box<dyn Store>`.
    #[test]
    fn contracts_are_dyn_compatible() {
        const fn assert_dyn_compatible(_: Option<&dyn LiveSource>, _: Option<&dyn Store>) {}
        assert_dyn_compatible(None, None);
    }

    #[test]
    fn flight_category_recognizes_the_four_documented_values() {
        assert_eq!(
            FlightCategory::from_metar_str("VFR"),
            Some(FlightCategory::Vfr)
        );
        assert_eq!(
            FlightCategory::from_metar_str("MVFR"),
            Some(FlightCategory::Mvfr)
        );
        assert_eq!(
            FlightCategory::from_metar_str("IFR"),
            Some(FlightCategory::Ifr)
        );
        assert_eq!(
            FlightCategory::from_metar_str("LIFR"),
            Some(FlightCategory::Lifr)
        );
    }

    #[test]
    fn flight_category_rejects_unrecognized_values_instead_of_guessing() {
        for unrecognized in ["", "vfr", "N/A", "UNKNOWN"] {
            assert_eq!(FlightCategory::from_metar_str(unrecognized), None);
        }
    }

    #[test]
    fn flight_category_as_str_round_trips_through_from_metar_str() {
        for category in [
            FlightCategory::Vfr,
            FlightCategory::Mvfr,
            FlightCategory::Ifr,
            FlightCategory::Lifr,
        ] {
            assert_eq!(
                FlightCategory::from_metar_str(category.as_str()),
                Some(category)
            );
        }
    }
}
