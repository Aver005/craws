//! Operation kernels. Pure functions over tiles/images — no cache, no I/O.
//!
//! Two shapes exist in M0:
//! - **pointwise** (`exposure`, `grayscale`): tile → tile, embarrassingly parallel,
//!   cached per-tile by the engine;
//! - **global** (`crop`, `resize`): whole image → whole image, cached per output
//!   tile under one signature. (Crop is actually neighborhood-local — tightening
//!   its hashing to only the overlapped source tiles is a known improvement.)

use crate::hash::ContentHash;
use crate::tile::{grid_dims, tile_dims, Tile, TileRef, TiledImage, TILE_SIZE};
use craws_domain::{Filter, Rect, Size};
use image::imageops::FilterType;
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
/// M0 goes through a flat buffer (`image::imageops`); a streaming tile-band
/// resampler is a planned BLAZING upgrade for huge inputs.
pub fn resize(
    input: &TiledImage,
    target: Size,
    filter: Filter,
    tile_hash: &(dyn Fn(u32) -> ContentHash + Sync),
) -> TiledImage {
    let src = input.size();
    let buf: image::ImageBuffer<image::Rgba<f32>, Vec<f32>> =
        image::ImageBuffer::from_raw(src.width, src.height, input.to_flat_f32())
            .expect("flat buffer matches dimensions");
    let ft = match filter {
        Filter::Nearest => FilterType::Nearest,
        Filter::Bilinear => FilterType::Triangle,
        Filter::CatmullRom => FilterType::CatmullRom,
        Filter::Lanczos3 => FilterType::Lanczos3,
    };
    let out = image::imageops::resize(&buf, target.width, target.height, ft);
    TiledImage::from_flat_f32(target, out.as_raw(), tile_hash)
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
}
