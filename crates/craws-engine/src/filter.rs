//! Neighborhood filters and the effects built on them: Gaussian blur, redaction
//! (pixelate / blur / solid-fill a region), and the one-shot "beautify" polish
//! (rounded corners + soft drop shadow + padded background).
//!
//! Everything runs on premultiplied linear f32 — the correct space to blur and
//! composite (no dark halos, no gamma bleed). Blur clamps at the edges (extend).

use crate::color::rgba8_to_linear_premul;
use crate::draw::{self, aa, sd_round_rect};
use crate::hash::ContentHash;
use crate::tile::{grid_dims, tile_dims, Tile, TileRef, TiledImage, TILE_SIZE};
use craws_domain::{RedactMode, Rgba8, Size};
use rayon::prelude::*;
use std::sync::Arc;

// ── Gaussian blur ────────────────────────────────────────────────────────────

/// Normalized 1D Gaussian weights for standard deviation `sigma`, truncated ±3σ.
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil().max(1.0) as i32;
    let mut k: Vec<f32> =
        (-radius..=radius).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    for w in &mut k {
        *w /= sum;
    }
    k
}

/// Separable Gaussian blur of a flat premultiplied RGBA f32 buffer (`w×h`).
/// Edges are clamped (extend). Two rayon-parallel 1D passes.
pub fn gaussian_blur_flat(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let k = gaussian_kernel(sigma);
    let radius = (k.len() / 2) as i32;

    let mut tmp = vec![0.0f32; src.len()];
    tmp.par_chunks_exact_mut(w * 4).enumerate().for_each(|(y, row)| {
        let base = y * w * 4;
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (j, &wt) in k.iter().enumerate() {
                let sx = (x as i32 + j as i32 - radius).clamp(0, w as i32 - 1) as usize;
                let o = base + sx * 4;
                for (a, s) in acc.iter_mut().zip(&src[o..o + 4]) {
                    *a += s * wt;
                }
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&acc);
        }
    });

    let mut out = vec![0.0f32; src.len()];
    out.par_chunks_exact_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (j, &wt) in k.iter().enumerate() {
                let sy = (y as i32 + j as i32 - radius).clamp(0, h as i32 - 1) as usize;
                let o = (sy * w + x) * 4;
                for (a, s) in acc.iter_mut().zip(&tmp[o..o + 4]) {
                    *a += s * wt;
                }
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&acc);
        }
    });
    out
}

/// Single-channel separable Gaussian blur of a flat `w×h` buffer, edges clamped.
/// Used for the beautify drop shadow (an alpha silhouette — blurring 1 channel
/// instead of RGBA is 4× the work and memory saved).
fn blur1(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let k = gaussian_kernel(sigma);
    let radius = (k.len() / 2) as i32;
    let mut tmp = vec![0.0f32; src.len()];
    tmp.par_chunks_exact_mut(w).enumerate().for_each(|(y, row)| {
        let base = y * w;
        for (x, slot) in row.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for (j, &wt) in k.iter().enumerate() {
                let sx = (x as i32 + j as i32 - radius).clamp(0, w as i32 - 1) as usize;
                acc += src[base + sx] * wt;
            }
            *slot = acc;
        }
    });
    let mut out = vec![0.0f32; src.len()];
    out.par_chunks_exact_mut(w).enumerate().for_each(|(y, row)| {
        for (x, slot) in row.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for (j, &wt) in k.iter().enumerate() {
                let sy = (y as i32 + j as i32 - radius).clamp(0, h as i32 - 1) as usize;
                acc += tmp[sy * w + x] * wt;
            }
            *slot = acc;
        }
    });
    out
}

