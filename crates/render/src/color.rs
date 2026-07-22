//! The map background and base-map palette, and the sRGB conversion they share.

use look_above_core::contracts::FlightCategory;

/// Background of the map view, authored as nonlinear sRGB (`#0A0E14`).
///
/// docs/01 asks for a dark, desaturated field so aircraft stay the brightest thing on
/// screen; it does not fix a shade, so this one is ours (`DECISION_LOG` 2026-07-15).
const BACKGROUND_SRGB: [u8; 3] = [0x0A, 0x0E, 0x14];

/// Land fill, authored as nonlinear sRGB (`#12161D`).
///
/// docs/01 wants the map "desaturated" with aircraft as "the brightest things on screen";
/// docs/13's QA checklist wants land/ocean legible as distinct without either competing for
/// attention. Neither doc fixes a shade — this one is ours (M2 item 2.2b), picked as barely
/// lighter than [`BACKGROUND_SRGB`] (the ocean/void color) so the coastline does the work of
/// separating them, not a strong land/ocean contrast.
const LAND_FILL_SRGB: [u8; 3] = [0x12, 0x16, 0x1D];

/// Coastline stroke, authored as nonlinear sRGB (`#2E3742`).
///
/// docs/13's QA checklist wants coastlines "crisp, desaturated". A soft blue-gray reads as a
/// coastline without approaching the saturation/brightness reserved for aircraft.
const COASTLINE_STROKE_SRGB: [u8; 3] = [0x2E, 0x37, 0x42];

/// The sRGB electro-optical transfer function (IEC 61966-2-1): encoded → linear.
fn srgb_to_linear(encoded: f64) -> f64 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// The sRGB opto-electronic transfer function: linear → encoded. [`srgb_to_linear`]'s inverse —
/// needed by [`AltitudeRamp`], whose Oklab interpolation only ever produces a *linear* result,
/// which then needs re-encoding for a non-`srgb` target the same way [`linearize_for_format`]
/// passes flat colors' already-encoded values straight through.
fn linear_to_srgb(linear: f64) -> f64 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// `srgb` (nonlinear, 8-bit-per-channel) widened to `[0, 1]` linear RGB.
fn linear_from_srgb_u8(srgb: [u8; 3]) -> [f64; 3] {
    srgb.map(|channel| srgb_to_linear(f64::from(channel) / 255.0))
}

/// `srgb`, linearized if `format` stores linear values, passed through unchanged if it stores
/// what it is given. Shared by [`clear_color`] (a [`wgpu::Color`], written by the clear op) and
/// the base-map layer colors (written by a fragment shader) — both land on the same surface
/// and both need the same answer to "does the hardware apply the transfer function on the way
/// out or not".
fn linearize_for_format(srgb: [u8; 3], format: wgpu::TextureFormat) -> [f64; 3] {
    let encoded = srgb.map(|channel| f64::from(channel) / 255.0);
    if format.is_srgb() {
        encoded.map(srgb_to_linear)
    } else {
        encoded
    }
}

/// The clear value for a surface of `format`.
///
/// A [`wgpu::Color`] is always taken to be linear: written to an sRGB-encoded surface, the
/// hardware applies the transfer function on the way out, so an encoded value handed over
/// as-is is brightened twice and the background lands far lighter than authored. Linearize
/// for those formats, and pass the encoded values through for the ones that store what they
/// are given.
pub fn clear_color(format: wgpu::TextureFormat) -> wgpu::Color {
    let [r, g, b] = linearize_for_format(BACKGROUND_SRGB, format);
    wgpu::Color { r, g, b, a: 1.0 }
}

/// The land fill color as a shader-ready, opaque linear RGBA — same linearize-if-`srgb`
/// reasoning as [`clear_color`], since a fragment shader's output is subject to the identical
/// hardware transfer function on write.
pub fn land_fill_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(LAND_FILL_SRGB, format)
}

/// The coastline stroke color as a shader-ready, opaque linear RGBA.
pub fn coastline_stroke_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(COASTLINE_STROKE_SRGB, format)
}

/// The L0 density-dot color (M4 item 4.3), authored as nonlinear sRGB (`#FFFFFF`) with a low
/// per-dot alpha — unlike [`layer_color`]'s usual opaque `1.0`, since the globe density pass's
/// additive `BlendState` (`renderer.rs`) sums overlapping instances in the framebuffer: a low
/// per-dot alpha is what lets a handful of dots over a busy region visibly out-brighten an
/// isolated one — "brightness proportional to local count" (docs/01), not a fixed per-dot
/// brightness regardless of traffic. Neither doc fixes a shade or an alpha; both are ours, tuned
/// by eye (M4 item 4.3).
const DENSITY_DOT_SRGB: [u8; 3] = [0xFF, 0xFF, 0xFF];
const DENSITY_DOT_ALPHA: f32 = 0.25;

