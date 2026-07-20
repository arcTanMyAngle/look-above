//! The seams the crates agree on (docs/09). Changing one requires a decision-log entry.
//!
//! `core` owns the traits; `ingest` and `store` implement them. M0 defines the
//! shapes only — there are no implementations yet.

use async_trait::async_trait;

use crate::error::{SourceError, StoreError};
use crate::types::{BBox, Icao24, SourceId, StateVector, UnixSeconds};

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

    /// Airports within `bbox` at or above `min_size` (see [`AirportSize`]).
    fn airports_in_bbox(
        &self,
        bbox: BBox,
        min_size: AirportSize,
    ) -> Result<Vec<Airport>, StoreError>;

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

    /// The traits must be usable as trait objects: the poller keeps a failover
    /// list of sources, and the app swaps store backends behind a `Box<dyn Store>`.
    #[test]
    fn contracts_are_dyn_compatible() {
        const fn assert_dyn_compatible(_: Option<&dyn LiveSource>, _: Option<&dyn Store>) {}
        assert_dyn_compatible(None, None);
    }
}