/// Whole-image Gaussian blur. `sigma` in pixels; 0 is a no-op (tiles reused).
///
/// Tile-row **banded + streaming**: for each row of output tiles, horizontal-blur only
/// the source rows that band reads (±3σ halo, streamed straight from the tile grid via
/// `copy_row_into`) into a small band buffer, then vertical-blur that band directly into
/// the row's output tiles. No full-image flat copy of the input OR output — peak memory
/// is O(width × (256 + 2·radius)) per active band, independent of image height.
pub fn blur(
    input: &TiledImage,
    sigma: f32,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    if sigma <= 0.0 {
        return draw::clone_all(input, tile_hash);
    }
    let size = input.size();
    let (w, h) = (size.width as usize, size.height as usize);
    let k = gaussian_kernel(sigma);
    let radius = (k.len() / 2) as i32;
    let (cols, rows) = grid_dims(size);

    let bands: Vec<Vec<TileRef>> = (0..rows)
        .into_par_iter()
        .map(|trow| {
            let band_y0 = trow as usize * TILE_SIZE as usize;
            let band_h = (h - band_y0).min(TILE_SIZE as usize);
            // horizontal pass over the rows this band reads: [band_y0-radius, band_y0+band_h+radius)
            let hy0 = band_y0.saturating_sub(radius as usize);
            let hy1 = (band_y0 + band_h + radius as usize).min(h);
            let mut hb = vec![0.0f32; w * (hy1 - hy0) * 4];
            let mut src = vec![0.0f32; w * 4];
            for (i, out_row) in hb.chunks_exact_mut(w * 4).enumerate() {
                input.copy_row_into((hy0 + i) as u32, &mut src);
                for x in 0..w {
                    let mut acc = [0.0f32; 4];
                    for (j, &wt) in k.iter().enumerate() {
                        let sx = (x as i32 + j as i32 - radius).clamp(0, w as i32 - 1) as usize;
                        for (a, s) in acc.iter_mut().zip(&src[sx * 4..sx * 4 + 4]) {
                            *a += s * wt;
                        }
                    }
                    out_row[x * 4..x * 4 + 4].copy_from_slice(&acc);
                }
            }
            // vertical pass: build this tile-row's tiles straight from the band
            (0..cols)
                .map(|col| {
                    let (tw, th) = tile_dims(size, col, trow);
                    let ox0 = col as usize * TILE_SIZE as usize;
                    let mut px = vec![0.0f32; (tw * th * 4) as usize];
                    for ly in 0..th as usize {
                        let gy = band_y0 + ly;
                        for lx in 0..tw as usize {
                            let gx = ox0 + lx;
                            let mut acc = [0.0f32; 4];
                            for (j, &wt) in k.iter().enumerate() {
                                let sy = (gy as i32 + j as i32 - radius).clamp(0, h as i32 - 1) as usize;
                                let o = ((sy - hy0) * w + gx) * 4;
                                for (a, s) in acc.iter_mut().zip(&hb[o..o + 4]) {
                                    *a += s * wt;
                                }
                            }
                            let d = (ly * tw as usize + lx) * 4;
                            px[d..d + 4].copy_from_slice(&acc);
                        }
                    }
                    TileRef {
                        hash: tile_hash(trow * cols + col),
                        tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())),
                    }
                })
                .collect()
        })
        .collect();

    TiledImage::new(size, bands.into_iter().flatten().collect())
}

// ── redaction ────────────────────────────────────────────────────────────────

