//! Operation kernels. Pure functions over tiles/images — no cache, no I/O.
//!
//! Two shapes exist in M0:
//! - **pointwise** (`exposure`, `grayscale`): tile → tile, embarrassingly parallel,
//!   cached per-tile by the engine;
//! - **global** (`crop`, `resize`): whole image → whole image, cached per output
//!   tile under one signature. (Crop is actually neighborhood-local — tightening
//!   its hashing to only the overlapped source tiles is a known improvement.)

use crate::color::{linear_to_srgb_f32, rgba8_to_linear_premul, srgb_to_linear_f32};
use crate::hash::{self, ContentHash};
use crate::resample::{self, Contribs};
use crate::tile::{grid_dims, tile_dims, Tile, TileRef, TiledImage, TILE_SIZE};
use craws_domain::{Filter, FlipAxis, Rect, Rgba8, Size};
use rayon::prelude::*;
use std::sync::Arc;

/// Linear multiply of RGB by `factor` (premultiplied ⇒ alpha untouched).
/// No clamping — HDR headroom survives until u8 export.
pub fn exposure(t: &Tile, factor: f32) -> Tile {
    let mut px = t.px.to_vec();
    for p in px.chunks_exact_mut(4) {
        p[0] *= factor;
        p[1] *= factor;
        p[2] *= factor;
    }
    Tile::new(t.width, t.height, px.into_boxed_slice())
}

/// Rec.709 relative luminance, computed in linear light.
/// (A linear combination, so it commutes with premultiplication.)
pub fn grayscale(t: &Tile) -> Tile {
    let mut px = t.px.to_vec();
    for p in px.chunks_exact_mut(4) {
        let y = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        p[0] = y;
        p[1] = y;
        p[2] = y;
    }
    Tile::new(t.width, t.height, px.into_boxed_slice())
}

/// Row-major 3×3 hue-rotation matrix (SVG `feColorMatrix` "hueRotate" constants).
/// A rotation about the luma axis, so it preserves luminance and — being linear —
/// commutes with premultiplication (apply it straight to premultiplied RGB).
pub fn hue_matrix(degrees: f32) -> [f32; 9] {
    let r = degrees.to_radians();
    let (c, s) = (r.cos(), r.sin());
    [
        0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928,
        0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283,
        0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072,
    ]
}

/// Apply a hue-rotation matrix. RGB only; alpha (and premultiplication) untouched.
/// Not clamped — out-of-gamut values keep their headroom until u8 export.
pub fn hue_rotate(t: &Tile, m: &[f32; 9]) -> Tile {
    let mut px = t.px.to_vec();
    for p in px.chunks_exact_mut(4) {
        let (r, g, b) = (p[0], p[1], p[2]);
        p[0] = m[0] * r + m[1] * g + m[2] * b;
        p[1] = m[3] * r + m[4] * g + m[5] * b;
        p[2] = m[6] * r + m[7] * g + m[8] * b;
    }
    Tile::new(t.width, t.height, px.into_boxed_slice())
}

/// Photographic negative: invert in perceptual (sRGB) space, not linear — so
/// mid-gray maps to mid-gray as users expect. Unpremultiply → sRGB → `1−x` →
/// linear → re-premultiply, per pixel. Fully transparent pixels are left alone.
pub fn invert(t: &Tile) -> Tile {
    let mut px = t.px.to_vec();
    for p in px.chunks_exact_mut(4) {
        let a = p[3];
        if a <= 0.0 {
            continue;
        }
        let inv = 1.0 / a;
        for ch in p.iter_mut().take(3) {
            let straight = *ch * inv;
            let inverted = 1.0 - linear_to_srgb_f32(straight);
            *ch = srgb_to_linear_f32(inverted) * a;
        }
    }
    Tile::new(t.width, t.height, px.into_boxed_slice())
}

