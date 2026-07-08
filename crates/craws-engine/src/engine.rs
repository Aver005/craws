//! The executor: walks a pipeline, derives content hashes, consults the cache,
//! computes only what's missing, and reports per-node stats.

use crate::cache::TileCache;
use crate::draw;
use crate::hash::{self, ContentHash};
use crate::ops;
use crate::tile::{grid_dims, Tile, TileRef, TiledImage};
use craws_domain::{OpSpec, Pipeline, PipelineError, Rect};
use rayon::prelude::*;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Default tile-cache budget: 2 GiB.
pub const DEFAULT_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
}

/// Per-step execution report.
#[derive(Debug, Clone)]
pub struct NodeStat {
    pub name: &'static str,
    pub duration: Duration,
    /// tiles in this step's output
    pub tiles: u32,
    /// tiles actually computed (the rest came from the cache)
    pub computed: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RunStats {
    pub nodes: Vec<NodeStat>,
}

impl RunStats {
    pub fn total_duration(&self) -> Duration {
        self.nodes.iter().map(|n| n.duration).sum()
    }

    pub fn total_computed(&self) -> u32 {
        self.nodes.iter().map(|n| n.computed).sum()
    }
}

pub struct Engine {
    cache: Mutex<TileCache>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::with_cache_budget(DEFAULT_CACHE_BYTES)
    }

    pub fn with_cache_budget(bytes: u64) -> Self {
        Self { cache: Mutex::new(TileCache::new(bytes)) }
    }

