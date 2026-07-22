//! The selected-aircraft info card's content (M2 item 2.8b; enrichment fields added M3 item
//! 3.5) — docs/01's "Selection: white outline + info card". The outline is
//! [`crate::aircraft::pack_selection_outline_instance`]; this module is the card's other half,
//! pure and `wgpu`-free like [`crate::stats_overlay`] (the same "layer owns GPU state, a plain
//! module owns content" split every M2 pass uses).
//!
//! **Enrichment fields (M3 item 3.5).** docs/13's info-card acceptance line asks for
//! "callsign/type/operator/route" — type/operator/route come from `app::enrichment`'s adsbdb
//! lookup (docs/09: "on-selection only"), read from the store at selection time and passed in
//! via [`InfoCardContent::with_enrichment`]. Unlike callsign/altitude/speed (omitted when
//! unknown, per this module's original M2 convention), an unknown type/operator/route shows
//! `UNKNOWN` rather than being dropped — the checklist's own "'—' for any unknown field, never
//! an error state on a 404 or cache miss" line, spelled with a word instead of a dash character
//! since [`crate::label_atlas::CHARSET`] has no dash of any kind.
//!
//! **Privacy rule 2.2.** An anonymized (LADD/PIA) target's card shows only "UNIDENTIFIED" plus
//! its altitude if known ("position/altitude only") — never a callsign (it does not really have
//! one), speed, or any enrichment field, mirroring [`crate::label::format_label_text`]'s own
//! anonymous-but-selected exception exactly. `app` never populates the enrichment lookup for an
//! anonymous target in the first place (privacy rule 2.2 gates the lookup itself, not just this
//! branch), so [`InfoCardContent::with_enrichment`] is never even called with real data for one —
//! but this branch also ignores those fields unconditionally, so a future caller mistake here
//! can't leak them either. Raw position (lat/lon) text is not rendered anywhere in this crate
//! yet, and adding it would mean widening [`crate::label_atlas::CHARSET`] (a decimal point, a
//! sign) for a feature this item's own checklist wording does not name — deferred, not silently
//! dropped; docs/13's fuller "position data" bar is the M2 gate's (2.10) concern to verify, not
//! this item's to invent ahead of it.
//!
//! Reuses [`crate::stats_overlay::pack_overlay_instances`] directly for GPU packing rather than
//! duplicating it: that function is already generic over an arbitrary list of lines, an origin,
//! and a color — exactly this card's own shape, just a different origin/color/line count.

use look_above_core::contracts::{AircraftMeta, Flight};
use look_above_core::sim::AircraftInstance;
use look_above_core::types::SourceId;

use crate::label;

/// Text shown for any enrichment field (type/operator/route) the store has no answer for yet —
/// never absent (unlike callsign/altitude/speed) and never an error, per this item's acceptance
/// line. Plain word rather than a dash: [`crate::label_atlas::CHARSET`] has no dash character.
const UNKNOWN: &str = "UNKNOWN";

/// Plain content for the selected-aircraft info card, built from the selected
/// [`AircraftInstance`] each frame `app`/`renderer.rs` finds one (`None` when nothing is
/// selected, or the selected `icao24` has left the feed), plus (M3 item 3.5) whatever
/// `app::enrichment`'s cached [`AircraftMeta`]/[`Flight`] lookup already has for it.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoCardContent {
    pub anonymous: bool,
    pub callsign: Option<String>,
    pub altitude_ft: Option<f64>,
    pub ground_speed_kt: Option<f64>,
    pub source: SourceId,
    pub type_code: Option<String>,
    pub operator: Option<String>,
    pub route_origin: Option<String>,
    pub route_destination: Option<String>,
}

impl InfoCardContent {
    /// Snapshots the fields this card needs from a live `instance` — owned, so it outlives the
    /// frame's borrowed `RenderFeed`. Enrichment fields start empty; attach them with
    /// [`Self::with_enrichment`].
    pub fn from_instance(instance: &AircraftInstance) -> Self {
        Self {
            anonymous: instance.anonymous,
            callsign: instance
                .callsign
                .as_ref()
                .map(|callsign| callsign.as_str().to_owned()),
            altitude_ft: instance.altitude_ft,
            ground_speed_kt: instance.ground_speed_kt,
            source: instance.source,
            type_code: None,
            operator: None,
            route_origin: None,
            route_destination: None,
        }
    }

    /// Folds in `app::enrichment`'s cached lookup (M3 item 3.5) — `meta`/`flight` are `None`
    /// both when nothing has been looked up yet and when the caller (privacy rule 2.2) never
    /// looked at all for an anonymous target; either way this card shows `UNKNOWN`, never an
    /// error, via [`format_lines`].
    #[must_use]
    pub fn with_enrichment(mut self, meta: Option<&AircraftMeta>, flight: Option<&Flight>) -> Self {
        self.type_code = meta.and_then(|meta| meta.type_code.clone());
        self.operator = meta.and_then(|meta| meta.operator.clone());
        self.route_origin = flight.and_then(|flight| flight.origin.clone());
        self.route_destination = flight.and_then(|flight| flight.destination.clone());
        self
    }
}

