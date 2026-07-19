//! The map background and base-map palette, and the sRGB conversion they share.

use look_above_core::sim::AltitudeBucket;

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
/// high-fidelity-flight-visualization skill's table. docs/01's checklist parenthetical is
/// explicit that the *perceptual* ramp (Oklab-interpolated between stops) lands in M4 — this
/// wires the six discrete altitude-bucket tints the glyph attribute carries now, one flat color
/// per bucket, no interpolation between them yet.
const ALT_GROUND_SRGB: [u8; 3] = [0x6E, 0x70, 0x76];
const ALT_BELOW_2000FT_SRGB: [u8; 3] = [0xC9, 0x7B, 0x3D];
const ALT_TO_10000FT_SRGB: [u8; 3] = [0xA8, 0xB8, 0x4B];
const ALT_TO_28000FT_SRGB: [u8; 3] = [0x4D, 0xBE, 0x8F];
const ALT_TO_40000FT_SRGB: [u8; 3] = [0x3F, 0xA9, 0xD0];
const ALT_ABOVE_40000FT_SRGB: [u8; 3] = [0x8B, 0x7B, 0xD8];

/// [`AltitudeBucket`]'s stop index in the skill's ramp order (ground up to FL400+) — the single
/// place that ordering is encoded; both [`altitude_bucket_tint_table`] (which builds an ordered
/// array) and `aircraft::pack_instance` (which indexes into it) go through this function so
/// there is exactly one place that could disagree with itself.
pub(crate) fn altitude_bucket_index(bucket: AltitudeBucket) -> usize {
    match bucket {
        AltitudeBucket::Ground => 0,
        AltitudeBucket::Below2000Ft => 1,
        AltitudeBucket::To10000Ft => 2,
        AltitudeBucket::To28000Ft => 3,
        AltitudeBucket::To40000Ft => 4,
        AltitudeBucket::Above40000Ft => 5,
    }
}

/// The altitude-bucket tint for `bucket`, opaque, in shader-ready linear RGBA (same
/// linearize-if-`srgb` reasoning as [`land_fill_color`]/[`coastline_stroke_color`]).
pub fn altitude_bucket_tint(bucket: AltitudeBucket, format: wgpu::TextureFormat) -> [f32; 4] {
    let srgb = match bucket {
        AltitudeBucket::Ground => ALT_GROUND_SRGB,
        AltitudeBucket::Below2000Ft => ALT_BELOW_2000FT_SRGB,
        AltitudeBucket::To10000Ft => ALT_TO_10000FT_SRGB,
        AltitudeBucket::To28000Ft => ALT_TO_28000FT_SRGB,
        AltitudeBucket::To40000Ft => ALT_TO_40000FT_SRGB,
        AltitudeBucket::Above40000Ft => ALT_ABOVE_40000FT_SRGB,
    };
    layer_color(srgb, format)
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

/// All six bucket tints, indexed by [`altitude_bucket_index`] — built once per surface format in
/// `renderer.rs` (the colors never change frame to frame, only which bucket applies), so the
/// per-instance packing path (`aircraft::pack_instance`) is a plain array lookup rather than
/// re-running the sRGB transfer function for every aircraft, every frame.
pub fn altitude_bucket_tint_table(format: wgpu::TextureFormat) -> [[f32; 4]; 6] {
    let mut table = [[0.0_f32; 4]; 6];
    for bucket in [
        AltitudeBucket::Ground,
        AltitudeBucket::Below2000Ft,
        AltitudeBucket::To10000Ft,
        AltitudeBucket::To28000Ft,
        AltitudeBucket::To40000Ft,
        AltitudeBucket::Above40000Ft,
    ] {
        table[altitude_bucket_index(bucket)] = altitude_bucket_tint(bucket, format);
    }
    table
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

    // ---- Altitude-bucket tint (M2 item 2.5) --------------------------------------------------

    const ALL_BUCKETS: [AltitudeBucket; 6] = [
        AltitudeBucket::Ground,
        AltitudeBucket::Below2000Ft,
        AltitudeBucket::To10000Ft,
        AltitudeBucket::To28000Ft,
        AltitudeBucket::To40000Ft,
        AltitudeBucket::Above40000Ft,
    ];

    #[test]
    fn altitude_bucket_tint_covers_all_six_buckets_with_distinct_opaque_colors() {
        let mut seen = Vec::new();
        for bucket in ALL_BUCKETS {
            let color = altitude_bucket_tint(bucket, wgpu::TextureFormat::Bgra8UnormSrgb);
            assert!(
                (color[3] - 1.0).abs() < 1e-6,
                "{bucket:?} tint must be opaque"
            );
            for channel in &color[..3] {
                assert!(
                    (0.0..=1.0).contains(channel),
                    "{bucket:?} channel {channel} out of range"
                );
            }
            seen.push(color);
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                assert_ne!(
                    seen[i], seen[j],
                    "buckets {:?} and {:?} share a tint",
                    ALL_BUCKETS[i], ALL_BUCKETS[j]
                );
            }
        }
    }

    #[test]
    fn altitude_bucket_index_is_a_bijection_onto_zero_through_five() {
        let mut indices: Vec<usize> = ALL_BUCKETS
            .iter()
            .map(|&b| altitude_bucket_index(b))
            .collect();
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn altitude_bucket_tint_table_matches_the_per_bucket_lookup() {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let table = altitude_bucket_tint_table(format);
        for bucket in ALL_BUCKETS {
            assert_eq!(
                table[altitude_bucket_index(bucket)],
                altitude_bucket_tint(bucket, format)
            );
        }
    }

    #[test]
    fn altitude_bucket_tint_darkens_on_an_srgb_surface_too() {
        let srgb = altitude_bucket_tint(
            AltitudeBucket::To28000Ft,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        );
        let plain =
            altitude_bucket_tint(AltitudeBucket::To28000Ft, wgpu::TextureFormat::Bgra8Unorm);
        assert!(srgb[0] < plain[0] || srgb[1] < plain[1] || srgb[2] < plain[2]);
    }
}