/// The density-dot color as a shader-ready linear RGBA, alpha included (see
/// [`DENSITY_DOT_ALPHA`]'s own doc comment on why it isn't opaque).
pub fn density_dot_color(format: wgpu::TextureFormat) -> [f32; 4] {
    let [r, g, b, _] = layer_color(DENSITY_DOT_SRGB, format);
    [r, g, b, DENSITY_DOT_ALPHA]
}

fn layer_color(srgb: [u8; 3], format: wgpu::TextureFormat) -> [f32; 4] {
    let [r, g, b] = linearize_for_format(srgb, format);
    // `f64 as f32`: colors are 8-bit-per-channel sRGB run through a bounded transfer function,
    // landing in [0, 1] either way — nowhere near the range where `f32` would lose meaningful
    // precision for a color a screen displays at 8-10 bits per channel anyway.
    #[allow(clippy::cast_possible_truncation)]
    let narrow = |channel: f64| channel as f32;
    [narrow(r), narrow(g), narrow(b), 1.0]
}

/// The altitude ramp's six stops (M2 item 2.5), authored as flat nonlinear-sRGB hex per the
/// high-fidelity-flight-visualization skill's table. [`AltitudeRamp`] (M4 item 4.5) Oklab-
/// interpolates between the five airborne ones by the aircraft's actual altitude; `ALT_GROUND_SRGB`
/// stays a flat fallback (on-ground or altitude-unknown is categorical, not a point on the
/// altitude domain — see [`AltitudeRamp::tint`]).
///
/// M4 item 4.5 nudged these from their original M2/M3 hex (`#6E7076`/`#C97B3D`/`#A8B84B`/
/// `#4DBE8F`/`#3FA9D0`/`#8B7BD8`): the originals' Oklab lightness peaked at `To10000Ft` and
/// declined afterward (`Above40000Ft` was the *dimmest* airborne stop), which fails docs/13's
/// "lightness ordering survives a deuteranopia simulation" line — that requires monotonic
/// lightness, and a peak-then-decline shape can't survive any colorblindness simulation, since
/// it was never actually ordered to begin with. Each stop's Oklab hue/chroma (`a`, `b`) is
/// unchanged; only lightness (`L`) was corrected, by the minimal-L2-distance non-decreasing
/// sequence (isotonic regression, ~0.02 `L` margin per step for a safe deuteranopia-simulated
/// margin) that fits the original values — the smallest change that makes the ramp actually
/// monotonic rather than a free re-authoring (`DECISION_LOG` 2026-07-22).
const ALT_GROUND_SRGB: [u8; 3] = [0x6E, 0x70, 0x76];
const ALT_BELOW_2000FT_SRGB: [u8; 3] = [0xC8, 0x7A, 0x3C];
const ALT_TO_10000FT_SRGB: [u8; 3] = [0x91, 0xA0, 0x2F];
const ALT_TO_28000FT_SRGB: [u8; 3] = [0x41, 0xB3, 0x85];
const ALT_TO_40000FT_SRGB: [u8; 3] = [0x47, 0xB0, 0xD7];
const ALT_ABOVE_40000FT_SRGB: [u8; 3] = [0xA7, 0x98, 0xF8];

/// The five airborne ramp stops, each anchored at its bucket's midpoint altitude (feet) — e.g.
/// `ALT_BELOW_2000FT_SRGB` (the `[0, 2,000)` ft bucket's flat M2 color) anchors at 1,000 ft, the
/// point that bucket's flat tint was truest to. Anchoring at bucket midpoints rather than
/// boundaries spreads each transition evenly across the *whole* neighboring bucket instead of
/// collapsing it to a hairline right at the old hard edge. The `Above40000Ft` band has no upper
/// bound, so it anchors at its own lower boundary (40,000 ft) and stays flat above it —
/// [`AltitudeRamp::tint`] clamps there rather than extrapolating past the ramp's last color.
const RAMP_STOPS_FT: [(f64, [u8; 3]); 5] = [
    (1_000.0, ALT_BELOW_2000FT_SRGB),
    (6_000.0, ALT_TO_10000FT_SRGB),
    (19_000.0, ALT_TO_28000FT_SRGB),
    (34_000.0, ALT_TO_40000FT_SRGB),
    (40_000.0, ALT_ABOVE_40000FT_SRGB),
];

/// Björn Ottosson's Oklab forward transform (<https://bottosson.github.io/posts/oklab/>),
/// linear sRGB → Oklab. The perceptual space [`AltitudeRamp`] interpolates in: a straight RGB
/// lerp between, say, amber and yellow-green dips through a muddy, less legible in-between hue;
/// Oklab keeps the ramp's lightness monotonic (docs/13's colorblind-safety line) at every
/// intermediate altitude, not just the six authored stops.
#[allow(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "r/g/b, l/m/s, l_/m_/s_ are Oklab's own published names (bottosson.github.io/posts/\
              oklab) — renaming them to satisfy the lint would make this harder to check against \
              its source, not easier"
)]
fn linear_srgb_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    [
        0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
        1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
        0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
    ]
}