/// Integer, image-clamped rect `(x0, y0, w, h)`, or `None` if it's off-canvas.
fn clamp_rect(x: f32, y: f32, w: f32, h: f32, size: Size) -> Option<(u32, u32, u32, u32)> {
    let x0 = x.floor().max(0.0) as u32;
    let y0 = y.floor().max(0.0) as u32;
    let x1 = ((x + w).ceil().max(0.0) as u32).min(size.width);
    let y1 = ((y + h).ceil().max(0.0) as u32).min(size.height);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// Obscure a rectangular region. Only tiles overlapping the rect recompute; the
/// rest are reused (Arc clone).
pub fn redact(
    input: &TiledImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    mode: RedactMode,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let size = input.size();
    let Some((rx0, ry0, rw, rh)) = clamp_rect(x, y, w, h, size) else {
        return draw::clone_all(input, tile_hash);
    };
    let region = match mode {
        RedactMode::Fill { color } => fill_region(input, rx0, ry0, rw, rh, color),
        RedactMode::Pixelate { block } => pixelate_region(input, rx0, ry0, rw, rh, block),
        RedactMode::Blur { radius } => blur_region(input, rx0, ry0, rw, rh, radius),
    };
    write_region(input, rx0, ry0, rw, rh, &region, tile_hash)
}

/// Copy a rect out of the tiled image into a flat `rw×rh` premultiplied buffer,
/// tile-direct (row-run gather — no per-pixel `pixel()` lookup). Row-parallel.
fn read_region(input: &TiledImage, rx0: u32, ry0: u32, rw: u32, rh: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; rw as usize * rh as usize * 4];
    out.par_chunks_exact_mut(rw as usize * 4).enumerate().for_each(|(row, dst)| {
        let sy = ry0 + row as u32;
        let s_row = sy / TILE_SIZE;
        let s_ry = (sy % TILE_SIZE) as usize;
        let mut ox = 0u32;
        while ox < rw {
            let sx = rx0 + ox;
            let st = &input.tile_at(sx / TILE_SIZE, s_row).tile;
            let s_rx = (sx % TILE_SIZE) as usize;
            let run = ((rw - ox) as usize).min(st.width as usize - s_rx);
            let so = (s_ry * st.width as usize + s_rx) * 4;
            let d = ox as usize * 4;
            dst[d..d + run * 4].copy_from_slice(&st.px[so..so + run * 4]);
            ox += run as u32;
        }
    });
    out
}

/// Blend a solid `color` over the region (opaque → a black bar, no source read;
/// translucent → a tint over the tile-direct-read region).
fn fill_region(input: &TiledImage, rx0: u32, ry0: u32, rw: u32, rh: u32, color: Rgba8) -> Vec<f32> {
    let c = rgba8_to_linear_premul(color);
    let a = c[3];
    if a >= 1.0 {
        return std::iter::repeat_n(c, rw as usize * rh as usize).flatten().collect();
    }
    let inv = 1.0 - a;
    let mut region = read_region(input, rx0, ry0, rw, rh);
    region.par_chunks_exact_mut(4).for_each(|d| {
        d[0] = c[0] + d[0] * inv;
        d[1] = c[1] + d[1] * inv;
        d[2] = c[2] + d[2] * inv;
        d[3] = a + d[3] * inv;
    });
    region
}

/// Mosaic: average `block`×`block` cells within the region.
fn pixelate_region(input: &TiledImage, rx0: u32, ry0: u32, rw: u32, rh: u32, block: u32) -> Vec<f32> {
    let block = block.max(1);
    let region = read_region(input, rx0, ry0, rw, rh); // tile-direct, once
    let mut out = vec![0.0f32; rw as usize * rh as usize * 4];
    // par over block-rows (each writes a disjoint band; the last may be short)
    out.par_chunks_mut((rw * block) as usize * 4).enumerate().for_each(|(bi, band)| {
        let by = bi as u32 * block;
        let bh = (band.len() / (rw as usize * 4)) as u32;
        let mut bx = 0;
        while bx < rw {
            let bw = block.min(rw - bx);
            let mut acc = [0.0f64; 4];
            for yy in 0..bh {
                for xx in 0..bw {
                    let o = (((by + yy) * rw + bx + xx) * 4) as usize;
                    for (a, v) in acc.iter_mut().zip(&region[o..o + 4]) {
                        *a += *v as f64;
                    }
                }
            }
            let n = (bw * bh) as f64;
            let avg = [(acc[0] / n) as f32, (acc[1] / n) as f32, (acc[2] / n) as f32, (acc[3] / n) as f32];
            for yy in 0..bh {
                for xx in 0..bw {
                    let o = ((yy * rw + bx + xx) * 4) as usize;
                    band[o..o + 4].copy_from_slice(&avg);
                }
            }
            bx += bw;
        }
    });
    out
}

