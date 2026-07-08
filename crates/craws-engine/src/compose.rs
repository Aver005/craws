//! Multi-image composition: overlay one image onto another, and lay several
//! images out into a "smart" collage. These take more than one input, so they
//! live outside the single-input [`crate::engine::Engine::run`] pipeline — the
//! MCP session calls them directly.

use crate::color::rgba8_to_linear_premul;
use crate::hash::{self, ContentHash};
use crate::ops;
use crate::tile::{grid_dims, tile_dims, Tile, TileRef, TiledImage, TILE_SIZE};
use craws_domain::{Filter, Rgba8, Size};
use rayon::prelude::*;
use std::sync::Arc;

fn hash_by_index(i: u32) -> ContentHash {
    hash::digest_bytes(&i.to_le_bytes())
}

/// Composite `top` onto `base` at integer offset `(x, y)` with `opacity` in
/// [0,1]. Output size = base size. Source-over in linear premultiplied light.
/// Output tiles are **content-addressed** (folds both inputs' hashes + params via
/// [`stamp`]) so two different overlays never collide in a downstream cache.
pub fn overlay(base: &TiledImage, top: &TiledImage, x: i32, y: i32, opacity: f32) -> TiledImage {
    let opacity = opacity.clamp(0.0, 1.0);
    let mut params = Vec::with_capacity(16);
    params.extend_from_slice(b"overlay");
    params.extend_from_slice(&x.to_le_bytes());
    params.extend_from_slice(&y.to_le_bytes());
    params.extend_from_slice(&opacity.to_le_bytes());
    let sig = hash::compose_signature(
        &params,
        base.tiles().iter().chain(top.tiles().iter()).map(|t| &t.hash),
    );
    stamp(blend(base, top, x, y, opacity), sig)
}

/// Re-stamp assembled tiles with content-true identities from `sig` (Arcs/pixels
/// kept). Compose ops build with throwaway index hashes, then stamp so distinct
/// inputs/params never share tile identities downstream.
fn stamp(img: TiledImage, sig: ContentHash) -> TiledImage {
    let size = img.size();
    let tiles = img
        .tiles()
        .iter()
        .enumerate()
        .map(|(i, t)| TileRef { hash: hash::global_tile_hash(sig, i as u32), tile: Arc::clone(&t.tile) })
        .collect();
    TiledImage::new(size, tiles)
}

/// Source-over blend of `top` onto `base` — pixels only, index-hashed; public
/// callers wrap this with [`stamp`] for content-true identities.
fn blend(base: &TiledImage, top: &TiledImage, x: i32, y: i32, opacity: f32) -> TiledImage {
    let size = base.size();
    let ts = top.size();
    let opacity = opacity.clamp(0.0, 1.0);
    // placement rect of `top` in base space, clamped to the canvas
    let px0 = x.max(0);
    let py0 = y.max(0);
    let px1 = (x + ts.width as i32).min(size.width as i32);
    let py1 = (y + ts.height as i32).min(size.height as i32);

    // `top` read once into a contiguous buffer (tile-direct) so the per-pixel blend
    // indexes flat memory instead of paying a tile lookup (`pixel()`) per sample.
    let top_flat = top.to_flat_f32();
    let tw_top = ts.width as usize;

    let (cols, rows) = grid_dims(size);
    let tiles: Vec<TileRef> = (0..cols * rows)
        .into_par_iter()
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let (tw, th) = tile_dims(size, col, row);
            let (ox, oy) = ((col * TILE_SIZE) as i32, (row * TILE_SIZE) as i32);
            // tile ∩ placement rect
            let lx0 = (px0 - ox).max(0);
            let ly0 = (py0 - oy).max(0);
            let lx1 = (px1 - ox).min(tw as i32);
            let ly1 = (py1 - oy).min(th as i32);
            if opacity <= 0.0 || lx1 <= lx0 || ly1 <= ly0 {
                let src = &base.tiles()[index as usize];
                return TileRef { hash: hash_by_index(index), tile: Arc::clone(&src.tile) };
            }
            let mut buf = base.tiles()[index as usize].tile.px.to_vec();
            for ly in ly0..ly1 {
                let gy = oy + ly;
                let ty = (gy - y) as usize;
                for lx in lx0..lx1 {
                    let gx = ox + lx;
                    let tx = (gx - x) as usize;
                    let ti = (ty * tw_top + tx) * 4;
                    let s = &top_flat[ti..ti + 4]; // linear premultiplied
                    let a_eff = s[3] * opacity;
                    if a_eff <= 0.0 {
                        continue;
                    }
                    let inv = 1.0 - a_eff;
                    let off = (ly as usize * tw as usize + lx as usize) * 4;
                    let d = &mut buf[off..off + 4];
                    d[0] = s[0] * opacity + d[0] * inv;
                    d[1] = s[1] * opacity + d[1] * inv;
                    d[2] = s[2] * opacity + d[2] * inv;
                    d[3] = a_eff + d[3] * inv;
                }
            }
            TileRef { hash: hash_by_index(index), tile: Arc::new(Tile::new(tw, th, buf.into_boxed_slice())) }
        })
        .collect();
    TiledImage::new(size, tiles)
}