#[inline]
fn lerpf(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Apply a per-pixel transform in perceptual sRGB: unpremultiply → sRGB →
/// `f(straight sRGB rgb)` → linear → re-premultiply. Alpha and transparent pixels
/// untouched. This is the space tonal ops (brightness/levels/curves/saturation/
/// gradient-map) are authored in — matching photo-editor expectations.
fn map_srgb(t: &Tile, f: impl Fn([f32; 3]) -> [f32; 3]) -> Tile {
    let mut px = t.px.to_vec();
    for p in px.chunks_exact_mut(4) {
        let a = p[3];
        if a <= 0.0 {
            continue;
        }
        let inv = 1.0 / a;
        let s = [linear_to_srgb_f32(p[0] * inv), linear_to_srgb_f32(p[1] * inv), linear_to_srgb_f32(p[2] * inv)];
        let o = f(s);
        p[0] = srgb_to_linear_f32(o[0]) * a;
        p[1] = srgb_to_linear_f32(o[1]) * a;
        p[2] = srgb_to_linear_f32(o[2]) * a;
    }
    Tile::new(t.width, t.height, px.into_boxed_slice())
}

/// Additive `brightness` and a `contrast` S-curve around mid-gray, in sRGB.
pub fn brightness_contrast(t: &Tile, brightness: f32, contrast: f32) -> Tile {
    let slope = 1.0 + contrast;
    map_srgb(t, move |c| {
        let g = |x: f32| ((x - 0.5) * slope + 0.5 + brightness).clamp(0.0, 1.0);
        [g(c[0]), g(c[1]), g(c[2])]
    })
}

/// Saturation around Rec.601 luma (1 = identity, 0 = gray, >1 boosts), in sRGB.
pub fn saturation(t: &Tile, amount: f32) -> Tile {
    map_srgb(t, move |c| {
        let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        let s = |x: f32| (l + (x - l) * amount).clamp(0.0, 1.0);
        [s(c[0]), s(c[1]), s(c[2])]
    })
}

/// Levels remap in sRGB: input black/white, gamma, output black/white (per channel).
pub fn levels(t: &Tile, in_black: f32, in_white: f32, gamma: f32, out_black: f32, out_white: f32) -> Tile {
    let span = (in_white - in_black).abs().max(1e-6);
    let inv_gamma = 1.0 / gamma;
    map_srgb(t, move |c| {
        let f = |x: f32| {
            let n = ((x - in_black) / span).clamp(0.0, 1.0);
            (n.powf(inv_gamma) * (out_white - out_black) + out_black).clamp(0.0, 1.0)
        };
        [f(c[0]), f(c[1]), f(c[2])]
    })
}

/// Build a 256-entry sRGB tone LUT from control points (x, y in 0..1). Sorted by x;
/// piecewise-linear between points; held flat outside. Empty points ⇒ identity.
pub fn build_curve_lut(points: &[[f32; 2]]) -> [f32; 256] {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    let mut lut = [0.0f32; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        let x = i as f32 / 255.0;
        *v = if pts.is_empty() {
            x
        } else if x <= pts[0][0] {
            pts[0][1]
        } else if x >= pts[pts.len() - 1][0] {
            pts[pts.len() - 1][1]
        } else {
            let k = pts.iter().position(|p| p[0] >= x).unwrap();
            let (a, b) = (pts[k - 1], pts[k]);
            let span = (b[0] - a[0]).max(1e-6);
            lerpf(a[1], b[1], (x - a[0]) / span)
        }
        .clamp(0.0, 1.0);
    }
    lut
}

/// Apply a precomputed 256-entry tone LUT per channel, in sRGB.
pub fn apply_curve(t: &Tile, lut: &[f32; 256]) -> Tile {
    map_srgb(t, |c| {
        let s = |x: f32| lut[(x.clamp(0.0, 1.0) * 255.0 + 0.5) as usize & 0xFF];
        [s(c[0]), s(c[1]), s(c[2])]
    })
}

/// Linear per-channel gains from temperature (blue↔amber) and tint (green↔magenta).
pub fn white_balance_gains(temperature: f32, tint: f32) -> [f32; 3] {
    let k = 0.4;
    [(1.0 + k * temperature).max(0.0), (1.0 - k * tint).max(0.0), (1.0 - k * temperature).max(0.0)]
}

/// Linear per-channel multiply (premultiplication-safe; alpha untouched, no clamp).
pub fn white_balance(t: &Tile, gains: [f32; 3]) -> Tile {
    let mut px = t.px.to_vec();
    for p in px.chunks_exact_mut(4) {
        p[0] *= gains[0];
        p[1] *= gains[1];
        p[2] *= gains[2];
    }
    Tile::new(t.width, t.height, px.into_boxed_slice())
}

/// Map each pixel's sRGB luminance through a `low → (mid) → high` color gradient.
pub fn gradient_map(t: &Tile, low: Rgba8, high: Rgba8, mid: Option<Rgba8>) -> Tile {
    let srgb = |c: Rgba8| [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0];
    let (lo, hi) = (srgb(low), srgb(high));
    let md = mid.map(srgb);
    map_srgb(t, move |c| {
        let l = (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]).clamp(0.0, 1.0);
        match md {
            Some(m) if l < 0.5 => {
                let u = l / 0.5;
                [lerpf(lo[0], m[0], u), lerpf(lo[1], m[1], u), lerpf(lo[2], m[2], u)]
            }
            Some(m) => {
                let u = (l - 0.5) / 0.5;
                [lerpf(m[0], hi[0], u), lerpf(m[1], hi[1], u), lerpf(m[2], hi[2], u)]
            }
            None => [lerpf(lo[0], hi[0], l), lerpf(lo[1], hi[1], l), lerpf(lo[2], hi[2], l)],
        }
    })
}

