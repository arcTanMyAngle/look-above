//! CPU-side click hit-testing for aircraft selection (M2 item 2.8a) — the render-side geometry
//! half of turning a click into a selected aircraft. `app::window` owns the click-vs-drag input
//! disambiguation and threads the result into `core::sim::Simulator::set_selected` (via the
//! simulation worker); this module only answers "which aircraft, if any, is under this screen
//! point", reusing `label::world_to_screen_px` so both passes agree on where a glyph actually is.
//!
//! **Linear scan, not the design notes' uniform grid.** `plans/M2_HIGH_FIDELITY_RENDERER.md`'s
//! design notes suggest "a simple uniform grid over screen space rebuilt per frame ... for
//! hit-testing and later label density". Hit-testing, though, runs once per click — not once per
//! frame the way the label pass's collision sweep does — so even a full scan over every drawn
//! aircraft (a regional view's few hundred, or a whole-world view's low thousands) costs nothing
//! worth optimizing ahead of a real cost. Building the grid now, before any label-density work
//! exists to justify a per-frame structure, would be exactly the premature abstraction
//! CLAUDE.md's conventions warn against — recorded here as a deliberate deviation rather than a
//! silently dropped requirement; revisit if profiling ever says otherwise.

use look_above_core::geo::MercatorXy;
use look_above_core::sim::AircraftInstance;
use look_above_core::types::Icao24;

use crate::aircraft::AIRCRAFT_GLYPH_PX;
use crate::label::{ScreenPoint, world_to_screen_px};

/// How far, in physical pixels, a click may land from a glyph's projected center and still hit
/// it — half the glyph's fixed on-screen size (docs/01's L2 glyph) plus a small tolerance, since a
/// real cursor click rarely lands dead-center on a 20px target.
const HIT_RADIUS_PX: f64 = AIRCRAFT_GLYPH_PX / 2.0 + 4.0;

/// Finds the aircraft whose glyph is nearest `cursor_px`, within [`HIT_RADIUS_PX`] — or `None` if
/// nothing is that close, so a click on open map deselects rather than always picking the nearest
/// aircraft regardless of distance.
///
/// Anonymous aircraft are selectable like any other: privacy rule 2.3 permits searching for and
/// selecting a specific aircraft of interest, and the anonymized target is still drawn (just never
/// labeled — `render::label::format_label_text`'s own gate). Showing "Unidentified" instead of its
/// real content in the info card is 2.8b's job, not this function's.
pub fn hit_test(
    aircraft: &[AircraftInstance],
    cursor_px: ScreenPoint,
    camera_center_m: MercatorXy,
    meters_per_pixel: f64,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> Option<Icao24> {
    aircraft
        .iter()
        .filter_map(|instance| {
            let glyph_px = world_to_screen_px(
                instance.position,
                camera_center_m,
                meters_per_pixel,
                viewport_width_px,
                viewport_height_px,
            );
            let distance_px = (glyph_px.0 - cursor_px.0).hypot(glyph_px.1 - cursor_px.1);
            (distance_px <= HIT_RADIUS_PX).then_some((distance_px, instance.icao24))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, icao24)| icao24)
}

#[cfg(test)]
mod tests {
    use look_above_core::contracts::AircraftCategory;
    use look_above_core::sim::AltitudeBucket;

    use super::*;

    fn icao(hex: &str) -> Icao24 {
        Icao24::from_hex(hex).expect("valid test ICAO24")
    }

    fn instance_at(icao_hex: &str, position: MercatorXy) -> AircraftInstance {
        AircraftInstance {
            icao24: icao(icao_hex),
            position,
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::To28000Ft,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous: false,
            callsign: None,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: false,
        }
    }

    const CENTER: MercatorXy = MercatorXy::new(0.0, 0.0);
    const MPP: f64 = 10.0;
    const VIEWPORT: (f64, f64) = (1000.0, 800.0);

    #[test]
    fn a_click_on_a_glyph_selects_it() {
        // The camera center itself projects to the viewport center.
        let aircraft = [instance_at("3c6444", CENTER)];
        let hit = hit_test(
            &aircraft,
            (500.0, 400.0),
            CENTER,
            MPP,
            VIEWPORT.0,
            VIEWPORT.1,
        );
        assert_eq!(hit, Some(icao("3c6444")));
    }

    #[test]
    fn a_click_within_the_hit_radius_but_off_center_still_selects() {
        let aircraft = [instance_at("3c6444", CENTER)];
        let hit = hit_test(
            &aircraft,
            (500.0 + HIT_RADIUS_PX - 1.0, 400.0),
            CENTER,
            MPP,
            VIEWPORT.0,
            VIEWPORT.1,
        );
        assert_eq!(hit, Some(icao("3c6444")));
    }

    #[test]
    fn a_click_beyond_the_hit_radius_selects_nothing() {
        let aircraft = [instance_at("3c6444", CENTER)];
        let hit = hit_test(
            &aircraft,
            (500.0 + HIT_RADIUS_PX + 1.0, 400.0),
            CENTER,
            MPP,
            VIEWPORT.0,
            VIEWPORT.1,
        );
        assert_eq!(hit, None);
    }

    #[test]
    fn a_click_on_open_map_with_no_aircraft_selects_nothing() {
        let hit = hit_test(&[], (500.0, 400.0), CENTER, MPP, VIEWPORT.0, VIEWPORT.1);
        assert_eq!(hit, None);
    }

    #[test]
    fn the_nearest_of_two_overlapping_glyphs_wins() {
        // Both within range of the click; the second is closer.
        let near = MercatorXy::new(2.0 * MPP, 0.0); // 2px right of center
        let far = MercatorXy::new(8.0 * MPP, 0.0); // 8px right of center
        let aircraft = [instance_at("3c6444", far), instance_at("4b1815", near)];
        let hit = hit_test(
            &aircraft,
            (500.0, 400.0),
            CENTER,
            MPP,
            VIEWPORT.0,
            VIEWPORT.1,
        );
        assert_eq!(hit, Some(icao("4b1815")));
    }

    #[test]
    fn an_anonymous_aircraft_is_selectable() {
        let mut instance = instance_at("3c6444", CENTER);
        instance.anonymous = true;
        let aircraft = [instance];
        let hit = hit_test(
            &aircraft,
            (500.0, 400.0),
            CENTER,
            MPP,
            VIEWPORT.0,
            VIEWPORT.1,
        );
        assert_eq!(hit, Some(icao("3c6444")));
    }
}