/// How a collage is arranged.
#[derive(Debug, Clone, Copy)]
pub struct CollageOptions {
    pub target_width: u32,
    /// Nominal row height before justification.
    pub row_height: u32,
    /// Gap between cells and around the edges, in pixels.
    pub gap: u32,
    pub background: Rgba8,
}

/// Justified-rows ("smart") layout: images keep their aspect ratio, are packed
/// into rows, and each full row is scaled to fill the target width exactly — the
/// classic Flickr/Google-Photos layout, so mixed resolutions tile cleanly.
pub fn collage(images: &[&TiledImage], opts: CollageOptions) -> TiledImage {
    assert!(!images.is_empty(), "collage needs at least one image");
    let sizes: Vec<Size> = images.iter().map(|i| i.size()).collect();
    let (canvas, rects) = justified_layout(&sizes, opts);

    // solid background canvas (linear premultiplied)
    let bg = rgba8_to_linear_premul(opts.background);
    let flat: Vec<f32> = std::iter::repeat_n(bg, canvas.area() as usize).flatten().collect();
    let mut out = TiledImage::from_flat_f32(canvas, &flat, hash_by_index);

    // resize each image to its cell and blend it (index-hashed; stamped below)
    for (img, r) in images.iter().zip(&rects) {
        let cell = Size::new(r.width.max(1), r.height.max(1));
        let scaled = ops::resize(img, cell, Filter::Lanczos3, &hash_by_index);
        out = blend(&out, &scaled, r.x as i32, r.y as i32, 1.0);
    }
    // stamp with a content-true signature from the ORIGINAL images + options
    let mut params = Vec::with_capacity(19);
    params.extend_from_slice(b"collage");
    params.extend_from_slice(&opts.target_width.to_le_bytes());
    params.extend_from_slice(&opts.row_height.to_le_bytes());
    params.extend_from_slice(&opts.gap.to_le_bytes());
    params.extend_from_slice(&[opts.background.r, opts.background.g, opts.background.b, opts.background.a]);
    let sig = hash::compose_signature(&params, images.iter().flat_map(|i| i.tiles().iter().map(|t| &t.hash)));
    stamp(out, sig)
}

/// Pure geometry: place `sizes` into justified rows. Returns the canvas size and
/// one rect per input (same order).
fn justified_layout(sizes: &[Size], opts: CollageOptions) -> (Size, Vec<craws_domain::Rect>) {
    use craws_domain::Rect;
    let gap = opts.gap as f32;
    let avail = (opts.target_width as f32 - 2.0 * gap).max(1.0);
    let rh0 = opts.row_height.max(1) as f32;

    let mut rects = vec![Rect::new(0, 0, 1, 1); sizes.len()];
    let mut y = gap;
    let mut i = 0;
    while i < sizes.len() {
        // greedily collect a row of indices
        let mut row: Vec<usize> = Vec::new();
        let mut sum_w = 0.0f32; // sum of widths at rh0
        let mut j = i;
        while j < sizes.len() {
            let s = sizes[j];
            let w = rh0 * s.width as f32 / s.height.max(1) as f32;
            row.push(j);
            sum_w += w;
            j += 1;
            let gaps = (row.len() as f32 - 1.0) * gap;
            if sum_w + gaps >= avail {
                break;
            }
        }
        let gaps = (row.len() as f32 - 1.0) * gap;
        let is_last = j >= sizes.len();
        // justify full rows to `avail`; leave an underfilled last row at rh0
        let scale = if is_last && sum_w + gaps < avail { 1.0 } else { (avail - gaps).max(1.0) / sum_w };
        let rh = rh0 * scale;

        let mut x = gap;
        for &idx in &row {
            let s = sizes[idx];
            let w = rh0 * s.width as f32 / s.height.max(1) as f32 * scale;
            rects[idx] = Rect::new(
                x.round() as u32,
                y.round() as u32,
                (w.round() as u32).max(1),
                (rh.round() as u32).max(1),
            );
            x += w + gap;
        }
        y += rh + gap;
        i = j;
    }
    let canvas = Size::new(opts.target_width.max(1), (y.round() as u32).max(1));
    (canvas, rects)
}

