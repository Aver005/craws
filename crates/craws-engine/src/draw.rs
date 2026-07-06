//! Vector annotation rasterizer — rectangles, ellipses, lines, arrows.
//!
//! Shapes are drawn by analytic **signed-distance fields**: for each pixel the
//! distance to the shape edge gives a 1px anti-aliased coverage, and that
//! coverage composites the paint (converted to linear premultiplied) over the
//! destination in **linear light** — so edges blend correctly and HDR headroom
//! is preserved (no sRGB round-trip, unlike a general 2D canvas lib).
//!
//! Only tiles overlapping the shape's bounding box are recomputed; the rest are
//! reused (Arc clone). The same coverage→composite path will drive text glyph
//! masks in the next iteration.

use crate::color::rgba8_to_linear_premul;
use crate::hash::ContentHash;
use crate::tile::{grid_dims, tile_dims, Tile, TileRef, TiledImage, TILE_SIZE};
use craws_domain::{Rgba8, Size};
use rayon::prelude::*;
use std::sync::Arc;

/// A color ready to composite: linear premultiplied RGB + straight alpha.
#[derive(Clone, Copy)]
struct Paint {
    premul: [f32; 4],
    alpha: f32,
}

impl Paint {
    fn new(c: Rgba8) -> Self {
        let premul = rgba8_to_linear_premul(c);
        Self { premul, alpha: premul[3] }
    }
}

/// One paint + its per-pixel coverage function. Layers composite in order.
struct Layer<'a> {
    paint: Paint,
    cov: Box<dyn Fn(f32, f32) -> f32 + Sync + 'a>,
}

/// Source-over composite of `paint` at coverage `c` onto a premultiplied pixel.
#[inline]
fn over(dst: &mut [f32], paint: &Paint, c: f32) {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0 {
        return;
    }
    let a_eff = paint.alpha * c;
    let inv = 1.0 - a_eff;
    // premul already carries alpha; * c scales it to the effective alpha
    dst[0] = paint.premul[0] * c + dst[0] * inv;
    dst[1] = paint.premul[1] * c + dst[1] * inv;
    dst[2] = paint.premul[2] * c + dst[2] * inv;
    dst[3] = a_eff + dst[3] * inv;
}

/// Integer, image-clamped bounding box `[x0,x1) × [y0,y1)`.
struct BBox {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl BBox {
    fn clamp(min_x: f32, min_y: f32, max_x: f32, max_y: f32, size: Size) -> Option<Self> {
        // pad by AA margin
        let x0 = (min_x - 1.0).floor().max(0.0) as u32;
        let y0 = (min_y - 1.0).floor().max(0.0) as u32;
        let x1 = ((max_x + 1.0).ceil().max(0.0) as u32).min(size.width);
        let y1 = ((max_y + 1.0).ceil().max(0.0) as u32).min(size.height);
        (x1 > x0 && y1 > y0).then_some(BBox { x0, y0, x1, y1 })
    }
}

/// Composite the layers over `input` within `bbox`, tile by tile. Tiles that do
/// not overlap the bbox are reused unchanged.
fn render(
    input: &TiledImage,
    bbox: BBox,
    layers: &[Layer],
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let size = input.size();
    let (cols, rows) = grid_dims(size);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(size, col, row);
            let (ox, oy) = (col * TILE_SIZE, row * TILE_SIZE);

            // tile ∩ bbox in local tile coordinates
            let lx0 = bbox.x0.saturating_sub(ox);
            let ly0 = bbox.y0.saturating_sub(oy);
            let lx1 = bbox.x1.min(ox + tw).saturating_sub(ox);
            let ly1 = bbox.y1.min(oy + th).saturating_sub(oy);
            if lx1 <= lx0 || ly1 <= ly0 {
                let src = &input.tiles()[index as usize];
                return TileRef { hash: tile_hash(index), tile: Arc::clone(&src.tile) };
            }

            let mut px = input.tiles()[index as usize].tile.px.to_vec();
            for ly in ly0..ly1 {
                let gy = (oy + ly) as f32 + 0.5;
                for lx in lx0..lx1 {
                    let gx = (ox + lx) as f32 + 0.5;
                    let off = (ly as usize * tw as usize + lx as usize) * 4;
                    for layer in layers {
                        let c = (layer.cov)(gx, gy);
                        if c > 0.0 {
                            over(&mut px[off..off + 4], &layer.paint, c);
                        }
                    }
                }
            }
            TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(size, tiles)
}

// ── SDF primitives (all in pixel units; negative = inside) ──────────────────

/// Rounded-box SDF (centered at c, half-extents h, corner radius r).
fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    qx.max(qy).min(0.0) + (ax * ax + ay * ay).sqrt() - r
}

