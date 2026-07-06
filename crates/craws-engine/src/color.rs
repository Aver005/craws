//! sRGB ⇄ linear-light conversions.
//!
//! Decode (u8 → f32) goes through a 256-entry LUT. Encode (f32 → u8) uses the
//! exact transfer formula; it runs tile-parallel in practice. (A quantized
//! encode LUT is a known micro-opt candidate — measure first.)

use std::sync::LazyLock;

static SRGB_TO_LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut lut = [0.0f32; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    }
    lut
});

#[inline]
pub fn srgb8_to_linear(v: u8) -> f32 {
    SRGB_TO_LINEAR[v as usize]
}

/// Convert an authored straight-alpha sRGB color to the engine's internal
/// linear-light **premultiplied** RGBA. Used by the rasterizer and compositor.
#[inline]
pub fn rgba8_to_linear_premul(c: craws_domain::Rgba8) -> [f32; 4] {
    let a = c.a as f32 / 255.0;
    [
        srgb8_to_linear(c.r) * a,
        srgb8_to_linear(c.g) * a,
        srgb8_to_linear(c.b) * a,
        a,
    ]
}

/// Clamps to [0, 1] (this is the only place the pipeline clamps) and encodes.
#[inline]
pub fn linear_to_srgb8(v: f32) -> u8 {
    let c = v.clamp(0.0, 1.0);
    let e = if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (e * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_roundtrip_is_exact() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb8(srgb8_to_linear(v)), v, "value {v}");
        }
    }

    #[test]
    fn encode_clamps_hdr_and_negatives() {
        assert_eq!(linear_to_srgb8(7.5), 255);
        assert_eq!(linear_to_srgb8(-0.2), 0);
    }

    #[test]
    fn linear_anchors() {
        assert_eq!(srgb8_to_linear(0), 0.0);
        assert!((srgb8_to_linear(255) - 1.0).abs() < 1e-6);
        // mid-gray sRGB 128 ≈ 0.2158 linear (not 0.5!) — guards against gamma bugs
        assert!((srgb8_to_linear(128) - 0.2158).abs() < 1e-3);
    }
}