/// [`linear_srgb_to_oklab`]'s inverse, Oklab → linear sRGB.
#[allow(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "same as linear_srgb_to_oklab: these are Oklab's own published variable names"
)]
fn oklab_to_linear_srgb(lab: [f64; 3]) -> [f64; 3] {
    let [l, a, b] = lab;
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;

    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;

    [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3,
    ]
}

/// The Oklab-interpolated altitude tint (M4 item 4.5), built once per surface format — the same
/// "precompute what doesn't change frame to frame" shape the old `altitude_bucket_tint_table`
/// used, just resolving to Oklab-space stops instead of six flat RGBA values, so the per-frame,
/// per-instance/per-vertex cost of [`AltitudeRamp::tint`] is a lerp plus one *inverse* Oklab
/// conversion (no `cbrt`) rather than a full forward+inverse round trip.
#[derive(Debug, Clone, Copy)]
pub struct AltitudeRamp {
    ground_tint: [f32; 4],
    /// [`RAMP_STOPS_FT`], pre-converted to Oklab — altitude (feet) paired with its Oklab color.
    stops_oklab: [(f64, [f64; 3]); 5],
    linear_output: bool,
}

impl AltitudeRamp {
    /// Resolves [`RAMP_STOPS_FT`] to Oklab for `format` — call once per surface format (mirrors
    /// the old `altitude_bucket_tint_table(format)` call site in `renderer.rs`), not per frame.
    pub fn new(format: wgpu::TextureFormat) -> Self {
        let stops_oklab =
            RAMP_STOPS_FT.map(|(ft, srgb)| (ft, linear_srgb_to_oklab(linear_from_srgb_u8(srgb))));
        Self {
            ground_tint: layer_color(ALT_GROUND_SRGB, format),
            stops_oklab,
            linear_output: format.is_srgb(),
        }
    }

    /// The tint for an aircraft/trail-sample at `altitude_ft`, opaque. `on_ground` or an unknown
    /// `altitude_ft` (`None`) reads as the flat ground stop — mirroring
    /// `look_above_core::sim::AltitudeBucket::classify`'s own on-ground-or-unknown fallback, since
    /// neither case is a point on the airborne ramp's numeric domain. Otherwise, Oklab-
    /// interpolates between the two [`RAMP_STOPS_FT`] bracketing `altitude_ft`, clamping flat
    /// beyond the first/last anchor rather than extrapolating past the ramp's authored colors.
    pub fn tint(&self, on_ground: bool, altitude_ft: Option<f64>) -> [f32; 4] {
        let Some(ft) = (!on_ground).then_some(altitude_ft).flatten() else {
            return self.ground_tint;
        };

        let stops = self.stops_oklab;
        let (first_ft, first_oklab) = stops[0];
        if ft <= first_ft {
            return self.finish(first_oklab);
        }
        let (last_ft, last_oklab) = stops[stops.len() - 1];
        if ft >= last_ft {
            return self.finish(last_oklab);
        }

        for window in stops.windows(2) {
            let (from_ft, from_oklab) = window[0];
            let (to_ft, to_oklab) = window[1];
            if ft <= to_ft {
                let t = (ft - from_ft) / (to_ft - from_ft);
                let blended = [
                    from_oklab[0] + (to_oklab[0] - from_oklab[0]) * t,
                    from_oklab[1] + (to_oklab[1] - from_oklab[1]) * t,
                    from_oklab[2] + (to_oklab[2] - from_oklab[2]) * t,
                ];
                return self.finish(blended);
            }
        }
        // Unreachable given the clamps above (ft is strictly between the first and last anchor,
        // so some window's `to_ft` must reach it), but a flat fallback is still a correct color
        // rather than a panic if the ramp's stop count ever changes without this loop keeping up.
        self.finish(last_oklab)
    }

    /// Oklab → this ramp's target format, matching [`linearize_for_format`]'s dual path: pass a
    /// linear result straight through for an `srgb` surface (the hardware re-applies the transfer
    /// function on write), re-encode it for one that isn't.
    fn finish(&self, oklab: [f64; 3]) -> [f32; 4] {
        let linear = oklab_to_linear_srgb(oklab);
        let out = if self.linear_output {
            linear
        } else {
            linear.map(linear_to_srgb)
        };
        #[allow(
            clippy::cast_possible_truncation,
            reason = "same as layer_color: an 8-10-bit-per-channel display color, nowhere near \
                      f32's precision limits — clamped first since an Oklab round trip can \
                      overshoot [0, 1] slightly for saturated colors"
        )]
        let narrow = |c: f64| c.clamp(0.0, 1.0) as f32;
        [narrow(out[0]), narrow(out[1]), narrow(out[2]), 1.0]
    }
}