/// `origin`/`destination` as one line, `UNKNOWN` in either slot the store has no answer for, or
/// standing alone when neither is known — never a dash (see [`UNKNOWN`]'s own doc comment).
fn format_route(origin: Option<&str>, destination: Option<&str>) -> String {
    match (origin, destination) {
        (None, None) => UNKNOWN.to_owned(),
        (origin, destination) => format!(
            "{} {}",
            origin.unwrap_or(UNKNOWN),
            destination.unwrap_or(UNKNOWN)
        ),
    }
}

/// `source`'s uppercase display label — stays inside [`crate::label_atlas::CHARSET`] (`A`-`Z`
/// only, no lowercase), unlike [`SourceId::as_str`]'s lowercase wire spelling.
fn source_label(source: SourceId) -> &'static str {
    match source {
        SourceId::OpenSky => "OPENSKY",
        SourceId::AirplanesLive => "AIRPLANESLIVE",
        SourceId::AdsbLol => "ADSBLOL",
    }
}

/// The card's lines, top to bottom. Every character of every returned line is inside
/// [`crate::label_atlas::CHARSET`] (see this module's own charset test).
pub fn format_lines(content: &InfoCardContent) -> Vec<String> {
    if content.anonymous {
        let mut lines = vec!["UNIDENTIFIED".to_owned()];
        if let Some(altitude_ft) = content.altitude_ft {
            lines.push(label::format_flight_level(altitude_ft));
        }
        return lines;
    }

    let mut lines = Vec::new();
    if let Some(callsign) = &content.callsign {
        lines.push(callsign.clone());
    }
    if let Some(altitude_ft) = content.altitude_ft {
        lines.push(label::format_flight_level(altitude_ft));
    }
    if let Some(ground_speed_kt) = content.ground_speed_kt {
        lines.push(label::format_speed_kt(ground_speed_kt));
    }
    lines.push(format!(
        "TYPE {}",
        content.type_code.as_deref().unwrap_or(UNKNOWN)
    ));
    lines.push(format!(
        "OPR {}",
        content.operator.as_deref().unwrap_or(UNKNOWN)
    ));
    lines.push(format!(
        "RTE {}",
        format_route(
            content.route_origin.as_deref(),
            content.route_destination.as_deref()
        )
    ));
    lines.push(format!("SRC {}", source_label(content.source)));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label_atlas;

    fn known_content() -> InfoCardContent {
        InfoCardContent {
            anonymous: false,
            callsign: Some("DLH9LF".to_owned()),
            altitude_ft: Some(35_000.0),
            ground_speed_kt: Some(450.0),
            source: SourceId::OpenSky,
            type_code: Some("B738".to_owned()),
            operator: Some("LUFTHANSA".to_owned()),
            route_origin: Some("EDDF".to_owned()),
            route_destination: Some("KJFK".to_owned()),
        }
    }

    /// A selection with no enrichment fields yet — matches
    /// [`InfoCardContent::from_instance`]'s own default before
    /// [`InfoCardContent::with_enrichment`] runs, or a still-unresolved store lookup.
    fn unenriched_content() -> InfoCardContent {
        InfoCardContent {
            type_code: None,
            operator: None,
            route_origin: None,
            route_destination: None,
            ..known_content()
        }
    }

    #[test]
    fn every_character_of_every_line_is_inside_the_label_atlas_charset() {
        for content in [
            known_content(),
            unenriched_content(),
            InfoCardContent {
                anonymous: true,
                ..known_content()
            },
        ] {
            for line in format_lines(&content) {
                for ch in line.chars() {
                    assert!(
                        label_atlas::char_index(ch).is_some(),
                        "{ch:?} in {line:?} is outside label_atlas::CHARSET"
                    );
                }
            }
        }
    }

    #[test]
    fn a_normal_target_shows_callsign_altitude_speed_type_operator_route_and_source() {
        let lines = format_lines(&known_content());
        assert_eq!(
            lines,
            vec![
                "DLH9LF",
                "FL350",
                "450kt",
                "TYPE B738",
                "OPR LUFTHANSA",
                "RTE EDDF KJFK",
                "SRC OPENSKY",
            ]
        );
    }

    #[test]
    fn unknown_callsign_and_speed_are_omitted_not_placeholdered() {
        let content = InfoCardContent {
            callsign: None,
            ground_speed_kt: None,
            ..known_content()
        };
        let lines = format_lines(&content);
        assert!(!lines.iter().any(|line| line == "DLH9LF"));
        assert!(!lines.iter().any(|line| line.contains("kt")));
    }

    #[test]
    fn unresolved_enrichment_fields_show_unknown_never_omitted_or_an_error() {
        let lines = format_lines(&unenriched_content());
        assert_eq!(
            lines,
            vec![
                "DLH9LF",
                "FL350",
                "450kt",
                "TYPE UNKNOWN",
                "OPR UNKNOWN",
                "RTE UNKNOWN",
                "SRC OPENSKY",
            ]
        );
    }

    #[test]
    fn a_route_with_only_one_known_end_shows_unknown_in_the_other_slot() {
        let origin_only = InfoCardContent {
            route_destination: None,
            ..known_content()
        };
        assert!(format_lines(&origin_only).contains(&"RTE EDDF UNKNOWN".to_owned()));

        let destination_only = InfoCardContent {
            route_origin: None,
            ..known_content()
        };
        assert!(format_lines(&destination_only).contains(&"RTE UNKNOWN KJFK".to_owned()));
    }

    #[test]
    fn with_enrichment_fills_type_operator_and_route_from_a_meta_and_flight_lookup() {
        use look_above_core::contracts::AircraftCategory;
        use look_above_core::types::{Icao24, UnixSeconds};

        let icao24 = Icao24::from_hex("a4b213").expect("valid icao24 in test");
        let meta = AircraftMeta {
            icao24,
            registration: Some("N401TT".to_owned()),
            type_code: Some("SR22".to_owned()),
            category: AircraftCategory::Unknown,
            operator: Some("SOME OWNER".to_owned()),
            is_anonymous: false,
            fetched_at: Some(UnixSeconds(0)),
            lookup_failed_at: None,
        };
        let flight = Flight {
            icao24,
            callsign: None,
            origin: Some("PANC".to_owned()),
            destination: Some("KORD".to_owned()),
            first_seen: UnixSeconds(0),
            last_seen: UnixSeconds(0),
        };

        let content = InfoCardContent::from_instance(&test_instance())
            .with_enrichment(Some(&meta), Some(&flight));

        assert_eq!(content.type_code.as_deref(), Some("SR22"));
        assert_eq!(content.operator.as_deref(), Some("SOME OWNER"));
        assert_eq!(content.route_origin.as_deref(), Some("PANC"));
        assert_eq!(content.route_destination.as_deref(), Some("KORD"));
    }

    #[test]
    fn with_enrichment_leaves_fields_unknown_when_meta_and_flight_are_none() {
        let content = InfoCardContent::from_instance(&test_instance()).with_enrichment(None, None);
        assert_eq!(content.type_code, None);
        assert_eq!(content.operator, None);
        assert_eq!(content.route_origin, None);
        assert_eq!(content.route_destination, None);
    }

    fn test_instance() -> AircraftInstance {
        use look_above_core::contracts::AircraftCategory;
        use look_above_core::geo::MercatorXy;
        use look_above_core::sim::AltitudeBucket;
        use look_above_core::types::{CallSign, Icao24};

        AircraftInstance {
            icao24: Icao24::from_hex("a4b213").expect("valid icao24 in test"),
            position: MercatorXy::new(0.0, 0.0),
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::Ground,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous: false,
            callsign: Some(CallSign::new("DLH9LF").expect("valid callsign in test")),
            altitude_ft: Some(35_000.0),
            ground_speed_kt: Some(450.0),
            selected: true,
            source: SourceId::OpenSky,
        }
    }

    #[test]
    fn source_is_always_shown_since_it_is_always_known() {
        for source in [
            SourceId::OpenSky,
            SourceId::AirplanesLive,
            SourceId::AdsbLol,
        ] {
            let content = InfoCardContent {
                source,
                ..known_content()
            };
            assert!(format_lines(&content).contains(&format!("SRC {}", source_label(source))));
        }
    }

    #[test]
    fn an_anonymous_target_shows_only_unidentified_and_altitude() {
        let content = InfoCardContent {
            anonymous: true,
            ..known_content()
        };
        assert_eq!(format_lines(&content), vec!["UNIDENTIFIED", "FL350"]);
    }

    #[test]
    fn an_anonymous_target_never_shows_callsign_speed_source_or_enrichment_even_if_present() {
        let content = InfoCardContent {
            anonymous: true,
            ..known_content()
        };
        let lines = format_lines(&content);
        assert!(!lines.iter().any(|line| line.contains("DLH9LF")));
        assert!(!lines.iter().any(|line| line.contains("kt")));
        assert!(!lines.iter().any(|line| line.contains("SRC")));
        assert!(!lines.iter().any(|line| line.contains("TYPE")));
        assert!(!lines.iter().any(|line| line.contains("OPR")));
        assert!(!lines.iter().any(|line| line.contains("RTE")));
        assert!(!lines.iter().any(|line| line.contains("B738")));
        assert!(!lines.iter().any(|line| line.contains("LUFTHANSA")));
    }

    #[test]
    fn an_anonymous_target_with_no_known_altitude_shows_only_unidentified() {
        let content = InfoCardContent {
            anonymous: true,
            altitude_ft: None,
            ..known_content()
        };
        assert_eq!(format_lines(&content), vec!["UNIDENTIFIED"]);
    }
}