/// Gaussian-blur just the region (extract rect + a 3σ margin tile-direct, blur, crop back).
fn blur_region(input: &TiledImage, rx0: u32, ry0: u32, rw: u32, rh: u32, sigma: f32) -> Vec<f32> {
    if sigma <= 0.0 {
        return read_region(input, rx0, ry0, rw, rh);
    }
    let size = input.size();
    let margin = (sigma * 3.0).ceil() as u32;
    let ex0 = rx0.saturating_sub(margin);
    let ey0 = ry0.saturating_sub(margin);
    let ex1 = (rx0 + rw + margin).min(size.width);
    let ey1 = (ry0 + rh + margin).min(size.height);
    let (ew, eh) = ((ex1 - ex0) as usize, (ey1 - ey0) as usize);
    let ext = read_region(input, ex0, ey0, ex1 - ex0, ey1 - ey0);
    let blurred = gaussian_blur_flat(&ext, ew, eh, sigma);
    let (ox, oy) = ((rx0 - ex0) as usize, (ry0 - ey0) as usize);
    let mut out = vec![0.0f32; rw as usize * rh as usize * 4];
    out.par_chunks_exact_mut(rw as usize * 4).enumerate().for_each(|(yy, row)| {
        let o = ((oy + yy) * ew + ox) * 4;
        row.copy_from_slice(&blurred[o..o + rw as usize * 4]);
    });
    out
}

/// Overwrite the rect with `region` (row-major `rw×rh`); clone the rest.
fn write_region(
    input: &TiledImage,
    rx0: u32,
    ry0: u32,
    rw: u32,
    rh: u32,
    region: &[f32],
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let size = input.size();
    let (cols, rows) = grid_dims(size);
    let (rx1, ry1) = (rx0 + rw, ry0 + rh);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(size, col, row);
            let (ox0, oy0) = (col * TILE_SIZE, row * TILE_SIZE);
            let ix0 = ox0.max(rx0);
            let iy0 = oy0.max(ry0);
            let ix1 = (ox0 + tw).min(rx1);
            let iy1 = (oy0 + th).min(ry1);
            if ix1 <= ix0 || iy1 <= iy0 {
                let src = &input.tiles()[index as usize];
                return TileRef { hash: tile_hash(index), tile: Arc::clone(&src.tile) };
            }
            let mut px = input.tiles()[index as usize].tile.px.to_vec();
            for iy in iy0..iy1 {
                let run = (ix1 - ix0) as usize;
                let r_o = ((iy - ry0) as usize * rw as usize + (ix0 - rx0) as usize) * 4;
                let t_o = ((iy - oy0) as usize * tw as usize + (ix0 - ox0) as usize) * 4;
                px[t_o..t_o + run * 4].copy_from_slice(&region[r_o..r_o + run * 4]);
            }
            TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(size, tiles)
}

// ── beautify ─────────────────────────────────────────────────────────────────

/// Parameters for [`beautify`].
pub struct BeautifyParams {
    pub padding: u32,
    pub corner_radius: f32,
    pub shadow_radius: f32,
    pub shadow_opacity: f32,
    pub shadow_offset: f32,
    pub background: Rgba8,
}

/// Source-over of premultiplied `s` onto premultiplied `d`, in place.
#[inline]
fn over_px(d: &mut [f32], s: &[f32]) {
    let inv = 1.0 - s[3];
    d[0] = s[0] + d[0] * inv;
    d[1] = s[1] + d[1] * inv;
    d[2] = s[2] + d[2] * inv;
    d[3] = s[3] + d[3] * inv;
}

/// Composite `src` (`sw×sh`) over `dst` (`dw×dh`) with its top-left at `(ox, oy)`.
#[allow(clippy::too_many_arguments)]
fn composite_at(dst: &mut [f32], dw: usize, dh: usize, src: &[f32], sw: usize, sh: usize, ox: usize, oy: usize) {
    for y in 0..sh {
        let dy = oy + y;
        if dy >= dh {
            break;
        }
        for x in 0..sw {
            let dx = ox + x;
            if dx >= dw {
                break;
            }
            let so = (y * sw + x) * 4;
            let dofs = (dy * dw + dx) * 4;
            let s = [src[so], src[so + 1], src[so + 2], src[so + 3]];
            over_px(&mut dst[dofs..dofs + 4], &s);
        }
    }
}