/// Label text color (M2 item 2.7b), authored as nonlinear sRGB (`#EAF0F6`) — docs/01: "aircraft
/// are the brightest things on screen," and a label is an aircraft's own text, so it reads at a
/// comparably bright near-white rather than receding into the map palette.
const LABEL_TEXT_SRGB: [u8; 3] = [0xEA, 0xF0, 0xF6];

/// Leader-line color (M2 item 2.7b), authored as nonlinear sRGB (`#8A939E`) — a muted gray that
/// reads as connective tissue between a displaced label and its aircraft without competing with
/// either.
const LABEL_LEADER_SRGB: [u8; 3] = [0x8A, 0x93, 0x9E];

/// The label text color as shader-ready, opaque linear RGBA (same linearize-if-`srgb` reasoning
/// as [`land_fill_color`]/[`coastline_stroke_color`]).
pub fn label_text_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(LABEL_TEXT_SRGB, format)
}

/// The leader-line color as shader-ready linear RGBA, at a reduced alpha (`0.6`) so it stays
/// visually secondary to both the label text and the aircraft glyph it connects.
pub fn label_leader_color(format: wgpu::TextureFormat) -> [f32; 4] {
    let mut color = layer_color(LABEL_LEADER_SRGB, format);
    color[3] = 0.6;
    color
}

/// Stats-overlay (F3 HUD) text color (M2 item 2.1b), authored as nonlinear sRGB (`#8FE3FF`) — a
/// pale cyan, deliberately distinct from [`LABEL_TEXT_SRGB`]'s warm near-white so the debug HUD
/// reads as its own separate layer rather than another aircraft label.
const STATS_OVERLAY_TEXT_SRGB: [u8; 3] = [0x8F, 0xE3, 0xFF];

/// The stats-overlay text color as shader-ready, opaque linear RGBA (same linearize-if-`srgb`
/// reasoning as [`label_text_color`]).
pub fn stats_overlay_text_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(STATS_OVERLAY_TEXT_SRGB, format)
}

/// Selection info-card text color (M2 item 2.8b), pure white (`#FFFFFF`) — docs/01's "Selection:
/// white outline + info card" ties the card's text to the same white as the glyph outline
/// (`aircraft::pack_selection_outline_instance`), so the whole selection affordance reads as one
/// consistent highlight color rather than a third, unrelated one.
const INFO_CARD_TEXT_SRGB: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// The info-card text color as shader-ready, opaque linear RGBA (same linearize-if-`srgb`
/// reasoning as [`label_text_color`]).
pub fn info_card_text_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(INFO_CARD_TEXT_SRGB, format)
}

/// Airport marker fill (M3 item 3.2), authored as nonlinear sRGB (`#4A525E`).
///
/// docs/01: aircraft stay the brightest things on screen; docs/13 wants map furniture (this is
/// static, not live traffic) legible without competing for attention. A cool, desaturated
/// gray-blue — clearly dimmer than every [`AltitudeRamp::tint`] output (including
/// `ALT_GROUND_SRGB`'s own already-muted gray, the ramp's dimmest stop) and far below
/// [`LABEL_TEXT_SRGB`]'s near-white — reads as a static point-of-interest marker, not an
/// aircraft.
const AIRPORT_MARKER_SRGB: [u8; 3] = [0x4A, 0x52, 0x5E];

/// Runway outline stroke (M3 item 3.2), authored as nonlinear sRGB (`#363D48`).
///
/// Dimmer again than [`AIRPORT_MARKER_SRGB`] (a runway outline is a longer, more visually
/// dominant shape than a small marker dot, so it reads better a touch quieter at the same
/// brightness) but still a shade brighter than [`COASTLINE_STROKE_SRGB`], so a runway reads as
/// "on top of" the coastline layer rather than blending into it — both stay well under the
/// altitude-ramp/label brightness this module's own
/// `palette_brightens_background_then_land_then_coastline_and_stays_dark`-style tests pin for the
/// rest of the map furniture.
const RUNWAY_OUTLINE_SRGB: [u8; 3] = [0x36, 0x3D, 0x48];

/// The airport marker fill color as shader-ready, opaque linear RGBA (same linearize-if-`srgb`
/// reasoning as [`land_fill_color`]/[`coastline_stroke_color`]).
pub fn airport_marker_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(AIRPORT_MARKER_SRGB, format)
}

/// The runway outline stroke color as shader-ready, opaque linear RGBA.
pub fn runway_outline_color(format: wgpu::TextureFormat) -> [f32; 4] {
    layer_color(RUNWAY_OUTLINE_SRGB, format)
}

