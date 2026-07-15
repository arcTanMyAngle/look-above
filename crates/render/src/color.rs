//! The map background, and the sRGB conversion its clear value needs.

/// Background of the map view, authored as nonlinear sRGB (`#0A0E14`).
///
/// docs/01 asks for a dark, desaturated field so aircraft stay the brightest thing on
/// screen; it does not fix a shade, so this one is ours (`DECISION_LOG` 2026-07-15).
const BACKGROUND_SRGB: [u8; 3] = [0x0A, 0x0E, 0x14];

/// The sRGB electro-optical transfer function (IEC 61966-2-1): encoded → linear.
fn srgb_to_linear(encoded: f64) -> f64 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
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
    let encoded = BACKGROUND_SRGB.map(|channel| f64::from(channel) / 255.0);
    let [r, g, b] = if format.is_srgb() {
        encoded.map(srgb_to_linear)
    } else {
        encoded
    };

    wgpu::Color { r, g, b, a: 1.0 }
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
}
