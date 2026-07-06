//! Tiled image storage: linear-light premultiplied RGBA f32 in a 256² grid.
//!
//! Edge tiles are stored exact-size (no padding). Tiles are immutable behind
//! `Arc` — pipeline steps produce new tiles and structurally share the rest,
//! which is what makes undo, caching and "only recompute what changed" cheap.

use crate::color::{linear_to_srgb8, srgb8_to_linear};
use crate::hash::{self, ContentHash};
use craws_domain::Size;
use rayon::prelude::*;
use std::sync::Arc;

pub const TILE_SIZE: u32 = 256;

/// One tile: `width * height * 4` f32, row-major RGBA, premultiplied, linear.
#[derive(Debug, PartialEq)]
pub struct Tile {
    pub width: u32,
    pub height: u32,
    pub px: Box<[f32]>,
}

impl Tile {
    pub fn new(width: u32, height: u32, px: Box<[f32]>) -> Self {
        debug_assert_eq!(px.len(), (width * height * 4) as usize);
        Self { width, height, px }
    }

    pub fn solid(width: u32, height: u32, rgba: [f32; 4]) -> Self {
        let mut px = vec![0.0f32; (width * height * 4) as usize];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&rgba);
        }
        Self::new(width, height, px.into_boxed_slice())
    }

    /// Heap footprint, for cache accounting.
    pub fn bytes(&self) -> u64 {
        self.px.len() as u64 * size_of::<f32>() as u64
    }
}

/// A tile plus its content identity.
#[derive(Debug, Clone)]
pub struct TileRef {
    pub hash: ContentHash,
    pub tile: Arc<Tile>,
}

/// Immutable tiled image. Cloning is cheap (Arc per tile).
#[derive(Debug, Clone)]
pub struct TiledImage {
    size: Size,
    cols: u32,
    rows: u32,
    /// row-major tile order: index = row * cols + col
    tiles: Vec<TileRef>,
}

/// Number of tile columns/rows covering `size`.
pub fn grid_dims(size: Size) -> (u32, u32) {
    (size.width.div_ceil(TILE_SIZE), size.height.div_ceil(TILE_SIZE))
}

/// Pixel dimensions of the tile at grid position (col, row).
pub fn tile_dims(size: Size, col: u32, row: u32) -> (u32, u32) {
    let w = (size.width - col * TILE_SIZE).min(TILE_SIZE);
    let h = (size.height - row * TILE_SIZE).min(TILE_SIZE);
    (w, h)
}

impl TiledImage {
    pub fn new(size: Size, tiles: Vec<TileRef>) -> Self {
        let (cols, rows) = grid_dims(size);
        debug_assert_eq!(tiles.len(), (cols * rows) as usize);
        Self { size, cols, rows, tiles }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn grid(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    pub fn tiles(&self) -> &[TileRef] {
        &self.tiles
    }

    pub fn tile_at(&self, col: u32, row: u32) -> &TileRef {
        &self.tiles[(row * self.cols + col) as usize]
    }

    /// Decode sRGB RGBA8 into linear premultiplied tiles.
    /// `source` is the digest of the *original bytes* (file, stream…); tile
    /// identities derive from it, so pixel data itself is never hashed.
    pub fn from_srgb_rgba8(size: Size, rgba: &[u8], source: ContentHash) -> Self {
        assert_eq!(rgba.len() as u64, size.area() * 4, "buffer size mismatch");
        let (cols, rows) = grid_dims(size);
        let stride = size.width as usize * 4;

        let tiles: Vec<TileRef> = (0..cols * rows)
            .into_par_iter()
            .map(|index| {
                let (col, row) = (index % cols, index / cols);
                let (tw, th) = tile_dims(size, col, row);
                let (x0, y0) = ((col * TILE_SIZE) as usize, (row * TILE_SIZE) as usize);
                let mut px = vec![0.0f32; (tw * th * 4) as usize];
                for ty in 0..th as usize {
                    let src = &rgba[(y0 + ty) * stride + x0 * 4..][..tw as usize * 4];
                    let dst = &mut px[ty * tw as usize * 4..][..tw as usize * 4];
                    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                        let a = s[3] as f32 / 255.0;
                        d[0] = srgb8_to_linear(s[0]) * a;
                        d[1] = srgb8_to_linear(s[1]) * a;
                        d[2] = srgb8_to_linear(s[2]) * a;
                        d[3] = a;
                    }
                }
                TileRef {
                    hash: hash::source_tile_hash(source, index),
                    tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())),
                }
            })
            .collect();