/// First-order signed-distance estimate to an ellipse edge (exact for circles).
fn sd_ellipse(px: f32, py: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let dx = px - cx;
    let dy = py - cy;
    let f = (dx * dx) / (rx * rx) + (dy * dy) / (ry * ry) - 1.0;
    let gx = 2.0 * dx / (rx * rx);
    let gy = 2.0 * dy / (ry * ry);
    let g = (gx * gx + gy * gy).sqrt();
    if g > 1e-9 {
        f / g
    } else {
        f
    }
}

/// Unsigned distance from a point to segment a→b.
fn sd_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (pax, pay) = (px - ax, py - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let denom = bax * bax + bay * bay;
    let h = if denom > 1e-9 { ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt()
}

/// 1px anti-aliased coverage from a signed distance (edge at d = 0).
#[inline]
fn aa(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

// ── public entry points (one per draw OpSpec) ───────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn rect(
    input: &TiledImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    corner_radius: f32,
    fill: Option<Rgba8>,
    stroke: Option<Rgba8>,
    stroke_width: f32,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let r = corner_radius.clamp(0.0, hw.min(hh));
    let half = stroke_width / 2.0;
    let mut layers: Vec<Layer> = Vec::new();
    if let Some(c) = fill {
        layers.push(Layer {
            paint: Paint::new(c),
            cov: Box::new(move |px, py| aa(sd_round_rect(px, py, cx, cy, hw, hh, r))),
        });
    }
    if let Some(c) = stroke {
        layers.push(Layer {
            paint: Paint::new(c),
            cov: Box::new(move |px, py| aa(sd_round_rect(px, py, cx, cy, hw, hh, r).abs() - half)),
        });
    }
    let margin = half.max(0.0);
    let Some(bbox) = BBox::clamp(x - margin, y - margin, x + w + margin, y + h + margin, input.size())
    else {
        return clone_all(input, tile_hash);
    };
    render(input, bbox, &layers, tile_hash)
}

#[allow(clippy::too_many_arguments)]
pub fn ellipse(
    input: &TiledImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    fill: Option<Rgba8>,
    stroke: Option<Rgba8>,
    stroke_width: f32,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (rx, ry) = (w / 2.0, h / 2.0);
    let half = stroke_width / 2.0;
    let mut layers: Vec<Layer> = Vec::new();
    if rx > 0.0 && ry > 0.0 {
        if let Some(c) = fill {
            layers.push(Layer {
                paint: Paint::new(c),
                cov: Box::new(move |px, py| aa(sd_ellipse(px, py, cx, cy, rx, ry))),
            });
        }
        if let Some(c) = stroke {
            layers.push(Layer {
                paint: Paint::new(c),
                cov: Box::new(move |px, py| aa(sd_ellipse(px, py, cx, cy, rx, ry).abs() - half)),
            });
        }
    }
    let margin = half.max(0.0);
    let Some(bbox) = BBox::clamp(x - margin, y - margin, x + w + margin, y + h + margin, input.size())
    else {
        return clone_all(input, tile_hash);
    };
    render(input, bbox, &layers, tile_hash)
}

#[allow(clippy::too_many_arguments)]
pub fn line(
    input: &TiledImage,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: Rgba8,
    thickness: f32,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let half = thickness / 2.0;
    let layers = [Layer {
        paint: Paint::new(color),
        cov: Box::new(move |px, py| aa(sd_segment(px, py, x1, y1, x2, y2) - half)) as Box<_>,
    }];
    let Some(bbox) = BBox::clamp(x1.min(x2) - half, y1.min(y2) - half, x1.max(x2) + half, y1.max(y2) + half, input.size())
    else {
        return clone_all(input, tile_hash);
    };
    render(input, bbox, &layers, tile_hash)
}

#[allow(clippy::too_many_arguments)]
pub fn arrow(
    input: &TiledImage,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: Rgba8,
    thickness: f32,
    head_length: f32,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let half = thickness / 2.0;
    // arrowhead: two short segments from the tip, ±28° off the shaft direction
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt();
    let (hlx, hly, hrx, hry) = if len > 1e-6 && head_length > 0.0 {
        let (ux, uy) = (dx / len, dy / len);
        let (c, s) = (0.88f32, 0.475f32); // cos/sin ~28.4°
        let lx = x2 - head_length * (ux * c - uy * s);
        let ly = y2 - head_length * (ux * s + uy * c);
        let rx = x2 - head_length * (ux * c + uy * s);
        let ry = y2 - head_length * (ux * (-s) + uy * c);
        (lx, ly, rx, ry)
    } else {
        (x2, y2, x2, y2)
    };

    let cov = move |px: f32, py: f32| {
        let shaft = sd_segment(px, py, x1, y1, x2, y2) - half;
        let hl = sd_segment(px, py, x2, y2, hlx, hly) - half;
        let hr = sd_segment(px, py, x2, y2, hrx, hry) - half;
        // union of the three strokes = the smallest distance
        aa(shaft.min(hl).min(hr))
    };
    let layers = [Layer { paint: Paint::new(color), cov: Box::new(cov) as Box<_> }];

    let xs = [x1, x2, hlx, hrx];
    let ys = [y1, y2, hly, hry];
    let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for i in 0..4 {
        minx = minx.min(xs[i]);
        maxx = maxx.max(xs[i]);
        miny = miny.min(ys[i]);
        maxy = maxy.max(ys[i]);
    }
    let Some(bbox) = BBox::clamp(minx - half, miny - half, maxx + half, maxy + half, input.size()) else {
        return clone_all(input, tile_hash);
    };
    render(input, bbox, &layers, tile_hash)
}

/// Fallback when a shape is entirely off-canvas: reuse every tile under new hashes.
fn clone_all(input: &TiledImage, tile_hash: &(dyn Fn(u32) -> ContentHash + Sync)) -> TiledImage {
    let tiles = input
        .tiles()
        .iter()
        .enumerate()
        .map(|(i, t)| TileRef { hash: tile_hash(i as u32), tile: Arc::clone(&t.tile) })
        .collect();
    TiledImage::new(input.size(), tiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;

    fn hash_by_index(i: u32) -> ContentHash {
        digest_bytes(&i.to_le_bytes())
    }

    fn blank(w: u32, h: u32) -> TiledImage {
        let flat = vec![0.0f32; (w * h * 4) as usize]; // transparent
        TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index)
    }

    #[test]
    fn filled_rect_paints_interior_opaque_red() {
        let img = blank(300, 300); // spans tiles
        let out = rect(&img, 40.0, 40.0, 200.0, 180.0, 0.0, Some(Rgba8::rgb(255, 0, 0)), None, 0.0, &hash_by_index);
        // deep interior is opaque red (alpha 1, r≈1 linear)
        let p = out.pixel(140, 130);
        assert!(p[3] > 0.99, "interior alpha {}", p[3]);
        assert!(p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01, "interior color {p:?}");
        // far outside is untouched (transparent)
        assert_eq!(out.pixel(5, 5)[3], 0.0);
    }

    #[test]
    fn stroke_only_leaves_center_empty() {
        let img = blank(200, 200);
        let out = rect(&img, 20.0, 20.0, 160.0, 160.0, 0.0, None, Some(Rgba8::rgb(0, 255, 0)), 6.0, &hash_by_index);
        assert_eq!(out.pixel(100, 100)[3], 0.0, "center stays empty for stroke-only");
        assert!(out.pixel(100, 21)[3] > 0.5, "top edge is painted");
    }

    #[test]
    fn ellipse_inside_vs_outside() {
        let img = blank(200, 200);
        let out = ellipse(&img, 20.0, 20.0, 160.0, 160.0, Some(Rgba8::rgb(0, 0, 255)), None, 0.0, &hash_by_index);
        assert!(out.pixel(100, 100)[3] > 0.99, "center filled");
        assert_eq!(out.pixel(25, 25)[3], 0.0, "corner outside the ellipse is empty");
    }

    #[test]
    fn line_and_arrow_paint_along_path() {
        let img = blank(200, 60);
        let out = line(&img, 10.0, 30.0, 190.0, 30.0, Rgba8::rgb(255, 255, 255), 4.0, &hash_by_index);
        assert!(out.pixel(100, 30)[3] > 0.9, "on the line");
        assert_eq!(out.pixel(100, 55)[3], 0.0, "away from the line");

        let a = arrow(&img, 10.0, 30.0, 180.0, 30.0, Rgba8::rgb(255, 255, 0), 4.0, 20.0, &hash_by_index);
        assert!(a.pixel(100, 30)[3] > 0.9, "shaft painted");
        // arrowhead spreads off the shaft line near the tip (the two V segments)
        assert!(a.pixel(166, 23)[3] > 0.3, "upper head segment painted");
        assert!(a.pixel(166, 37)[3] > 0.3, "lower head segment painted");
    }

    #[test]
    fn semi_transparent_blends_over_background() {
        // opaque white background, draw 50% black rect → mid gray
        let flat: Vec<f32> = std::iter::repeat_n([1.0f32, 1.0, 1.0, 1.0], 100 * 100).flatten().collect();
        let img = TiledImage::from_flat_f32(Size::new(100, 100), &flat, hash_by_index);
        let out = rect(&img, 10.0, 10.0, 80.0, 80.0, 0.0, Some(Rgba8::new(0, 0, 0, 128)), None, 0.0, &hash_by_index);
        let p = out.pixel(50, 50);
        assert!(p[3] > 0.99, "stays opaque");
        assert!((p[0] - 0.5).abs() < 0.05, "≈50% blend in linear: {}", p[0]);
    }
}