/// Gather `rect` from the source grid into a fresh grid. Row-run copies: each
/// output row is assembled from at most a few contiguous source spans.
pub fn crop(
    input: &TiledImage,
    rect: Rect,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let out_size = rect.size();
    let (cols, rows) = grid_dims(out_size);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(out_size, col, row);
            let mut px = vec![0.0f32; (tw * th * 4) as usize];
            for oy in 0..th {
                let sy = rect.y + row * TILE_SIZE + oy;
                let s_row = sy / TILE_SIZE;
                let s_ry = (sy % TILE_SIZE) as usize;
                let mut ox = 0u32;
                while ox < tw {
                    let sx = rect.x + col * TILE_SIZE + ox;
                    let st = &input.tile_at(sx / TILE_SIZE, s_row).tile;
                    let s_rx = (sx % TILE_SIZE) as usize;
                    let run = ((tw - ox) as usize).min(st.width as usize - s_rx);
                    let src_o = (s_ry * st.width as usize + s_rx) * 4;
                    let dst_o = (oy as usize * tw as usize + ox as usize) * 4;
                    px[dst_o..dst_o + run * 4].copy_from_slice(&st.px[src_o..src_o + run * 4]);
                    ox += run as u32;
                }
            }
            TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(out_size, tiles)
}

/// Whole-image resample. Runs on premultiplied linear f32 — the only correct
/// place to resample (no dark fringing, no gamma-space bleed).
///
/// Separable two-pass with precomputed, reused weights, both passes rayon-
/// parallel (`crate::resample`). The horizontal pass reads source rows straight
/// from the tile grid, so there is no giant intermediate flat copy of the input.
pub fn resize(
    input: &TiledImage,
    target: Size,
    filter: Filter,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let src = input.size();
    let cx = Contribs::new(src.width, target.width, filter);
    let cy = Contribs::new(src.height, target.height, filter);
    // horizontal: gather each source row from tiles → intermediate (out_w × in_h)
    let inter = resample::horizontal(&cx, src.width, src.height, |y, dst| {
        input.copy_row_into(y, dst);
    });
    // vertical: intermediate → final flat (out_w × out_h)
    let out = resample::vertical(&cy, &inter, target.width);
    TiledImage::from_flat_f32(target, &out, tile_hash)
}