// ── diff / compare ───────────────────────────────────────────────────────────

/// How [`diff`] visualizes two images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffView {
    /// Per-channel absolute difference (dark where identical).
    Difference,
    /// `a` dimmed to gray, differing pixels glowing red by magnitude.
    Heatmap,
    /// `a` and `b` laid side by side with a small gap (any sizes).
    SideBySide,
}

/// Numeric comparison returned alongside a [`diff`] visualization. `None` when
/// the two images differ in size (not directly comparable).
#[derive(Debug, Clone, Copy)]
pub struct DiffStats {
    /// Fraction of pixels differing beyond the threshold (0..1).
    pub fraction: Option<f32>,
    /// Largest per-channel sRGB8 difference seen (0..1).
    pub max: Option<f32>,
}

/// Compare two images: a visualization plus a change metric. `Difference` and
/// `Heatmap` require equal sizes (the caller enforces it); `SideBySide` accepts
/// any. The metric is computed whenever the sizes match.
pub fn diff(a: &TiledImage, b: &TiledImage, view: DiffView, threshold: f32) -> (TiledImage, DiffStats) {
    let stats = if a.size() == b.size() {
        diff_stats(a, b, threshold)
    } else {
        DiffStats { fraction: None, max: None }
    };
    let img = match view {
        DiffView::SideBySide => side_by_side(a, b),
        DiffView::Difference => per_channel_map(a, b, |da, _| da as u8),
        DiffView::Heatmap => heatmap(a, b),
    };
    (img, stats)
}

fn diff_stats(a: &TiledImage, b: &TiledImage, threshold: f32) -> DiffStats {
    let (a8, b8) = (a.to_srgb_rgba8(), b.to_srgb_rgba8());
    let thr = (threshold.clamp(0.0, 1.0) * 255.0) as i32;
    let (mut count, mut maxd) = (0u64, 0i32);
    for (pa, pb) in a8.chunks_exact(4).zip(b8.chunks_exact(4)) {
        let d = (0..3).map(|c| (pa[c] as i32 - pb[c] as i32).abs()).max().unwrap();
        if d > thr {
            count += 1;
        }
        maxd = maxd.max(d);
    }
    let n = (a8.len() / 4) as f32;
    DiffStats { fraction: Some(count as f32 / n), max: Some(maxd as f32 / 255.0) }
}

/// Build a same-size image whose channels are `f(abs_diff, magnitude)`.
fn per_channel_map(a: &TiledImage, b: &TiledImage, f: impl Fn(i32, i32) -> u8 + Sync) -> TiledImage {
    let (a8, b8) = (a.to_srgb_rgba8(), b.to_srgb_rgba8());
    let mut out = vec![0u8; a8.len()];
    for (o, (pa, pb)) in out.chunks_exact_mut(4).zip(a8.chunks_exact(4).zip(b8.chunks_exact(4))) {
        let m = (0..3).map(|c| (pa[c] as i32 - pb[c] as i32).abs()).max().unwrap();
        for (c, slot) in o[..3].iter_mut().enumerate() {
            *slot = f((pa[c] as i32 - pb[c] as i32).abs(), m);
        }
        o[3] = 255;
    }
    TiledImage::from_srgb_rgba8(a.size(), &out, hash::digest_bytes(&out))
}