/// METAR flight-category badge colors (M3 item 3.3), authored as nonlinear sRGB — docs/13's
/// own naming ("VFR green / MVFR blue / IFR red / LIFR magenta") is the aviation-standard
/// convention, so hue is fixed; these are this project's own shades of it (`DECISION_LOG`).
/// Moderately saturated and mid-brightness: distinguishable at a glance from
/// [`AIRPORT_MARKER_SRGB`]'s desaturated gray and from each other, but dimmer than every
/// [`AltitudeRamp::tint`] output so a badge still reads as map furniture, not live traffic
/// (docs/01: aircraft stay the brightest things on screen).
const FLIGHT_CATEGORY_VFR_SRGB: [u8; 3] = [0x22, 0x75, 0x3C];
const FLIGHT_CATEGORY_MVFR_SRGB: [u8; 3] = [0x2E, 0x6F, 0xB0];
const FLIGHT_CATEGORY_IFR_SRGB: [u8; 3] = [0xB2, 0x3A, 0x3A];
const FLIGHT_CATEGORY_LIFR_SRGB: [u8; 3] = [0xA2, 0x3F, 0xA8];

/// The badge color for `category`, as shader-ready, opaque linear RGBA (same
/// linearize-if-`srgb` reasoning as [`airport_marker_color`]).
pub fn flight_category_badge_color(
    category: FlightCategory,
    format: wgpu::TextureFormat,
) -> [f32; 4] {
    let srgb = match category {
        FlightCategory::Vfr => FLIGHT_CATEGORY_VFR_SRGB,
        FlightCategory::Mvfr => FLIGHT_CATEGORY_MVFR_SRGB,
        FlightCategory::Ifr => FLIGHT_CATEGORY_IFR_SRGB,
        FlightCategory::Lifr => FLIGHT_CATEGORY_LIFR_SRGB,
    };
    layer_color(srgb, format)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both ends of the curve are fixed points; without them a conversion error can hide.
    #[test]
    fn transfer_function_fixes_black_and_white() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-12);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-12);
    }

    /// The curve is piecewise: a linear toe below 0.04045, a power segment above. The two
    /// pieces must meet, or dark colors — which is all of this background — step.
    #[test]
    fn transfer_function_is_continuous_at_the_breakpoint() {
        let breakpoint = 0.040_45_f64;
        let toe = breakpoint / 12.92;
        let power = ((breakpoint + 0.055) / 1.055).powf(2.4);
        assert!((toe - power).abs() < 1e-6, "toe {toe} vs power {power}");
    }

    /// Linearizing must darken: this is the bug the conversion exists to prevent.
    #[test]
    fn srgb_surface_gets_a_darker_value_than_the_encoded_color() {
        let srgb = clear_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let plain = clear_color(wgpu::TextureFormat::Bgra8Unorm);

        assert!(srgb.r < plain.r, "{} !< {}", srgb.r, plain.r);
        assert!(srgb.g < plain.g);
        assert!(srgb.b < plain.b);
    }

    #[test]
    fn non_srgb_surface_gets_the_authored_values() {
        let color = clear_color(wgpu::TextureFormat::Bgra8Unorm);

        assert!((color.r - 10.0 / 255.0).abs() < 1e-12);
        assert!((color.g - 14.0 / 255.0).abs() < 1e-12);
        assert!((color.b - 20.0 / 255.0).abs() < 1e-12);
    }

    /// Whatever the format, the background stays dark and fully opaque.
    #[test]
    fn background_is_dark_and_opaque() {
        for format in [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            let color = clear_color(format);
            assert!(color.a == 1.0, "{format:?} is not opaque");
            for channel in [color.r, color.g, color.b] {
                assert!((0.0..0.1).contains(&channel), "{format:?} is not dark");
            }
        }
    }

    /// docs/01's whole point: aircraft (elsewhere) must stay the brightest thing on screen, so
    /// the base map's palette — background, land, coastline — needs to climb in that order and
    /// stay dark throughout. Checked in linear space (post-`clear_color`/`land_fill_color`
    /// conversion), which is what the GPU actually compares brightness in.
    #[test]
    fn palette_brightens_background_then_land_then_coastline_and_stays_dark() {
        for format in [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
        ] {
            let background = clear_color(format);
            let land = land_fill_color(format);
            let coastline = coastline_stroke_color(format);

            let luma = |r: f64, g: f64, b: f64| 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let background_luma = luma(background.r, background.g, background.b);
            let land_luma = luma(f64::from(land[0]), f64::from(land[1]), f64::from(land[2]));
            let coastline_luma = luma(
                f64::from(coastline[0]),
                f64::from(coastline[1]),
                f64::from(coastline[2]),
            );

            assert!(
                background_luma < land_luma,
                "{format:?}: land ({land_luma}) is not brighter than background ({background_luma})"
            );
            assert!(
                land_luma < coastline_luma,
                "{format:?}: coastline ({coastline_luma}) is not brighter than land ({land_luma})"
            );
            assert!(
                coastline_luma < 0.5,
                "{format:?}: coastline is not desaturated/dark"
            );

            assert!((0.99..=1.0).contains(&land[3]), "land is not opaque");
            assert!(
                (0.99..=1.0).contains(&coastline[3]),
                "coastline is not opaque"
            );
        }
    }

    /// Same bug `srgb_surface_gets_a_darker_value_than_the_encoded_color` guards for the
    /// background: an sRGB surface must linearize the layer colors too, not just pass the
    /// encoded hex through.
    #[test]
    fn srgb_surface_darkens_the_layer_colors_too() {
        let land_srgb = land_fill_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let land_plain = land_fill_color(wgpu::TextureFormat::Bgra8Unorm);
        assert!(land_srgb[0] < land_plain[0]);
        assert!(land_srgb[1] < land_plain[1]);
        assert!(land_srgb[2] < land_plain[2]);
    }

    // ---- Label colors (M2 item 2.7b) ---------------------------------------------------------

    #[test]
    fn label_text_is_bright_and_opaque_and_darkens_on_an_srgb_surface() {
        let srgb = label_text_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let plain = label_text_color(wgpu::TextureFormat::Bgra8Unorm);
        assert!((plain[3] - 1.0).abs() < 1e-6, "label text is opaque");
        let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        assert!(luma(plain) > 0.8, "label text must read as near-white");
        assert!(
            srgb[0] < plain[0],
            "srgb surface must darken the encoded color"
        );
    }

    #[test]
    fn leader_line_is_dimmer_than_label_text_and_semi_transparent() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let text = label_text_color(format);
        let leader = label_leader_color(format);
        let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        assert!(
            luma(leader) < luma(text),
            "leader line must not outshine the label text"
        );
        assert!(leader[3] < 1.0, "leader line must be semi-transparent");
    }

    // ---- Stats-overlay color (M2 item 2.1b) --------------------------------------------------

    #[test]
    fn stats_overlay_text_is_opaque_and_distinguishable_from_label_text() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let overlay = stats_overlay_text_color(format);
        let text = label_text_color(format);
        assert!(
            (overlay[3] - 1.0).abs() < 1e-6,
            "overlay text must be opaque"
        );
        assert_ne!(
            overlay, text,
            "the overlay must use a visually distinct color from label text"
        );
    }

    #[test]
    fn stats_overlay_text_darkens_on_an_srgb_surface() {
        let srgb = stats_overlay_text_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let plain = stats_overlay_text_color(wgpu::TextureFormat::Bgra8Unorm);
        assert!(srgb[0] < plain[0] || srgb[1] < plain[1] || srgb[2] < plain[2]);
    }

    // ---- Perceptual altitude ramp (M4 item 4.5) ----------------------------------------------

    /// Ground/unknown plus a spread of airborne altitudes: the five [`RAMP_STOPS_FT`] anchors
    /// (where the ramp must reproduce the authored flat color exactly) and four in-between
    /// values (where it must not).
    const SAMPLE_ALTITUDES_FT: [Option<f64>; 9] = [
        None,
        Some(1_000.0),
        Some(3_500.0),
        Some(6_000.0),
        Some(12_500.0),
        Some(19_000.0),
        Some(31_000.0),
        Some(34_000.0),
        Some(45_000.0),
    ];

    #[test]
    fn altitude_tint_is_ground_gray_when_on_ground_or_altitude_unknown() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        let ground = layer_color(ALT_GROUND_SRGB, format);

        assert_eq!(ramp.tint(true, Some(35_000.0)), ground, "on the ground");
        assert_eq!(ramp.tint(false, None), ground, "altitude unknown");
        assert_eq!(ramp.tint(true, None), ground, "on the ground, also unknown");
    }

    #[test]
    fn altitude_tint_reproduces_the_authored_stop_at_each_anchor() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        for &(anchor_ft, srgb) in &RAMP_STOPS_FT {
            let expected = layer_color(srgb, format);
            let actual = ramp.tint(false, Some(anchor_ft));
            for channel in 0..4 {
                assert!(
                    (actual[channel] - expected[channel]).abs() < 1e-4,
                    "at {anchor_ft} ft: {actual:?} != authored {expected:?}"
                );
            }
        }
    }

    #[test]
    fn altitude_tint_clamps_flat_beyond_the_ramp_ends() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        let below_first = ramp.tint(false, Some(-500.0));
        let at_first = ramp.tint(false, Some(RAMP_STOPS_FT[0].0));
        assert_eq!(below_first, at_first);

        let above_last = ramp.tint(false, Some(60_000.0));
        let at_last = layer_color(ALT_ABOVE_40000FT_SRGB, format);
        for channel in 0..4 {
            assert!((above_last[channel] - at_last[channel]).abs() < 1e-4);
        }
    }

    #[test]
    fn altitude_tint_is_continuous_across_a_dense_sweep() {
        // The whole point of M4 item 4.5: no bucket-boundary hitch. A straight-RGB lerp of these
        // stops can jump as much as ~0.2 per channel between two samples 100 ft apart right at an
        // old bucket edge; Oklab interpolation at this sample density should stay an order of
        // magnitude smoother than that everywhere on the ramp.
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        let step_ft = 100.0;
        let mut previous = ramp.tint(false, Some(0.0));
        let mut ft = step_ft;
        while ft <= 41_000.0 {
            let current = ramp.tint(false, Some(ft));
            for channel in 0..3 {
                let delta = (current[channel] - previous[channel]).abs();
                assert!(
                    delta < 0.02,
                    "channel {channel} jumped {delta} between {} and {ft} ft",
                    ft - step_ft
                );
            }
            previous = current;
            ft += step_ft;
        }
    }

    #[test]
    fn altitude_tint_is_opaque_and_in_range_across_every_sample() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        for on_ground in [false, true] {
            for altitude_ft in SAMPLE_ALTITUDES_FT {
                let color = ramp.tint(on_ground, altitude_ft);
                assert!((color[3] - 1.0).abs() < 1e-6, "{color:?} must be opaque");
                for channel in &color[..3] {
                    assert!(
                        (0.0..=1.0).contains(channel),
                        "{color:?} channel {channel} out of range"
                    );
                }
            }
        }
    }

    #[test]
    fn altitude_tint_darkens_on_an_srgb_surface_too() {
        let srgb_ramp = AltitudeRamp::new(wgpu::TextureFormat::Bgra8UnormSrgb);
        let plain_ramp = AltitudeRamp::new(wgpu::TextureFormat::Bgra8Unorm);
        for altitude_ft in SAMPLE_ALTITUDES_FT {
            let srgb = srgb_ramp.tint(false, altitude_ft);
            let plain = plain_ramp.tint(false, altitude_ft);
            assert!(
                srgb[0] < plain[0] || srgb[1] < plain[1] || srgb[2] < plain[2],
                "at {altitude_ft:?} ft: srgb {srgb:?} must darken vs. plain {plain:?}"
            );
        }
    }

    /// Machado, Oliveira & Fernandes (2009) full-severity deuteranopia simulation matrix,
    /// applied in linear RGB — automates docs/13's "altitude ramp distinguishable in a
    /// deuteranopia simulation (lightness ordering survives)" line.
    fn simulate_deuteranopia(linear_rgb: [f64; 3]) -> [f64; 3] {
        let [r, g, b] = linear_rgb;
        [
            0.367_322 * r + 0.860_646 * g - 0.227_968 * b,
            0.280_085 * r + 0.672_501 * g + 0.047_413 * b,
            -0.011_820 * r + 0.042_940 * g + 0.968_881 * b,
        ]
    }

    #[test]
    fn altitude_ramp_lightness_ordering_survives_a_deuteranopia_simulation() {
        let stops_srgb = [
            ALT_GROUND_SRGB,
            ALT_BELOW_2000FT_SRGB,
            ALT_TO_10000FT_SRGB,
            ALT_TO_28000FT_SRGB,
            ALT_TO_40000FT_SRGB,
            ALT_ABOVE_40000FT_SRGB,
        ];
        let lightness: Vec<f64> = stops_srgb
            .iter()
            .map(|&srgb| {
                let simulated = simulate_deuteranopia(linear_from_srgb_u8(srgb));
                linear_srgb_to_oklab(simulated)[0]
            })
            .collect();
        for pair in lightness.windows(2) {
            assert!(
                pair[1] > pair[0],
                "lightness order must survive deuteranopia simulation: {lightness:?}"
            );
        }
    }

    // ---- Airport marker / runway outline colors (M3 item 3.2) -------------------------------

    #[test]
    fn airport_marker_and_runway_outline_are_dimmer_than_every_altitude_tint_and_label_text() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let marker_luma = luma(airport_marker_color(format));
        let runway_luma = luma(runway_outline_color(format));
        let label_luma = luma(label_text_color(format));

        assert!(
            marker_luma < label_luma,
            "airport marker must stay dimmer than label text"
        );
        assert!(
            runway_luma < label_luma,
            "runway outline must stay dimmer than label text"
        );
        for on_ground in [false, true] {
            for altitude_ft in SAMPLE_ALTITUDES_FT {
                let tint_luma = luma(ramp.tint(on_ground, altitude_ft));
                assert!(
                    marker_luma < tint_luma,
                    "airport marker must stay dimmer than the tint at {altitude_ft:?} ft \
                     (on_ground={on_ground})"
                );
                assert!(
                    runway_luma < tint_luma,
                    "runway outline must stay dimmer than the tint at {altitude_ft:?} ft \
                     (on_ground={on_ground})"
                );
            }
        }
    }

    #[test]
    fn airport_marker_and_runway_outline_darken_on_an_srgb_surface() {
        let marker_srgb = airport_marker_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let marker_plain = airport_marker_color(wgpu::TextureFormat::Bgra8Unorm);
        assert!(
            marker_srgb[0] < marker_plain[0]
                || marker_srgb[1] < marker_plain[1]
                || marker_srgb[2] < marker_plain[2]
        );

        let runway_srgb = runway_outline_color(wgpu::TextureFormat::Bgra8UnormSrgb);
        let runway_plain = runway_outline_color(wgpu::TextureFormat::Bgra8Unorm);
        assert!(
            runway_srgb[0] < runway_plain[0]
                || runway_srgb[1] < runway_plain[1]
                || runway_srgb[2] < runway_plain[2]
        );
    }

    // ---- Flight-category badge colors (M3 item 3.3) ------------------------------------------

    const ALL_CATEGORIES: [FlightCategory; 4] = [
        FlightCategory::Vfr,
        FlightCategory::Mvfr,
        FlightCategory::Ifr,
        FlightCategory::Lifr,
    ];

    #[test]
    fn every_flight_category_gets_a_distinct_opaque_color() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let mut seen = Vec::new();
        for category in ALL_CATEGORIES {
            let color = flight_category_badge_color(category, format);
            assert!(
                (color[3] - 1.0).abs() < 1e-6,
                "{category:?} badge must be opaque"
            );
            seen.push(color);
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                assert_ne!(
                    seen[i], seen[j],
                    "{:?} and {:?} share a badge color",
                    ALL_CATEGORIES[i], ALL_CATEGORIES[j]
                );
            }
        }
    }

    #[test]
    #[allow(
        clippy::similar_names,
        reason = "ifr_color/lifr_color mirror the domain's own IFR/LIFR terminology (docs/13); \
                  obscuring the names to satisfy the lint would hurt readability more than it helps"
    )]
    fn badge_colors_follow_the_documented_hue_convention() {
        // docs/13: VFR green / MVFR blue / IFR red / LIFR magenta — checked on the dominant
        // channel(s) rather than pinning exact values, so the shade can be retuned freely.
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let vfr_color = flight_category_badge_color(FlightCategory::Vfr, format);
        let mvfr_color = flight_category_badge_color(FlightCategory::Mvfr, format);
        let ifr_color = flight_category_badge_color(FlightCategory::Ifr, format);
        let lifr_color = flight_category_badge_color(FlightCategory::Lifr, format);

        assert!(
            vfr_color[1] > vfr_color[0] && vfr_color[1] > vfr_color[2],
            "VFR must read as green"
        );
        assert!(
            mvfr_color[2] > mvfr_color[0] && mvfr_color[2] > mvfr_color[1],
            "MVFR must read as blue"
        );
        assert!(
            ifr_color[0] > ifr_color[1] && ifr_color[0] > ifr_color[2],
            "IFR must read as red"
        );
        assert!(
            lifr_color[0] > lifr_color[1] && lifr_color[2] > lifr_color[1],
            "LIFR must read as magenta (red and blue both above green)"
        );
    }

    #[test]
    fn badge_colors_stay_dimmer_than_every_altitude_tint_and_label_text() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let ramp = AltitudeRamp::new(format);
        let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let label_luma = luma(label_text_color(format));

        for category in ALL_CATEGORIES {
            let badge_luma = luma(flight_category_badge_color(category, format));
            assert!(
                badge_luma < label_luma,
                "{category:?} badge must stay dimmer than label text"
            );
            for on_ground in [false, true] {
                for altitude_ft in SAMPLE_ALTITUDES_FT {
                    let tint_luma = luma(ramp.tint(on_ground, altitude_ft));
                    assert!(
                        badge_luma < tint_luma,
                        "{category:?} badge must stay dimmer than the tint at {altitude_ft:?} ft \
                         (on_ground={on_ground})"
                    );
                }
            }
        }
    }

    #[test]
    fn badge_colors_darken_on_an_srgb_surface() {
        for category in ALL_CATEGORIES {
            let srgb = flight_category_badge_color(category, wgpu::TextureFormat::Bgra8UnormSrgb);
            let plain = flight_category_badge_color(category, wgpu::TextureFormat::Bgra8Unorm);
            assert!(
                srgb[0] < plain[0] || srgb[1] < plain[1] || srgb[2] < plain[2],
                "{category:?} must darken on an sRGB surface"
            );
        }
    }
}
