//! The selected-aircraft info card's content (M2 item 2.8b) — docs/01's "Selection: white
//! outline + info card". The outline is [`crate::aircraft::pack_selection_outline_instance`];
//! this module is the card's other half, pure and `wgpu`-free like [`crate::stats_overlay`] (the
//! same "layer owns GPU state, a plain module owns content" split every M2 pass uses).
//!
//! **Enrichment fields deferred to M3.** docs/13's own info-card acceptance line asks for
//! "callsign/type/operator/route" — but type/operator/route come from the `adsbdb` enrichment
//! lookup (docs/09: "on-selection only"), which does not exist until M3. This card shows exactly
//! what `core::sim::AircraftInstance` already carries: callsign, altitude, ground speed, and
//! which feed the current fix came from — the M2 checklist's own "minimal info card
//! (callsign/alt/speed/source; enrichment fields arrive in M3)" scope, not a silent shortfall.
//!
//! **Privacy rule 2.2.** An anonymized (LADD/PIA) target's card shows only "UNIDENTIFIED" plus
//! its altitude if known ("position/altitude only") — never a callsign (it does not really have
//! one) or speed, mirroring [`crate::label::format_label_text`]'s own anonymous-but-selected
//! exception exactly. Raw position (lat/lon) text is not rendered anywhere in this crate yet, and
//! adding it would mean widening [`crate::label_atlas::CHARSET`] (a decimal point, a sign) for a
//! feature this item's own checklist wording does not name — deferred, not silently dropped;
//! docs/13's fuller "position data" bar is the M2 gate's (2.10) concern to verify, not this
//! item's to invent ahead of it.
//!
//! Reuses [`crate::stats_overlay::pack_overlay_instances`] directly for GPU packing rather than
//! duplicating it: that function is already generic over an arbitrary list of lines, an origin,
//! and a color — exactly this card's own shape, just a different origin/color/line count.

use look_above_core::sim::AircraftInstance;
use look_above_core::types::SourceId;

use crate::label;

/// Plain content for the selected-aircraft info card, built from the selected
/// [`AircraftInstance`] each frame `app`/`renderer.rs` finds one (`None` when nothing is
/// selected, or the selected `icao24` has left the feed).
#[derive(Debug, Clone, PartialEq)]
pub struct InfoCardContent {
    pub anonymous: bool,
    pub callsign: Option<String>,
    pub altitude_ft: Option<f64>,
    pub ground_speed_kt: Option<f64>,
    pub source: SourceId,
}

impl InfoCardContent {
    /// Snapshots the fields this card needs from a live `instance` — owned, so it outlives the
    /// frame's borrowed `RenderFeed`.
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
        }
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
        }
    }

    #[test]
    fn every_character_of_every_line_is_inside_the_label_atlas_charset() {
        for content in [
            known_content(),
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
    fn a_normal_target_shows_callsign_altitude_speed_and_source() {
        let lines = format_lines(&known_content());
        assert_eq!(lines, vec!["DLH9LF", "FL350", "450kt", "SRC OPENSKY"]);
    }

    #[test]
    fn unknown_fields_are_omitted_not_placeholdered() {
        let content = InfoCardContent {
            callsign: None,
            ground_speed_kt: None,
            ..known_content()
        };
        assert_eq!(format_lines(&content), vec!["FL350", "SRC OPENSKY"]);
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
    fn an_anonymous_target_never_shows_callsign_speed_or_source_even_if_present() {
        let content = InfoCardContent {
            anonymous: true,
            ..known_content()
        };
        let lines = format_lines(&content);
        assert!(!lines.iter().any(|line| line.contains("DLH9LF")));
        assert!(!lines.iter().any(|line| line.contains("kt")));
        assert!(!lines.iter().any(|line| line.contains("SRC")));
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