        Self::new(size, tiles)
    }

    /// Encode back to sRGB RGBA8 (unpremultiply, clamp, transfer). Row-parallel.
    pub fn to_srgb_rgba8(&self) -> Vec<u8> {
        let w = self.size.width as usize;
        let mut out = vec![0u8; self.size.area() as usize * 4];
        out.par_chunks_exact_mut(w * 4).enumerate().for_each(|(y, dst_row)| {
            let trow = y as u32 / TILE_SIZE;
            let ty = y % TILE_SIZE as usize;
            for col in 0..self.cols {
                let t = &self.tile_at(col, trow).tile;
                let tw = t.width as usize;
                let src = &t.px[ty * tw * 4..][..tw * 4];
                let dst = &mut dst_row[(col * TILE_SIZE) as usize * 4..][..tw * 4];
                for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                    let a = s[3];
                    let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
                    d[0] = linear_to_srgb8(s[0] * inv);
                    d[1] = linear_to_srgb8(s[1] * inv);
                    d[2] = linear_to_srgb8(s[2] * inv);
                    d[3] = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        });
        out
    }

    /// Flatten to one row-major f32 buffer (for whole-image resampling).
    pub fn to_flat_f32(&self) -> Vec<f32> {
        let w = self.size.width as usize;
        let mut out = vec![0.0f32; self.size.area() as usize * 4];
        out.par_chunks_exact_mut(w * 4).enumerate().for_each(|(y, dst_row)| {
            let trow = y as u32 / TILE_SIZE;
            let ty = y % TILE_SIZE as usize;
            for col in 0..self.cols {
                let t = &self.tile_at(col, trow).tile;
                let tw = t.width as usize;
                dst_row[(col * TILE_SIZE) as usize * 4..][..tw * 4]
                    .copy_from_slice(&t.px[ty * tw * 4..][..tw * 4]);
            }
        });
        out
    }

    /// Re-tile a flat row-major f32 buffer. Tile identities come from `tile_hash`
    /// (the producing op knows how its outputs are addressed).
    pub fn from_flat_f32(
        size: Size,
        flat: &[f32],
        tile_hash: impl Fn(u32) -> ContentHash + Sync,
    ) -> Self {
        assert_eq!(flat.len() as u64, size.area() * 4, "buffer size mismatch");
        let (cols, rows) = grid_dims(size);
        let stride = size.width as usize * 4;
        let tiles: Vec<TileRef> = (0..cols * rows)
            .into_par_iter()
            .map(|index| {
                let (col, row) = (index % cols, index / cols);
                let (tw, th) = tile_dims(size, col, row);
                let (x0, y0) = ((col * TILE_SIZE) as usize, (row * TILE_SIZE) as usize);
                let mut px = vec![0.0f32; (tw * th * 4) as usize];
                for ty in 0..th as usize {
                    px[ty * tw as usize * 4..][..tw as usize * 4].copy_from_slice(
                        &flat[(y0 + ty) * stride + x0 * 4..][..tw as usize * 4],
                    );
                }
                TileRef { hash: tile_hash(index), tile: Arc::new(Tile::new(tw, th, px.into_boxed_slice())) }
            })
            .collect();
        Self::new(size, tiles)
    }

    /// Read one pixel (linear premultiplied). Debug/test/picker helper — not a hot path.
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        assert!(x < self.size.width && y < self.size.height);
        let t = &self.tile_at(x / TILE_SIZE, y / TILE_SIZE).tile;
        let (rx, ry) = ((x % TILE_SIZE) as usize, (y % TILE_SIZE) as usize);
        let o = (ry * t.width as usize + rx) * 4;
        [t.px[o], t.px[o + 1], t.px[o + 2], t.px[o + 3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;

    #[test]
    fn grid_math_covers_edges() {
        assert_eq!(grid_dims(Size::new(256, 256)), (1, 1));
        assert_eq!(grid_dims(Size::new(257, 256)), (2, 1));
        assert_eq!(grid_dims(Size::new(1, 1)), (1, 1));
        assert_eq!(tile_dims(Size::new(300, 520), 1, 2), (300 - 256, 520 - 512));
    }

    #[test]
    fn rgba8_roundtrip_opaque_is_exact() {
        // 300x300 crosses a tile boundary; deterministic pseudo-random pixels
        let size = Size::new(300, 300);
        let mut rgba = vec![0u8; size.area() as usize * 4];
        let mut state = 0x12345678u32;
        for (i, b) in rgba.iter_mut().enumerate() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = if i % 4 == 3 { 255 } else { (state >> 24) as u8 };
        }
        let img = TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(b"t"));
        assert_eq!(img.to_srgb_rgba8(), rgba);
    }

    #[test]
    fn rgba8_roundtrip_translucent_within_1() {
        let size = Size::new(64, 64);
        let mut rgba = vec![0u8; size.area() as usize * 4];
        for (i, b) in rgba.iter_mut().enumerate() {
            *b = match i % 4 {
                0 => 200,
                1 => 100,
                2 => 50,
                _ => 128, // translucent
            };
        }
        let img = TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(b"t"));
        for (got, want) in img.to_srgb_rgba8().iter().zip(&rgba) {
            assert!((*got as i16 - *want as i16).abs() <= 1, "{got} vs {want}");
        }
    }

    #[test]
    fn zero_alpha_does_not_nan() {
        let size = Size::new(2, 1);
        let rgba = [10, 20, 30, 0, 40, 50, 60, 0];
        let img = TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(b"t"));
        let out = img.to_srgb_rgba8();
        assert_eq!(&out[..4], &[0, 0, 0, 0], "fully transparent encodes as zeros");
    }

    #[test]
    fn flat_roundtrip_and_pixel_access() {
        let size = Size::new(520, 300);
        let flat: Vec<f32> = (0..size.area() as usize * 4).map(|i| (i % 977) as f32 / 976.0).collect();
        let img = TiledImage::from_flat_f32(size, &flat, |i| digest_bytes(&i.to_le_bytes()));
        assert_eq!(img.to_flat_f32(), flat);
        // pixel() agrees with the flat layout across tile boundaries
        let (x, y) = (511, 299);
        let o = (y as usize * size.width as usize + x as usize) * 4;
        assert_eq!(img.pixel(x, y), [flat[o], flat[o + 1], flat[o + 2], flat[o + 3]]);
    }

    #[test]
    fn source_tiles_have_distinct_stable_hashes() {
        let size = Size::new(300, 300);
        let rgba = vec![255u8; size.area() as usize * 4];
        let a = TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(b"src"));
        let b = TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(b"src"));
        assert_eq!(a.tiles()[0].hash, b.tiles()[0].hash, "same source ⇒ same identity");
        assert_ne!(a.tiles()[0].hash, a.tiles()[1].hash, "different tiles differ");
        let c = TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(b"other"));
        assert_ne!(a.tiles()[0].hash, c.tiles()[0].hash, "different source ⇒ different identity");
    }
}
