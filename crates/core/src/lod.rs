//! LOD tier hysteresis state machine (M4 item 4.1).
//!
//! Pure math only — no rendering wiring. [`next_tier`] takes the current viewport span (see
//! [`crate::camera::Camera::viewport_span_km`] or an equivalent km-span source) and the
//! *previous* tier, and returns the tier that should now be active.
//!
//! This is a state machine rather than a stateless `span > threshold` check: the
//! high-fidelity-flight-visualization skill's tier table gives each boundary two different
//! thresholds depending on direction (e.g. 3,300 km entering `Global` while zooming out, 3,000 km
//! entering `Continental` while zooming in). A tier only exits toward the threshold it can be
//! re-entered from; the ~10% gap between the two is the hysteresis band that keeps a span
//! dithering near one raw value from flip-flopping the tier every frame.

/// One of the three zoom tiers (skill: L0/L1/L2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LodTier {
    /// L0: viewport span > ~3,300 km. Additive density dots, no glyphs/trails/labels.
    Global,
    /// L1: ~330-3,000 km. Small heading-rotated glyphs, no trails or labels.
    Continental,
    /// L2: < ~300 km. Full glyphs, altitude-colored trails, labels, selection.
    Regional,
}

/// Zooming out past this from `Continental` enters `Global`.
const ENTER_GLOBAL_ZOOM_OUT_KM: f64 = 3_300.0;
/// Zooming in past this from `Global` enters `Continental`.
const ENTER_CONTINENTAL_ZOOM_IN_KM: f64 = 3_000.0;
/// Zooming out past this from `Regional` enters `Continental`.
const ENTER_CONTINENTAL_ZOOM_OUT_KM: f64 = 330.0;
/// Zooming in past this from `Continental` (or `Global`) enters `Regional`.
const ENTER_REGIONAL_ZOOM_IN_KM: f64 = 300.0;

/// Given the previous tier and the current viewport span in kilometres, returns the tier that
/// should now be active.
///
/// Call this once per frame with the latest span; a span that jumps clear across a tier (a fast
/// zoom) resolves directly to the correct tier rather than requiring one call per intermediate
/// tier.
pub fn next_tier(previous: LodTier, viewport_span_km: f64) -> LodTier {
    match previous {
        LodTier::Global => {
            if viewport_span_km < ENTER_CONTINENTAL_ZOOM_IN_KM {
                if viewport_span_km < ENTER_REGIONAL_ZOOM_IN_KM {
                    LodTier::Regional
                } else {
                    LodTier::Continental
                }
            } else {
                LodTier::Global
            }
        }
        LodTier::Continental => {
            if viewport_span_km > ENTER_GLOBAL_ZOOM_OUT_KM {
                LodTier::Global
            } else if viewport_span_km < ENTER_REGIONAL_ZOOM_IN_KM {
                LodTier::Regional
            } else {
                LodTier::Continental
            }
        }
        LodTier::Regional => {
            if viewport_span_km > ENTER_CONTINENTAL_ZOOM_OUT_KM {
                if viewport_span_km > ENTER_GLOBAL_ZOOM_OUT_KM {
                    LodTier::Global
                } else {
                    LodTier::Continental
                }
            } else {
                LodTier::Regional
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `next_tier` once per span in sequence, returning the seed tier followed by the
    /// result of each call — so `history.windows(2)` shows every transition, including the
    /// first one away from the seed.
    fn run(seed: LodTier, spans: &[f64]) -> Vec<LodTier> {
        let mut tier = seed;
        let mut history = vec![tier];
        for &span in spans {
            tier = next_tier(tier, span);
            history.push(tier);
        }
        history
    }

    /// Alternates ±5% around `threshold_km`, starting above it.
    fn dither_sequence(threshold_km: f64, steps: usize) -> Vec<f64> {
        (0..steps)
            .map(|i| {
                if i % 2 == 0 {
                    threshold_km * 1.05
                } else {
                    threshold_km * 0.95
                }
            })
            .collect()
    }

    #[track_caller]
    fn assert_never_flips(history: &[LodTier]) {
        let mut seen_change = false;
        for window in history.windows(2) {
            if window[0] != window[1] {
                assert!(!seen_change, "tier flipped more than once: {history:?}");
                seen_change = true;
            }
        }
    }

    // --- dithering: at most one settle, never an oscillation --------------------

    #[test]
    fn dithering_around_3300_from_continental_settles_without_flipping() {
        let history = run(LodTier::Continental, &dither_sequence(3_300.0, 20));
        assert_never_flips(&history);
    }

    #[test]
    fn dithering_around_3000_from_global_settles_without_flipping() {
        let history = run(LodTier::Global, &dither_sequence(3_000.0, 20));
        assert_never_flips(&history);
    }

    #[test]
    fn dithering_around_330_from_regional_settles_without_flipping() {
        let history = run(LodTier::Regional, &dither_sequence(330.0, 20));
        assert_never_flips(&history);
    }

    #[test]
    fn dithering_around_300_from_continental_settles_without_flipping() {
        let history = run(LodTier::Continental, &dither_sequence(300.0, 20));
        assert_never_flips(&history);
    }

    // --- genuine crossings, both directions --------------------------------------

    #[test]
    fn genuine_zoom_out_crosses_continental_to_global() {
        assert_eq!(next_tier(LodTier::Continental, 3_301.0), LodTier::Global);
    }

    #[test]
    fn genuine_zoom_in_crosses_global_to_continental() {
        assert_eq!(next_tier(LodTier::Global, 2_999.0), LodTier::Continental);
    }

    #[test]
    fn genuine_zoom_out_crosses_regional_to_continental() {
        assert_eq!(next_tier(LodTier::Regional, 331.0), LodTier::Continental);
    }

    #[test]
    fn genuine_zoom_in_crosses_continental_to_regional() {
        assert_eq!(next_tier(LodTier::Continental, 299.0), LodTier::Regional);
    }

    #[test]
    fn staying_within_a_tiers_band_does_not_change_it() {
        assert_eq!(
            next_tier(LodTier::Continental, 1_500.0),
            LodTier::Continental
        );
        assert_eq!(next_tier(LodTier::Global, 5_000.0), LodTier::Global);
        assert_eq!(next_tier(LodTier::Regional, 50.0), LodTier::Regional);
    }

    #[test]
    fn a_fast_zoom_in_from_global_resolves_straight_to_regional() {
        assert_eq!(next_tier(LodTier::Global, 100.0), LodTier::Regional);
    }

    #[test]
    fn a_fast_zoom_out_from_regional_resolves_straight_to_global() {
        assert_eq!(next_tier(LodTier::Regional, 10_000.0), LodTier::Global);
    }
}