    /// Execute `pipeline` over `input`. Returns the result and per-node stats.
    ///
    /// Deterministic: same input identity + same pipeline ⇒ bit-identical
    /// output (whether tiles came from cache or recomputation).
    pub fn run(
        &self,
        input: &TiledImage,
        pipeline: &Pipeline,
    ) -> Result<(TiledImage, RunStats), EngineError> {
        let sizes = pipeline.validate(input.size())?;
        let mut cur = input.clone();
        let mut stats = RunStats::default();

        for (step, out_size) in pipeline.steps.iter().zip(sizes) {
            let started = Instant::now();
            let op = hash::op_digest(step);
            let (next, tiles, computed) = match *step {
                OpSpec::Exposure { stops } => {
                    let factor = 2.0f32.powf(stops);
                    self.run_pointwise(&cur, op, move |t| ops::exposure(t, factor))
                }
                OpSpec::Grayscale => self.run_pointwise(&cur, op, ops::grayscale),
                OpSpec::Crop { x, y, width, height } => {
                    let rect = Rect::new(x, y, width, height);
                    self.run_global(&cur, out_size, op, |img, hash_fn| ops::crop(img, rect, hash_fn))
                }
                OpSpec::Resize { filter, .. } => self.run_global(&cur, out_size, op, |img, hash_fn| {
                    ops::resize(img, out_size, filter, hash_fn)
                }),
                OpSpec::Rotate { degrees, expand } => self.run_global(&cur, out_size, op, |img, hash_fn| {
                    ops::rotate(img, out_size, degrees, expand, hash_fn)
                }),
                OpSpec::Flip { axis } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| ops::flip(img, axis, hash_fn))
                }
                OpSpec::Pad { left, top, color, .. } => self.run_global(&cur, out_size, op, |img, hash_fn| {
                    ops::pad(img, out_size, left, top, color, hash_fn)
                }),
                // color: pointwise
                OpSpec::HueRotate { degrees } => {
                    let m = ops::hue_matrix(degrees);
                    self.run_pointwise(&cur, op, move |t| ops::hue_rotate(t, &m))
                }
                OpSpec::Invert => self.run_pointwise(&cur, op, ops::invert),
                // filter: whole-image / region
                OpSpec::Blur { radius } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| crate::filter::blur(img, radius, hash_fn))
                }
                OpSpec::Redact { x, y, width, height, mode } => self.run_global(&cur, out_size, op, |img, hash_fn| {
                    crate::filter::redact(img, x, y, width, height, mode, hash_fn)
                }),
                OpSpec::Spotlight { x, y, width, height, corner_radius, dim, color, feather } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        draw::spotlight(img, x, y, width, height, corner_radius, dim, color, feather, hash_fn)
                    })
                }
                OpSpec::Beautify { padding, corner_radius, shadow_radius, shadow_opacity, shadow_offset, background } => {
                    let params = crate::filter::BeautifyParams {
                        padding,
                        corner_radius,
                        shadow_radius,
                        shadow_opacity,
                        shadow_offset,
                        background,
                    };
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        crate::filter::beautify(img, out_size, &params, hash_fn)
                    })
                }
                // annotation ops: same size, only bbox tiles recompute (inside draw)
                OpSpec::DrawRect { x, y, width, height, corner_radius, fill, stroke, stroke_width } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        draw::rect(img, x, y, width, height, corner_radius, fill, stroke, stroke_width, hash_fn)
                    })
                }
                OpSpec::DrawEllipse { x, y, width, height, fill, stroke, stroke_width } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        draw::ellipse(img, x, y, width, height, fill, stroke, stroke_width, hash_fn)
                    })
                }
                OpSpec::DrawLine { x1, y1, x2, y2, color, thickness } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        draw::line(img, x1, y1, x2, y2, color, thickness, hash_fn)
                    })
                }
                OpSpec::DrawArrow { x1, y1, x2, y2, color, thickness, head_length } => {
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        draw::arrow(img, x1, y1, x2, y2, color, thickness, head_length, hash_fn)
                    })
                }
                // text: String/Option fields aren't Copy, so re-borrow from `step`
                OpSpec::DrawText { .. } => {
                    let OpSpec::DrawText { x, y, text, color, font_size, font, align_x, align_y, line_height } = step
                    else {
                        unreachable!()
                    };
                    let face = crate::fonts::load(font.as_deref());
                    let params = crate::text::TextParams {
                        x: *x,
                        y: *y,
                        text: text.clone(),
                        color: *color,
                        font_size: *font_size,
                        align_x: *align_x,
                        align_y: *align_y,
                        line_height: *line_height,
                    };
                    self.run_global(&cur, out_size, op, |img, hash_fn| {
                        crate::text::draw(img, &params, &face, hash_fn)
                    })
                }
            };
            stats.nodes.push(NodeStat {
                name: step.name(),
                duration: started.elapsed(),
                tiles,
                computed,
            });
            cur = next;
        }
        Ok((cur, stats))
    }

    /// tile → tile ops: per-tile cache keys, misses computed in parallel.
    fn run_pointwise(
        &self,
        input: &TiledImage,
        op: ContentHash,
        f: impl Fn(&Tile) -> Tile + Sync,
    ) -> (TiledImage, u32, u32) {
        let hashes: Vec<ContentHash> =
            input.tiles().iter().map(|t| hash::pointwise_tile_hash(op, t.hash)).collect();

        // phase 1: one short lock to collect hits
        let mut out: Vec<Option<Arc<Tile>>> = {
            let mut c = self.lock_cache();
            hashes.iter().map(|h| c.get(h)).collect()
        };

        // phase 2: compute misses in parallel, lock-free
        let miss_idx: Vec<usize> =
            out.iter().enumerate().filter(|(_, o)| o.is_none()).map(|(i, _)| i).collect();
        let computed: Vec<(usize, Arc<Tile>)> = miss_idx
            .into_par_iter()
            .map(|i| (i, Arc::new(f(&input.tiles()[i].tile))))
            .collect();
        let n_computed = computed.len() as u32;

        // phase 3: one short lock to publish
        {
            let mut c = self.lock_cache();
            for (i, t) in computed {
                c.insert(hashes[i], Arc::clone(&t));
                out[i] = Some(t);
            }
        }

        let tiles: Vec<TileRef> = out
            .into_iter()
            .zip(&hashes)
            .map(|(t, h)| TileRef { hash: *h, tile: t.expect("all misses computed") })
            .collect();
        let n = tiles.len() as u32;
        (TiledImage::new(input.size(), tiles), n, n_computed)
    }

    /// Whole-image ops: all output tiles share one signature. Either every
    /// output tile is cached (skip compute entirely) or the op runs in full.
    fn run_global(
        &self,
        input: &TiledImage,
        out_size: craws_domain::Size,
        op: ContentHash,
        compute: impl FnOnce(&TiledImage, &(dyn Fn(u32) -> ContentHash + Sync)) -> TiledImage,
    ) -> (TiledImage, u32, u32) {
        let sig = hash::global_signature(op, input.tiles().iter().map(|t| &t.hash));
        let (cols, rows) = grid_dims(out_size);
        let n = cols * rows;
        let hashes: Vec<ContentHash> = (0..n).map(|i| hash::global_tile_hash(sig, i)).collect();

        {
            let mut c = self.lock_cache();
            let cached: Option<Vec<Arc<Tile>>> = hashes.iter().map(|h| c.get(h)).collect();
            if let Some(tiles) = cached {
                let tiles = tiles
                    .into_iter()
                    .zip(&hashes)
                    .map(|(tile, h)| TileRef { hash: *h, tile })
                    .collect();
                return (TiledImage::new(out_size, tiles), n, 0);
            }
        }

        let result = compute(input, &|i| hash::global_tile_hash(sig, i));
        {
            let mut c = self.lock_cache();
            for t in result.tiles() {
                c.insert(t.hash, Arc::clone(&t.tile));
            }
        }
        (result, n, n)
    }

    fn lock_cache(&self) -> MutexGuard<'_, TileCache> {
        // a poisoned cache is still a valid cache — correctness never depends on it
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;
    use craws_domain::{Filter, FlipAxis, RedactMode, Rgba8, Size};

    /// Deterministic sRGB test image spanning several tiles.
    fn test_image(w: u32, h: u32, seed: &[u8]) -> TiledImage {
        let size = Size::new(w, h);
        let mut rgba = vec![0u8; size.area() as usize * 4];
        let mut state = 0xC0FFEEu32;
        for (i, b) in rgba.iter_mut().enumerate() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = if i % 4 == 3 { 255 } else { (state >> 24) as u8 };
        }
        TiledImage::from_srgb_rgba8(size, &rgba, digest_bytes(seed))
    }

    fn pipeline(steps: Vec<OpSpec>) -> Pipeline {
        Pipeline { version: 0, steps }
    }

    #[test]
    fn second_run_is_fully_cached_and_identical() {
        let engine = Engine::new();
        let img = test_image(600, 520, b"src");
        let p = pipeline(vec![
            OpSpec::Resize { width: Some(300), height: None, filter: Filter::Bilinear },
            OpSpec::Exposure { stops: 0.5 },
            OpSpec::Grayscale,
        ]);

        let (out1, s1) = engine.run(&img, &p).unwrap();
        assert!(s1.total_computed() > 0);

        let (out2, s2) = engine.run(&img, &p).unwrap();
        assert_eq!(s2.total_computed(), 0, "everything served from cache: {s2:?}");
        assert_eq!(out1.to_srgb_rgba8(), out2.to_srgb_rgba8(), "determinism");
    }

    #[test]
    fn param_change_recomputes_only_downstream() {
        let engine = Engine::new();
        let img = test_image(600, 300, b"src");
        let base = |stops| {
            pipeline(vec![
                OpSpec::Resize { width: Some(400), height: None, filter: Filter::Bilinear },
                OpSpec::Exposure { stops },
                OpSpec::Grayscale,
            ])
        };

        engine.run(&img, &base(0.5)).unwrap();
        let (_, s) = engine.run(&img, &base(0.7)).unwrap();

        assert_eq!(s.nodes[0].computed, 0, "resize untouched by the exposure tweak");
        assert!(s.nodes[1].computed > 0, "exposure recomputed");
        assert!(s.nodes[2].computed > 0, "grayscale input changed");

        // and the original pipeline is still warm
        let (_, s) = engine.run(&img, &base(0.5)).unwrap();
        assert_eq!(s.total_computed(), 0);
    }

    #[test]
    fn different_source_never_hits_foreign_cache() {
        let engine = Engine::new();
        let p = pipeline(vec![OpSpec::Exposure { stops: 1.0 }]);
        let (_, s1) = engine.run(&test_image(300, 300, b"one"), &p).unwrap();
        let (_, s2) = engine.run(&test_image(300, 300, b"two"), &p).unwrap();
        assert_eq!(s1.total_computed(), s2.total_computed(), "both cold");
        assert!(s2.total_computed() > 0);
    }

    #[test]
    fn exposure_via_engine_matches_math() {
        let engine = Engine::new();
        let size = Size::new(10, 10);
        let flat: Vec<f32> = std::iter::repeat_n([0.25f32, 0.25, 0.25, 1.0], 100).flatten().collect();
        let img = TiledImage::from_flat_f32(size, &flat, |i| digest_bytes(&i.to_le_bytes()));
        let (out, _) = engine.run(&img, &pipeline(vec![OpSpec::Exposure { stops: 1.0 }])).unwrap();
        assert_eq!(out.pixel(5, 5), [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn geometry_ops_run_and_cache() {
        let engine = Engine::new();
        let img = test_image(500, 300, b"geo");
        let p = pipeline(vec![
            OpSpec::Rotate { degrees: 90.0, expand: true },
            OpSpec::Flip { axis: FlipAxis::Horizontal },
            OpSpec::Pad { left: 8, right: 8, top: 8, bottom: 8, color: Rgba8::new(0, 0, 0, 0) },
        ]);
        // 500×300 --rot90--> 300×500 --flip--> 300×500 --pad8--> 316×516
        let (out1, s1) = engine.run(&img, &p).unwrap();
        assert_eq!(out1.size(), Size::new(316, 516));
        assert!(s1.total_computed() > 0);

        let (out2, s2) = engine.run(&img, &p).unwrap();
        assert_eq!(s2.total_computed(), 0, "second run fully cached");
        assert_eq!(out1.to_srgb_rgba8(), out2.to_srgb_rgba8(), "deterministic geometry chain");
    }

    #[test]
    fn color_and_filter_chain_runs_and_caches() {
        let engine = Engine::new();
        let img = test_image(400, 300, b"cf");
        let p = pipeline(vec![
            OpSpec::HueRotate { degrees: 45.0 },
            OpSpec::Invert,
            OpSpec::Blur { radius: 2.0 },
            OpSpec::Redact { x: 20.0, y: 20.0, width: 60.0, height: 40.0, mode: RedactMode::Fill { color: Rgba8::rgb(0, 0, 0) } },
            OpSpec::Spotlight { x: 100.0, y: 100.0, width: 120.0, height: 90.0, corner_radius: 8.0, dim: 0.5, color: Rgba8::rgb(0, 0, 0), feather: 4.0 },
            OpSpec::Beautify { padding: 20, corner_radius: 12.0, shadow_radius: 8.0, shadow_opacity: 0.3, shadow_offset: 6.0, background: Rgba8::new(0, 0, 0, 0) },
        ]);
        let (out1, s1) = engine.run(&img, &p).unwrap();
        assert_eq!(out1.size(), Size::new(440, 340), "beautify adds 2·padding");
        assert!(s1.total_computed() > 0);

        let (out2, s2) = engine.run(&img, &p).unwrap();
        assert_eq!(s2.total_computed(), 0, "second run fully cached");
        assert_eq!(out1.to_srgb_rgba8(), out2.to_srgb_rgba8(), "deterministic color+filter chain");
    }

    #[test]
    fn validation_failure_surfaces() {
        let engine = Engine::new();
        let img = test_image(100, 100, b"src");
        let p = pipeline(vec![OpSpec::Crop { x: 0, y: 0, width: 500, height: 500 }]);
        assert!(matches!(engine.run(&img, &p), Err(EngineError::Pipeline(_))));
    }

    #[test]
    fn tiny_cache_budget_still_correct() {
        // budget too small to keep anything: every run recomputes, output identical
        let engine = Engine::with_cache_budget(1);
        let img = test_image(300, 300, b"src");
        let p = pipeline(vec![OpSpec::Exposure { stops: 0.3 }, OpSpec::Grayscale]);
        let (a, s1) = engine.run(&img, &p).unwrap();
        let (b, s2) = engine.run(&img, &p).unwrap();
        assert_eq!(s1.total_computed(), s2.total_computed(), "nothing could be cached");
        assert_eq!(a.to_srgb_rgba8(), b.to_srgb_rgba8());
    }
}