#[inline]
fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Bilinear sample at source pixel-center coordinates `(fx, fy)`. Reads are
/// premultiplied linear, so blending is correct (no dark fringes); samples that
/// fall outside the image contribute transparent, feathering the rotated edge.
fn sample_bilinear(img: &TiledImage, fx: f32, fy: f32) -> [f32; 4] {
    let (x, y) = (fx - 0.5, fy - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (w, h) = (img.size().width as i32, img.size().height as i32);
    let (ix0, iy0) = (x0 as i32, y0 as i32);
    let get = |ix: i32, iy: i32| -> [f32; 4] {
        if ix < 0 || iy < 0 || ix >= w || iy >= h {
            [0.0; 4]
        } else {
            img.pixel(ix as u32, iy as u32)
        }
    };
    let top = lerp4(get(ix0, iy0), get(ix0 + 1, iy0), tx);
    let bot = lerp4(get(ix0, iy0 + 1), get(ix0 + 1, iy0 + 1), tx);
    lerp4(top, bot, ty)
}

/// Rotate about the center by `degrees` (positive = clockwise), producing an
/// image of the pre-validated `out` size. Multiples of 90° are exact integer
/// gathers; other angles inverse-map each output pixel and bilinear-sample the
/// source. Whole-image op: all output tiles share one signature.
pub fn rotate(
    input: &TiledImage,
    out: Size,
    degrees: f32,
    _expand: bool,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let src = input.size();
    let norm = degrees.rem_euclid(360.0);
    let q = (norm / 90.0).round();
    let ortho = (norm - q * 90.0).abs() < 1e-3;
    let k = (q as i64).rem_euclid(4);
    let r = norm.to_radians();
    let (sn, cs) = (r.sin(), r.cos());
    let (ocx, ocy) = (out.width as f32 / 2.0, out.height as f32 / 2.0);
    let (scx, scy) = (src.width as f32 / 2.0, src.height as f32 / 2.0);

    let (cols, rows) = grid_dims(out);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(out, col, row);
            let (ox0, oy0) = (col * TILE_SIZE, row * TILE_SIZE);
            let mut px = vec![0.0f32; (tw * th * 4) as usize];
            for ly in 0..th {
                for lx in 0..tw {
                    let (oxp, oyp) = (ox0 + lx, oy0 + ly);
                    let p = if ortho {
                        // exact permutation (out coords → source coords)
                        let (sx, sy) = match k {
                            1 => (oyp, (src.height - 1) - oxp),
                            2 => ((src.width - 1) - oxp, (src.height - 1) - oyp),
                            3 => ((src.width - 1) - oyp, oxp),
                            _ => (oxp, oyp),
                        };
                        input.pixel(sx, sy)
                    } else {
                        // inverse-rotate the centered output coord by -θ, then sample
                        let dx = oxp as f32 + 0.5 - ocx;
                        let dy = oyp as f32 + 0.5 - ocy;
                        let sxf = scx + cs * dx + sn * dy;
                        let syf = scy - sn * dx + cs * dy;
                        sample_bilinear(input, sxf, syf)
                    };
                    let o = ((ly * tw + lx) * 4) as usize;
                    px[o..o + 4].copy_from_slice(&p);
                }
            }
            TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(out, tiles)
}

/// Mirror across `axis`. Size-preserving whole-image gather.
pub fn flip(
    input: &TiledImage,
    axis: FlipAxis,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let size = input.size();
    let (w, h) = (size.width, size.height);
    let (cols, rows) = grid_dims(size);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(size, col, row);
            let (ox0, oy0) = (col * TILE_SIZE, row * TILE_SIZE);
            let mut px = vec![0.0f32; (tw * th * 4) as usize];
            for ly in 0..th {
                for lx in 0..tw {
                    let (oxp, oyp) = (ox0 + lx, oy0 + ly);
                    let (sx, sy) = match axis {
                        FlipAxis::Horizontal => ((w - 1) - oxp, oyp),
                        FlipAxis::Vertical => (oxp, (h - 1) - oyp),
                    };
                    let o = ((ly * tw + lx) * 4) as usize;
                    px[o..o + 4].copy_from_slice(&input.pixel(sx, sy));
                }
            }
            TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(size, tiles)
}

/// Place `input` at `(left, top)` on a canvas of `out` size filled with `color`.
/// The border is an exact solid fill; the interior is copied by row-runs (like
/// [`crop`]), so no resampling touches the original pixels.
pub fn pad(
    input: &TiledImage,
    out: Size,
    left: u32,
    top: u32,
    color: Rgba8,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let src = input.size();
    let fill = rgba8_to_linear_premul(color);
    let (cols, rows) = grid_dims(out);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(out, col, row);
            let (ox0, oy0) = (col * TILE_SIZE, row * TILE_SIZE);
            let mut px = vec![0.0f32; (tw * th * 4) as usize];
            for p in px.chunks_exact_mut(4) {
                p.copy_from_slice(&fill);
            }
            // this tile ∩ the source-image rect, in output space
            let rx0 = ox0.max(left);
            let ry0 = oy0.max(top);
            let rx1 = (ox0 + tw).min(left + src.width);
            let ry1 = (oy0 + th).min(top + src.height);
            if rx1 > rx0 && ry1 > ry0 {
                for oy in ry0..ry1 {
                    let sy = oy - top;
                    let s_row = sy / TILE_SIZE;
                    let s_ry = (sy % TILE_SIZE) as usize;
                    let ly = (oy - oy0) as usize;
                    let mut ox = rx0;
                    while ox < rx1 {
                        let sx = ox - left;
                        let st = &input.tile_at(sx / TILE_SIZE, s_row).tile;
                        let s_rx = (sx % TILE_SIZE) as usize;
                        let run = ((rx1 - ox) as usize).min(st.width as usize - s_rx);
                        let src_o = (s_ry * st.width as usize + s_rx) * 4;
                        let dst_o = (ly * tw as usize + (ox - ox0) as usize) * 4;
                        px[dst_o..dst_o + run * 4].copy_from_slice(&st.px[src_o..src_o + run * 4]);
                        ox += run as u32;
                    }
                }
            }
            TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(out, tiles)
}

/// Tightest rectangle enclosing all pixels that differ from the background
/// `reference` (linear premultiplied) by more than `tolerance` on any channel.
/// `reference` defaults to the top-left pixel — matching the familiar
/// "auto-trim the uniform border" behavior, transparent or solid alike.
/// Returns `None` when the whole image is background. One parallel pass; each
/// tile row is read as a contiguous slice.
pub fn content_bounds(input: &TiledImage, reference: Option<Rgba8>, tolerance: f32) -> Option<Rect> {
    let size = input.size();
    let (w, h) = (size.width, size.height);
    let (cols, _) = input.grid();
    let refpx = match reference {
        Some(c) => rgba8_to_linear_premul(c),
        None => input.pixel(0, 0),
    };
    let tol = tolerance.max(0.0);
    let is_bg = |p: &[f32]| {
        (p[0] - refpx[0]).abs() <= tol
            && (p[1] - refpx[1]).abs() <= tol
            && (p[2] - refpx[2]).abs() <= tol
            && (p[3] - refpx[3]).abs() <= tol
    };
    // per row: horizontal span of non-background pixels, or None
    let spans: Vec<Option<(u32, u32)>> = (0..h)
        .into_par_iter()
        .map(|y| {
            let trow = y / TILE_SIZE;
            let ty = (y % TILE_SIZE) as usize;
            let (mut lo, mut hi) = (None, 0u32);
            for col in 0..cols {
                let t = &input.tile_at(col, trow).tile;
                let tw = t.width as usize;
                let base = ty * tw * 4;
                for lx in 0..tw {
                    let o = base + lx * 4;
                    if !is_bg(&t.px[o..o + 4]) {
                        let gx = col * TILE_SIZE + lx as u32;
                        if lo.is_none() {
                            lo = Some(gx);
                        }
                        hi = gx;
                    }
                }
            }
            lo.map(|l| (l, hi))
        })
        .collect();

    let (mut miny, mut maxy) = (None, 0u32);
    let (mut minx, mut maxx) = (w, 0u32);
    for (y, span) in spans.iter().enumerate() {
        if let Some((l, r)) = *span {
            if miny.is_none() {
                miny = Some(y as u32);
            }
            maxy = y as u32;
            minx = minx.min(l);
            maxx = maxx.max(r);
        }
    }
    let miny = miny?;
    Some(Rect::new(minx, miny, maxx - minx + 1, maxy - miny + 1))
}

/// Crop away a uniform border. `None` result means the image is entirely
/// background (nothing to keep); an image with no trimmable border comes back
/// unchanged (cheap `Arc` reuse). Content-addressed by the trim rect + inputs,
/// so downstream ops stay cache-correct.
pub fn trim(input: &TiledImage, reference: Option<Rgba8>, tolerance: f32) -> Option<TiledImage> {
    let rect = content_bounds(input, reference, tolerance)?;
    let size = input.size();
    if rect.x == 0 && rect.y == 0 && rect.width == size.width && rect.height == size.height {
        return Some(input.clone());
    }
    let mut tag = [0u8; 16];
    tag[0..4].copy_from_slice(&rect.x.to_le_bytes());
    tag[4..8].copy_from_slice(&rect.y.to_le_bytes());
    tag[8..12].copy_from_slice(&rect.width.to_le_bytes());
    tag[12..16].copy_from_slice(&rect.height.to_le_bytes());
    let sig = hash::global_signature(hash::digest_bytes(&tag), input.tiles().iter().map(|t| &t.hash));
    Some(crop(input, rect, &|i| hash::global_tile_hash(sig, i)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;

    fn hash_by_index(i: u32) -> ContentHash {
        digest_bytes(&i.to_le_bytes())
    }

    #[test]
    fn exposure_doubles_linear_and_keeps_alpha() {
        let t = Tile::solid(4, 4, [0.25, 0.1, 0.5, 0.8]);
        let out = exposure(&t, 2.0);
        assert_eq!(out.px[0], 0.5);
        assert_eq!(out.px[1], 0.2);
        assert_eq!(out.px[2], 1.0);
        assert_eq!(out.px[3], 0.8, "alpha untouched");
    }

    #[test]
    fn exposure_preserves_hdr_headroom() {
        let t = Tile::solid(1, 1, [0.9, 0.9, 0.9, 1.0]);
        let out = exposure(&t, 4.0);
        assert_eq!(out.px[0], 3.6, "no mid-pipeline clamp");
    }

    #[test]
    fn grayscale_rec709() {
        let t = Tile::solid(1, 1, [1.0, 0.0, 0.0, 1.0]);
        let out = grayscale(&t);
        assert!((out.px[0] - 0.2126).abs() < 1e-6);
        assert_eq!(out.px[0], out.px[1]);
        assert_eq!(out.px[1], out.px[2]);
    }

    #[test]
    fn hue_rotate_identity_and_direction() {
        // 0° is the identity matrix
        let m0 = hue_matrix(0.0);
        let t = Tile::solid(1, 1, [0.4, 0.2, 0.7, 1.0]);
        let same = hue_rotate(&t, &m0);
        for i in 0..3 {
            assert!((same.px[i] - t.px[i]).abs() < 1e-5, "0° identity ch{i}");
        }
        // 120° rotates red toward green (green becomes the dominant channel)
        let red = Tile::solid(1, 1, [1.0, 0.0, 0.0, 1.0]);
        let out = hue_rotate(&red, &hue_matrix(120.0));
        assert!(out.px[1] > out.px[0] && out.px[1] > out.px[2], "red → green: {:?}", &out.px[..3]);
    }

    #[test]
    fn invert_is_perceptual_and_alpha_safe() {
        use crate::color::{linear_to_srgb8, srgb8_to_linear};
        // opaque mid-gray sRGB 128 inverts to sRGB 127 (perceptual, not linear)
        let mid = srgb8_to_linear(128);
        let t = Tile::solid(1, 1, [mid, mid, mid, 1.0]);
        let out = invert(&t);
        assert_eq!(linear_to_srgb8(out.px[0]), 127, "sRGB-space invert");
        // white → black
        let out = invert(&Tile::solid(1, 1, [1.0, 1.0, 1.0, 1.0]));
        assert!(out.px[0] < 1e-4 && out.px[3] > 0.99, "white inverts to opaque black: {:?}", out.px);
        // premultiplication respected: alpha .5, straight red → straight cyan
        let out = invert(&Tile::solid(1, 1, [0.5, 0.0, 0.0, 0.5]));
        assert!(out.px[0] < 1e-4 && (out.px[1] - 0.5).abs() < 1e-4 && (out.px[3] - 0.5).abs() < 1e-6, "{:?}", out.px);
        // fully transparent stays untouched
        let out = invert(&Tile::solid(1, 1, [0.0, 0.0, 0.0, 0.0]));
        assert_eq!(out.px, Tile::solid(1, 1, [0.0, 0.0, 0.0, 0.0]).px);
    }

    #[test]
    fn brightness_contrast_and_saturation() {
        use crate::color::{linear_to_srgb8, srgb8_to_linear};
        let mid = srgb8_to_linear(128);
        let t = Tile::solid(1, 1, [mid, mid, mid, 1.0]);
        assert!(linear_to_srgb8(brightness_contrast(&t, 0.2, 0.0).px[0]) > 128, "+brightness lifts");
        let dark = srgb8_to_linear(80);
        let td = Tile::solid(1, 1, [dark, dark, dark, 1.0]);
        assert!(linear_to_srgb8(brightness_contrast(&td, 0.0, 0.5).px[0]) < 80, "+contrast darkens shadows");

        // saturation 0 → equal channels; 1 → unchanged
        let col = Tile::solid(1, 1, [srgb8_to_linear(200), srgb8_to_linear(60), srgb8_to_linear(60), 1.0]);
        let g = saturation(&col, 0.0);
        assert!((g.px[0] - g.px[1]).abs() < 1e-3 && (g.px[1] - g.px[2]).abs() < 1e-3, "gray: {:?}", &g.px[..3]);
        let id = saturation(&col, 1.0);
        for i in 0..3 {
            assert!((id.px[i] - col.px[i]).abs() < 1e-3);
        }
    }

    #[test]
    fn levels_curves_wb_gradient() {
        use crate::color::{linear_to_srgb8, srgb8_to_linear};
        let mid = srgb8_to_linear(128);
        let t = Tile::solid(1, 1, [mid, mid, mid, 1.0]);
        // levels identity keeps 128; gamma 2 brightens midtones
        assert_eq!(linear_to_srgb8(levels(&t, 0.0, 1.0, 1.0, 0.0, 1.0).px[0]), 128);
        assert!(linear_to_srgb8(levels(&t, 0.0, 1.0, 2.0, 0.0, 1.0).px[0]) > 128, "gamma 2 brightens");
        // curves: identity keeps; a lifting curve brightens
        assert_eq!(linear_to_srgb8(apply_curve(&t, &build_curve_lut(&[[0.0, 0.0], [1.0, 1.0]])).px[0]), 128);
        assert!(linear_to_srgb8(apply_curve(&t, &build_curve_lut(&[[0.0, 0.3], [1.0, 1.0]])).px[0]) > 128);
        // white balance: warm boosts R over B
        let wb = white_balance(&Tile::solid(1, 1, [0.5, 0.5, 0.5, 1.0]), white_balance_gains(0.5, 0.0));
        assert!(wb.px[0] > wb.px[2], "warm: {:?}", &wb.px[..3]);
        // gradient map black→red on a gray → toward red
        let g = gradient_map(&t, Rgba8::rgb(0, 0, 0), Rgba8::rgb(255, 0, 0), None);
        assert!(g.px[0] > g.px[1] && g.px[0] > g.px[2], "toward red: {:?}", &g.px[..3]);
    }

    #[test]
    fn crop_is_pixel_exact_across_tile_boundaries() {
        // pixel (x, y) stores its own flat index in R — any misgather is loud
        let size = Size::new(600, 300);
        let mut flat = vec![0.0f32; size.area() as usize * 4];
        for y in 0..300u32 {
            for x in 0..600u32 {
                flat[((y * 600 + x) * 4) as usize] = (y * 600 + x) as f32;
                flat[((y * 600 + x) * 4 + 3) as usize] = 1.0;
            }
        }
        let img = TiledImage::from_flat_f32(size, &flat, hash_by_index);
        let rect = Rect::new(250, 100, 300, 150); // straddles the 256-boundary both ways
        let out = crop(&img, rect, &hash_by_index);
        assert_eq!(out.size(), Size::new(300, 150));
        for (x, y) in [(0, 0), (299, 149), (5, 149), (299, 0), (10, 60)] {
            let want = ((y + 100) * 600 + (x + 250)) as f32;
            assert_eq!(out.pixel(x, y)[0], want, "at ({x},{y})");
        }
    }

    #[test]
    fn resize_solid_stays_solid() {
        let size = Size::new(500, 300);
        let flat: Vec<f32> = std::iter::repeat_n([0.3f32, 0.6, 0.9, 1.0], size.area() as usize)
            .flatten()
            .collect();
        let img = TiledImage::from_flat_f32(size, &flat, hash_by_index);
        let out = resize(&img, Size::new(123, 77), Filter::Lanczos3, &hash_by_index);
        assert_eq!(out.size(), Size::new(123, 77));
        for (x, y) in [(0, 0), (61, 38), (122, 76)] {
            let p = out.pixel(x, y);
            assert!((p[0] - 0.3).abs() < 1e-4 && (p[1] - 0.6).abs() < 1e-4, "at ({x},{y}): {p:?}");
        }
    }

    /// Each pixel stores its own (x, y) in R/G — any misplacement is loud.
    fn coord_img(w: u32, h: u32) -> TiledImage {
        let mut flat = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                flat[o] = x as f32;
                flat[o + 1] = y as f32;
                flat[o + 3] = 1.0;
            }
        }
        TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index)
    }

    fn solid(w: u32, h: u32, rgba: [f32; 4]) -> TiledImage {
        let flat: Vec<f32> = std::iter::repeat_n(rgba, (w * h) as usize).flatten().collect();
        TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index)
    }

    #[test]
    fn rotate_90_clockwise_is_exact() {
        // 400×260 crosses tile boundaries; 90° cw ⇒ 260×400, top-left → top-right
        let img = coord_img(400, 260);
        let out = rotate(&img, Size::new(260, 400), 90.0, true, &hash_by_index);
        assert_eq!(out.size(), Size::new(260, 400));
        // out(ox,oy) came from src(oy, (sh-1)-ox) with sh = 260
        for (ox, oy) in [(0u32, 0u32), (259, 399), (100, 50), (5, 300)] {
            let p = out.pixel(ox, oy);
            assert_eq!((p[0], p[1]), (oy as f32, (259 - ox) as f32), "at ({ox},{oy})");
        }
    }

    #[test]
    fn rotate_180_reverses_both_axes() {
        let img = coord_img(300, 200);
        let out = rotate(&img, Size::new(300, 200), 180.0, true, &hash_by_index);
        assert_eq!(out.size(), Size::new(300, 200));
        let p = out.pixel(10, 20);
        assert_eq!((p[0], p[1]), ((299 - 10) as f32, (199 - 20) as f32));
    }

    #[test]
    fn rotate_45_expands_with_transparent_corners() {
        let img = solid(200, 200, [0.5, 0.2, 0.1, 1.0]);
        let out = rotate(&img, Size::new(283, 283), 45.0, true, &hash_by_index);
        assert_eq!(out.size(), Size::new(283, 283));
        // center still inside the rotated square → opaque
        assert!(out.pixel(141, 141)[3] > 0.99, "center opaque");
        // the new canvas corner is outside the rotated square → transparent
        assert_eq!(out.pixel(1, 1)[3], 0.0, "corner transparent");
    }

    #[test]
    fn rotate_no_expand_keeps_canvas() {
        let img = solid(120, 90, [0.3, 0.3, 0.3, 1.0]);
        let out = rotate(&img, Size::new(120, 90), 30.0, false, &hash_by_index);
        assert_eq!(out.size(), Size::new(120, 90), "no-expand keeps size");
    }

    #[test]
    fn flip_mirrors_each_axis() {
        let img = coord_img(300, 260); // spans tiles both ways
        let h = flip(&img, FlipAxis::Horizontal, &hash_by_index);
        let hp = h.pixel(10, 20);
        assert_eq!((hp[0], hp[1]), ((299 - 10) as f32, 20.0), "horizontal mirrors x");
        let v = flip(&img, FlipAxis::Vertical, &hash_by_index);
        let vp = v.pixel(10, 20);
        assert_eq!((vp[0], vp[1]), (10.0, (259 - 20) as f32), "vertical mirrors y");
    }

    #[test]
    fn pad_frames_source_with_fill() {
        let img = solid(100, 80, [1.0, 0.0, 0.0, 1.0]); // opaque red
        // 10 left, 20 right, 5 top, 15 bottom, transparent border
        let out = pad(&img, Size::new(130, 100), 10, 5, Rgba8::new(0, 0, 0, 0), &hash_by_index);
        assert_eq!(out.size(), Size::new(130, 100));
        // border is transparent
        assert_eq!(out.pixel(0, 0)[3], 0.0, "top-left border transparent");
        assert_eq!(out.pixel(129, 99)[3], 0.0, "bottom-right border transparent");
        // source placed at (10, 5): its top-left and interior are red
        assert_eq!(out.pixel(10, 5), [1.0, 0.0, 0.0, 1.0], "source origin");
        assert_eq!(out.pixel(109, 84), [1.0, 0.0, 0.0, 1.0], "source far corner");
        // just outside the source on the right is border again
        assert_eq!(out.pixel(110, 40)[3], 0.0, "past source right edge");
    }

    #[test]
    fn trim_crops_to_content() {
        // white 20×16 with a black content rect at (3,3)..(17,13)
        let (w, h) = (20u32, 16u32);
        let mut flat = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                let inside = (3..17).contains(&x) && (3..13).contains(&y);
                let c = if inside { [0.0, 0.0, 0.0, 1.0] } else { [1.0, 1.0, 1.0, 1.0] };
                flat[o..o + 4].copy_from_slice(&c);
            }
        }
        let img = TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index);
        // reference defaults to the (white) top-left corner
        let rect = content_bounds(&img, None, 0.0).unwrap();
        assert_eq!(rect, Rect::new(3, 3, 14, 10));
        let out = trim(&img, None, 0.0).unwrap();
        assert_eq!(out.size(), Size::new(14, 10));
        assert_eq!(out.pixel(0, 0), [0.0, 0.0, 0.0, 1.0], "trimmed to the content");
    }

    #[test]
    fn trim_none_for_uniform_and_unchanged_without_border() {
        let uniform = solid(64, 64, [0.2, 0.4, 0.6, 1.0]);
        assert!(content_bounds(&uniform, None, 0.0).is_none(), "all background");
        assert!(trim(&uniform, None, 0.0).is_none());

        // content touches every edge → nothing to trim → same size back
        let full = coord_img(50, 40); // pixel(0,0) is [0,0,0,1]; interior differs
        let out = trim(&full, None, 0.0).unwrap();
        assert_eq!(out.size(), Size::new(50, 40));
    }
}
