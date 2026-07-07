//! Text rendering: lay out a string's glyphs, rasterize them to a coverage
//! mask (via `ab_glyph`), then composite that mask in linear light through the
//! same path shapes use ([`crate::draw::composite_mask`]).
//!
//! Layout is intentionally simple: left-to-right with kerning, `\n` for new
//! lines, anchored at `(x, y)` by `align_x`/`align_y`. Shaping (bidi, ligatures,
//! complex scripts) is a later concern; Latin + Cyrillic look correct.

use crate::draw;
use crate::hash::ContentHash;
use crate::tile::TiledImage;
use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use craws_domain::{AlignX, AlignY, Rgba8};

/// Engine-side text parameters (built from `OpSpec::DrawText`).
pub struct TextParams {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: Rgba8,
    pub font_size: f32,
    pub align_x: AlignX,
    pub align_y: AlignY,
    pub line_height: Option<f32>,
}

pub fn draw(
    input: &TiledImage,
    p: &TextParams,
    font: &FontVec,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let scale = PxScale::from(p.font_size.max(1.0));
    let sf = font.as_scaled(scale);
    let ascent = sf.ascent();
    let line_height = p.line_height.unwrap_or_else(|| sf.height() + sf.line_gap());

    let lines: Vec<&str> = p.text.split('\n').collect();
    let width_of = |line: &str| -> f32 {
        let mut w = 0.0;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(pv) = prev {
                w += sf.kern(pv, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    };
    let widths: Vec<f32> = lines.iter().map(|l| width_of(l)).collect();
    let block_h = line_height * lines.len() as f32;

    // top-of-block Y so the anchor lands per align_y
    let y_top = match p.align_y {
        AlignY::Top => p.y,
        AlignY::Middle => p.y - block_h / 2.0,
        AlignY::Bottom => p.y - block_h,
        AlignY::Baseline => p.y - ascent,
    };

    // place every glyph; track the union pixel bbox
    let mut outlines = Vec::new();
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (i, line) in lines.iter().enumerate() {
        let baseline = y_top + ascent + i as f32 * line_height;
        let mut pen = match p.align_x {
            AlignX::Left => p.x,
            AlignX::Center => p.x - widths[i] / 2.0,
            AlignX::Right => p.x - widths[i],
        };
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(pv) = prev {
                pen += sf.kern(pv, id);
            }
            let glyph = id.with_scale_and_position(scale, point(pen, baseline));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let b = outlined.px_bounds();
                minx = minx.min(b.min.x);
                miny = miny.min(b.min.y);
                maxx = maxx.max(b.max.x);
                maxy = maxy.max(b.max.y);
                outlines.push(outlined);
            }
            pen += sf.h_advance(id);
            prev = Some(id);
        }
    }
    if outlines.is_empty() {
        return draw::clone_all(input, tile_hash); // whitespace-only or empty
    }

    // mask rect (image coords; may extend off-canvas — composite_mask clips)
    let ox = minx.floor() as i32;
    let oy = miny.floor() as i32;
    let mw = (maxx.ceil() as i32 - ox).max(1) as u32;
    let mh = (maxy.ceil() as i32 - oy).max(1) as u32;
    let mut mask = vec![0.0f32; mw as usize * mh as usize];
    for outlined in &outlines {
        let b = outlined.px_bounds();
        let gx0 = b.min.x.floor() as i32 - ox;
        let gy0 = b.min.y.floor() as i32 - oy;
        outlined.draw(|gx, gy, cov| {
            let mx = gx0 + gx as i32;
            let my = gy0 + gy as i32;
            if mx >= 0 && my >= 0 && (mx as u32) < mw && (my as u32) < mh {
                let idx = my as usize * mw as usize + mx as usize;
                mask[idx] = mask[idx].max(cov); // union overlapping glyph coverage
            }
        });
    }
    draw::composite_mask(input, ox, oy, mw, mh, &mask, p.color, tile_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;
    use crate::tile::TiledImage;
    use craws_domain::Size;

    fn hash_by_index(i: u32) -> ContentHash {
        digest_bytes(&i.to_le_bytes())
    }

    fn blank(w: u32, h: u32) -> TiledImage {
        TiledImage::from_flat_f32(Size::new(w, h), &vec![0.0f32; (w * h * 4) as usize], hash_by_index)
    }

    fn params(text: &str) -> TextParams {
        TextParams {
            x: 20.0,
            y: 60.0,
            text: text.into(),
            color: Rgba8::rgb(255, 255, 255),
            font_size: 48.0,
            align_x: AlignX::Left,
            align_y: AlignY::Baseline,
            line_height: None,
        }
    }

    #[test]
    fn text_paints_some_pixels_and_keeps_size() {
        let font = crate::fonts::load(None);
        let img = blank(400, 120);
        let out = draw(&img, &params("Hello"), &font, &hash_by_index);
        assert_eq!(out.size(), Size::new(400, 120), "text never resizes the image");
        // some pixels within the text area became opaque-ish
        let mut painted = 0;
        for x in 0..300 {
            for y in 20..100 {
                if out.pixel(x, y)[3] > 0.3 {
                    painted += 1;
                }
            }
        }
        assert!(painted > 200, "expected glyph coverage, got {painted} painted pixels");
    }

    #[test]
    fn empty_and_whitespace_are_noops() {
        let font = crate::fonts::load(None);
        let img = blank(100, 100);
        for t in ["", "   "] {
            let out = draw(&img, &params(t), &font, &hash_by_index);
            let any = (0..100).any(|x| (0..100).any(|y| out.pixel(x, y)[3] > 0.0));
            assert!(!any, "{t:?} should paint nothing");
        }
    }

    #[test]
    fn cyrillic_renders() {
        let font = crate::fonts::load(None);
        let img = blank(400, 120);
        let out = draw(&img, &params("Привет"), &font, &hash_by_index);
        let painted = (0..380).any(|x| (10..110).any(|y| out.pixel(x, y)[3] > 0.3));
        assert!(painted, "Cyrillic should render with the embedded font");
    }

    #[test]
    fn center_alignment_shifts_left_of_anchor() {
        let font = crate::fonts::load(None);
        let img = blank(400, 120);
        let mut p = params("WW");
        p.x = 200.0;
        p.align_x = AlignX::Center;
        let out = draw(&img, &p, &font, &hash_by_index);
        // centered text straddles x=200: paint exists both left and right of it
        let left = (150..200).any(|x| (10..110).any(|y| out.pixel(x, y)[3] > 0.3));
        let right = (200..250).any(|x| (10..110).any(|y| out.pixel(x, y)[3] > 0.3));
        assert!(left && right, "centered text should straddle the anchor (l={left} r={right})");
    }
}