fn heatmap(a: &TiledImage, b: &TiledImage) -> TiledImage {
    let (a8, b8) = (a.to_srgb_rgba8(), b.to_srgb_rgba8());
    let mut out = vec![0u8; a8.len()];
    for (o, (pa, pb)) in out.chunks_exact_mut(4).zip(a8.chunks_exact(4).zip(b8.chunks_exact(4))) {
        let gray = (0.299 * pa[0] as f32 + 0.587 * pa[1] as f32 + 0.114 * pa[2] as f32) * 0.35;
        let m = (0..3).map(|c| (pa[c] as i32 - pb[c] as i32).abs()).max().unwrap() as f32;
        let fade = 1.0 - m / 255.0;
        o[0] = (gray + m).min(255.0) as u8;
        o[1] = (gray * fade).max(0.0) as u8;
        o[2] = (gray * fade).max(0.0) as u8;
        o[3] = 255;
    }
    TiledImage::from_srgb_rgba8(a.size(), &out, hash::digest_bytes(&out))
}

fn side_by_side(a: &TiledImage, b: &TiledImage) -> TiledImage {
    let gap = 8u32;
    let (aw, ah) = (a.size().width, a.size().height);
    let (bw, bh) = (b.size().width, b.size().height);
    let (cw, ch) = (aw + gap + bw, ah.max(bh));
    let bg = rgba8_to_linear_premul(Rgba8::rgb(20, 20, 20));
    let flat: Vec<f32> = std::iter::repeat_n(bg, (cw * ch) as usize).flatten().collect();
    let canvas = TiledImage::from_flat_f32(Size::new(cw, ch), &flat, hash_by_index);
    let canvas = blend(&canvas, a, 0, 0, 1.0);
    let out = blend(&canvas, b, (aw + gap) as i32, 0, 1.0);
    let mut params = Vec::with_capacity(16);
    params.extend_from_slice(b"side_by_side");
    params.extend_from_slice(&gap.to_le_bytes());
    let sig = hash::compose_signature(&params, a.tiles().iter().chain(b.tiles().iter()).map(|t| &t.hash));
    stamp(out, sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [f32; 4]) -> TiledImage {
        let flat: Vec<f32> = std::iter::repeat_n(rgba, (w * h) as usize).flatten().collect();
        TiledImage::from_flat_f32(Size::new(w, h), &flat, hash_by_index)
    }

    #[test]
    fn overlay_places_top_at_offset() {
        let base = solid(300, 300, [0.0, 0.0, 0.0, 1.0]); // opaque black
        let top = solid(100, 80, [1.0, 0.0, 0.0, 1.0]); // opaque red
        let out = overlay(&base, &top, 50, 40, 1.0);
        assert_eq!(out.pixel(60, 50), [1.0, 0.0, 0.0, 1.0], "inside the overlay");
        assert_eq!(out.pixel(10, 10), [0.0, 0.0, 0.0, 1.0], "outside stays base");
        assert_eq!(out.size(), Size::new(300, 300), "size follows base");
    }

    #[test]
    fn overlay_opacity_blends() {
        let base = solid(50, 50, [0.0, 0.0, 0.0, 1.0]);
        let top = solid(50, 50, [1.0, 1.0, 1.0, 1.0]);
        let out = overlay(&base, &top, 0, 0, 0.5);
        let p = out.pixel(25, 25);
        assert!((p[0] - 0.5).abs() < 1e-4, "50% over black = 0.5: {p:?}");
    }

    #[test]
    fn overlay_negative_offset_crops() {
        let base = solid(100, 100, [0.0, 0.0, 0.0, 1.0]);
        let top = solid(60, 60, [1.0, 0.0, 0.0, 1.0]);
        let out = overlay(&base, &top, -30, -30, 1.0);
        assert_eq!(out.pixel(5, 5), [1.0, 0.0, 0.0, 1.0], "visible corner of top");
        assert_eq!(out.pixel(50, 50), [0.0, 0.0, 0.0, 1.0], "past the cropped top");
    }

    #[test]
    fn collage_fills_target_width_and_covers_cells() {
        let a = solid(400, 300, [1.0, 0.0, 0.0, 1.0]);
        let b = solid(800, 400, [0.0, 1.0, 0.0, 1.0]);
        let c = solid(300, 300, [0.0, 0.0, 1.0, 1.0]);
        let opts = CollageOptions { target_width: 1000, row_height: 200, gap: 10, background: Rgba8::rgb(20, 20, 20) };
        let out = collage(&[&a, &b, &c], opts);
        assert_eq!(out.size().width, 1000);
        assert!(out.size().height > 0);
        // a cell interior shows its image color, not the background
        let (cw, ch) = (out.size().width, out.size().height);
        let mut found_colored = false;
        for &(x, y) in &[(cw / 4, ch / 4), (cw / 2, ch / 2), (3 * cw / 4, ch / 2)] {
            let p = out.pixel(x.min(cw - 1), y.min(ch - 1));
            if p[0] + p[1] + p[2] > 0.3 {
                found_colored = true;
            }
        }
        assert!(found_colored, "collage should show image content");
    }

    #[test]
    fn layout_rows_fit_width() {
        let sizes = vec![Size::new(400, 300); 6];
        let opts = CollageOptions { target_width: 900, row_height: 150, gap: 8, background: Rgba8::rgb(0, 0, 0) };
        let (canvas, rects) = justified_layout(&sizes, opts);
        assert_eq!(canvas.width, 900);
        for r in &rects {
            assert!(r.x + r.width <= 900, "cell {r:?} within width");
        }
    }

    #[test]
    fn diff_identical_is_zero() {
        let a = solid(64, 48, [0.5, 0.5, 0.5, 1.0]);
        let (img, s) = diff(&a, &a, DiffView::Difference, 0.0);
        assert_eq!(img.size(), Size::new(64, 48));
        assert_eq!(s.fraction, Some(0.0));
        assert_eq!(s.max, Some(0.0));
        assert_eq!(img.pixel(10, 10), [0.0, 0.0, 0.0, 1.0], "identical → black difference");
    }

    #[test]
    fn diff_detects_change_and_lays_out_side_by_side() {
        let a = solid(50, 50, [0.0, 0.0, 0.0, 1.0]);
        let b = solid(50, 50, [1.0, 1.0, 1.0, 1.0]);
        let (_, s) = diff(&a, &b, DiffView::Heatmap, 0.0);
        assert_eq!(s.fraction, Some(1.0), "every pixel changed");
        assert!(s.max.unwrap() > 0.99);
        // side-by-side of different sizes: width = aw + gap + bw, metric absent
        let c = solid(30, 40, [0.2, 0.2, 0.2, 1.0]);
        let (img, s) = diff(&a, &c, DiffView::SideBySide, 0.0);
        assert_eq!(img.size(), Size::new(50 + 8 + 30, 50), "side-by-side canvas");
        assert!(s.fraction.is_none(), "different sizes → no metric");
    }

    /// Content-addressed solid (distinct `id` ⇒ distinct tile hashes), mirroring how
    /// real images arrive (`from_srgb_rgba8` with a bytes digest).
    fn solid_id(w: u32, h: u32, rgb: [u8; 3], id: &[u8]) -> TiledImage {
        let px: Vec<u8> = std::iter::repeat_n([rgb[0], rgb[1], rgb[2], 255], (w * h) as usize).flatten().collect();
        TiledImage::from_srgb_rgba8(Size::new(w, h), &px, hash::digest_bytes(id))
    }

    #[test]
    fn compose_outputs_are_content_addressed() {
        // regression for the index-only hashing bug (BUGS.md): different content or
        // params must produce different tile identities so downstream caches can't collide.
        let base_a = solid_id(300, 300, [25, 25, 25], b"a");
        let base_b = solid_id(300, 300, [230, 230, 230], b"b");
        let top = solid_id(60, 60, [255, 0, 0], b"t");
        let oa = overlay(&base_a, &top, 10, 10, 1.0);
        let ob = overlay(&base_b, &top, 10, 10, 1.0);
        assert_ne!(oa.tiles()[0].hash, ob.tiles()[0].hash, "different base ⇒ different identity");
        assert_eq!(oa.tiles()[0].hash, overlay(&base_a, &top, 10, 10, 1.0).tiles()[0].hash, "deterministic");
        assert_ne!(oa.tiles()[0].hash, overlay(&base_a, &top, 20, 10, 1.0).tiles()[0].hash, "params matter");

        let opts = CollageOptions { target_width: 400, row_height: 150, gap: 8, background: Rgba8::rgb(0, 0, 0) };
        let ca = collage(&[&base_a, &top], opts);
        let cb = collage(&[&base_b, &top], opts);
        assert_ne!(ca.tiles()[0].hash, cb.tiles()[0].hash, "collage distinguishes inputs");
    }
}
