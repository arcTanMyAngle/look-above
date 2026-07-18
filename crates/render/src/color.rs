//! The map background and base-map palette, and the sRGB conversion they share.

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
}