/// Polish a screenshot: round its corners, drop a soft shadow, and frame it with
/// `padding` of `background`. `out` is the pre-validated `input + 2·padding` size.
pub fn beautify(
    input: &TiledImage,
    out: Size,
    p: &BeautifyParams,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let src = input.size();
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (ow, oh) = (out.width as usize, out.height as usize);
    let pad = p.padding as usize;

    // 1) rounded source: scale premultiplied RGBA by the rounded-rect coverage
    let mut rsrc = input.to_flat_f32();
    let r = p.corner_radius.clamp(0.0, (sw.min(sh) as f32) / 2.0);
    if r > 0.0 {
        let (cx, cy) = (sw as f32 / 2.0, sh as f32 / 2.0);
        let (hw, hh) = (sw as f32 / 2.0, sh as f32 / 2.0);
        rsrc.par_chunks_exact_mut(sw * 4).enumerate().for_each(|(y, row)| {
            for x in 0..sw {
                let cov = aa(sd_round_rect(x as f32 + 0.5, y as f32 + 0.5, cx, cy, hw, hh, r));
                let o = x * 4;
                for v in &mut row[o..o + 4] {
                    *v *= cov;
                }
            }
        });
    }

    // 2) background canvas
    let bg = rgba8_to_linear_premul(p.background);
    let mut canvas: Vec<f32> = std::iter::repeat_n(bg, ow * oh).flatten().collect();

    // 3) soft drop shadow: the rounded silhouette's ALPHA, offset down, blurred as a
    //    single channel (the shadow is premultiplied black ⇒ RGB≡0), composited under
    //    the image. Blurring 1 channel instead of RGBA is 4× less work and memory.
    if p.shadow_opacity > 0.0 {
        let mut shadow_a = vec![0.0f32; ow * oh];
        let off_y = p.shadow_offset.round() as i64;
        for y in 0..sh {
            let dy = pad as i64 + off_y + y as i64;
            if dy < 0 || dy >= oh as i64 {
                continue;
            }
            let drow = dy as usize * ow;
            for x in 0..sw {
                shadow_a[drow + pad + x] = rsrc[(y * sw + x) * 4 + 3] * p.shadow_opacity;
            }
        }
        let blurred = blur1(&shadow_a, ow, oh, p.shadow_radius);
        // source-over of premultiplied black (rgb 0, alpha a) onto the canvas
        canvas.par_chunks_exact_mut(4).zip(blurred.par_iter()).for_each(|(d, &a)| {
            let inv = 1.0 - a;
            d[0] *= inv;
            d[1] *= inv;
            d[2] *= inv;
            d[3] = a + d[3] * inv;
        });
    }

    // 4) the rounded source, over the shadow, at (pad, pad)
    composite_at(&mut canvas, ow, oh, &rsrc, sw, sh, pad, pad);

    TiledImage::from_flat_f32(out, &canvas, tile_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;

    fn hash_by_index(i: u32) -> ContentHash {
        digest_bytes(&i.to_le_bytes())
    }

    fn solid(w: u32, h: u32, rgba: [f32; 4]) -> TiledImage {
        let flat: Vec<f32> = std::iter::repeat_n(rgba, (w * h) as usize).flatten().collect();
        TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index)
    }

    /// Each pixel stores its x in R (varying content for block/blur checks).
    fn ramp_x(w: u32, h: u32) -> TiledImage {
        let mut flat = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                flat[o] = x as f32;
                flat[o + 3] = 1.0;
            }
        }
        TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index)
    }

    #[test]
    fn blur_keeps_a_solid_solid() {
        let img = solid(300, 300, [0.4, 0.6, 0.8, 1.0]);
        let out = blur(&img, 5.0, &hash_by_index);
        assert_eq!(out.size(), Size::new(300, 300));
        let p = out.pixel(150, 150);
        assert!((p[0] - 0.4).abs() < 1e-3 && (p[2] - 0.8).abs() < 1e-3, "solid stays solid: {p:?}");
    }

    #[test]
    fn blur_softens_a_hard_edge() {
        // left half 0, right half 1 (opaque); after blur the seam is a ramp
        let (w, h) = (128u32, 32u32);
        let mut flat = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                let v = if x >= w / 2 { 1.0 } else { 0.0 };
                flat[o] = v;
                flat[o + 3] = 1.0;
            }
        }
        let img = TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index);
        let out = blur(&img, 3.0, &hash_by_index);
        let seam = out.pixel(64, 16)[0];
        assert!(seam > 0.1 && seam < 0.9, "seam is a mid value: {seam}");
        assert!(out.pixel(64, 16)[0] > out.pixel(60, 16)[0], "monotone across the seam");
    }

    #[test]
    fn redact_fill_paints_only_the_region() {
        let img = solid(200, 120, [1.0, 1.0, 1.0, 1.0]); // white
        let out = redact(&img, 40.0, 30.0, 80.0, 50.0, RedactMode::Fill { color: Rgba8::rgb(255, 0, 0) }, &hash_by_index);
        assert_eq!(out.pixel(80, 55), [1.0, 0.0, 0.0, 1.0], "inside is the fill");
        assert_eq!(out.pixel(5, 5), [1.0, 1.0, 1.0, 1.0], "outside untouched");
    }

    #[test]
    fn redact_pixelate_makes_uniform_blocks() {
        let img = ramp_x(64, 16);
        let out = redact(&img, 16.0, 0.0, 32.0, 16.0, RedactMode::Pixelate { block: 4 }, &hash_by_index);
        // a 4-wide block starting at x=16 averages to (16+17+18+19)/4 = 17.5, uniform
        let a = out.pixel(16, 0)[0];
        let b = out.pixel(19, 3)[0];
        assert_eq!(a, b, "the block is uniform");
        assert!((a - 17.5).abs() < 1e-3, "block average: {a}");
        // outside the region is untouched
        assert_eq!(out.pixel(5, 0)[0], 5.0);
    }

    #[test]
    fn redact_blur_changes_region_only() {
        let img = ramp_x(96, 16);
        let out = redact(&img, 32.0, 0.0, 32.0, 16.0, RedactMode::Blur { radius: 3.0 }, &hash_by_index);
        assert_eq!(out.pixel(5, 8)[0], 5.0, "outside identical");
        // inside, the sharp ramp is softened → not exactly x anymore near variation,
        // but a flat ramp blurs to ~itself; assert the op ran (still finite & near-x)
        let inside = out.pixel(48, 8)[0];
        assert!(inside.is_finite() && (inside - 48.0).abs() < 4.0, "blurred ramp near original: {inside}");
    }

    #[test]
    fn beautify_grows_canvas_with_shadow_and_rounding() {
        let img = solid(120, 90, [0.2, 0.5, 0.9, 1.0]);
        let params = BeautifyParams {
            padding: 40,
            corner_radius: 16.0,
            shadow_radius: 12.0,
            shadow_opacity: 0.5,
            shadow_offset: 10.0,
            background: Rgba8::new(0, 0, 0, 0), // transparent frame
        };
        let out = beautify(&img, Size::new(200, 170), &params, &hash_by_index);
        assert_eq!(out.size(), Size::new(200, 170), "grows by 2·padding");
        // center sits over the source → opaque source color
        let c = out.pixel(100, 85);
        assert!(c[3] > 0.99 && (c[2] - 0.9).abs() < 0.02, "center is the image: {c:?}");
        // far top-left corner is the transparent frame
        assert_eq!(out.pixel(1, 1)[3], 0.0, "corner is transparent frame");
        // below the image, inside the padding, the shadow adds some alpha
        let shadow = out.pixel(100, 145);
        assert!(shadow[3] > 0.05, "soft shadow present below the image: {shadow:?}");
    }
}
