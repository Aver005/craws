//! Separable, precomputed-weight image resampling — the engine's own kernel.
//!
//! Resampling runs on premultiplied linear f32 (the internal format), so there
//! is no gamma-space bleed and no dark-edge fringing by construction. The
//! filter is applied as two 1-D passes (horizontal then vertical); the weights
//! for each output pixel are computed once and reused across the whole line.
//! Both passes are rayon-parallel.
//!
//! Downsampling stretches the filter (support × 1/scale) so it low-passes and
//! doesn't alias — the same thing any quality resampler does.

use craws_domain::Filter;

/// Per-output-pixel contribution: a contiguous run of input samples + weights.
struct Contrib {
    start: u32,
    weights: Vec<f32>,
}

/// The weight plan for one axis (input length → output length under a filter).
pub struct Contribs {
    items: Vec<Contrib>,
}

impl Contribs {
    pub fn new(in_len: u32, out_len: u32, filter: Filter) -> Self {
        debug_assert!(in_len > 0 && out_len > 0);
        let scale = out_len as f64 / in_len as f64;
        // when shrinking, widen the filter footprint to low-pass the input
        let filter_scale = if scale < 1.0 { 1.0 / scale } else { 1.0 };
        let support = support(filter) as f64 * filter_scale;

        let items = (0..out_len)
            .map(|ox| {
                // input-space coordinate of this output pixel's center
                let center = (ox as f64 + 0.5) / scale;
                let left = (center - support).floor() as i64;
                let right = (center + support).ceil() as i64;
                let start = left.clamp(0, in_len as i64 - 1) as u32;
                let end = right.clamp(0, in_len as i64 - 1) as u32;

                let mut weights = Vec::with_capacity((end - start + 1) as usize);
                let mut sum = 0.0f64;
                for ix in start..=end {
                    let t = (ix as f64 + 0.5 - center) / filter_scale;
                    let w = kernel(filter, t);
                    weights.push(w as f32);
                    sum += w;
                }
                // normalize so a flat input is reproduced exactly (weights sum to 1)
                if sum.abs() > 1e-12 {
                    let inv = (1.0 / sum) as f32;
                    for w in &mut weights {
                        *w *= inv;
                    }
                } else {
                    // degenerate (e.g. nearest landing between samples): take one tap
                    weights.clear();
                    weights.push(1.0);
                    return Contrib { start: (center as u32).min(in_len - 1), weights };
                }
                Contrib { start, weights }
            })
            .collect();
        Self { items }
    }

    pub fn out_len(&self) -> usize {
        self.items.len()
    }

    /// Resample one line: `src` holds `in_len` RGBA pixels (stride 4), `dst`
    /// holds `out_len` RGBA pixels. Weighted sum per channel.
    fn apply_line(&self, src: &[f32], dst: &mut [f32]) {
        for (ox, c) in self.items.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            let base = c.start as usize * 4;
            for (k, &w) in c.weights.iter().enumerate() {
                let s = &src[base + k * 4..base + k * 4 + 4];
                acc[0] += w * s[0];
                acc[1] += w * s[1];
                acc[2] += w * s[2];
                acc[3] += w * s[3];
            }
            dst[ox * 4..ox * 4 + 4].copy_from_slice(&acc);
        }
    }
}

fn support(filter: Filter) -> f32 {
    match filter {
        Filter::Nearest => 0.5,
        Filter::Bilinear => 1.0,
        Filter::CatmullRom => 2.0,
        Filter::Lanczos3 => 3.0,
    }
}

fn kernel(filter: Filter, x: f64) -> f64 {
    match filter {
        Filter::Nearest => {
            if x.abs() < 0.5 {
                1.0
            } else {
                0.0
            }
        }
        Filter::Bilinear => {
            let x = x.abs();
            if x < 1.0 {
                1.0 - x
            } else {
                0.0
            }
        }
        Filter::CatmullRom => catmull_rom(x),
        Filter::Lanczos3 => lanczos(x, 3.0),
    }
}

/// Catmull-Rom (Keys cubic, a = -0.5).
fn catmull_rom(x: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

fn lanczos(x: f64, a: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else if x.abs() < a {
        let px = std::f64::consts::PI * x;
        a * px.sin() * (px / a).sin() / (px * px)
    } else {
        0.0
    }
}

/// Horizontal pass: `src` is a full `in_w × in_h` RGBA f32 line-source accessed
/// row by row via `row(y, &mut buf)`; returns the intermediate `out_w × in_h`.
/// Kept generic over the row source so the caller can gather rows straight from
/// tiles without materializing the whole source as one flat buffer.
pub fn horizontal<F>(cx: &Contribs, in_w: u32, in_h: u32, row: F) -> Vec<f32>
where
    F: Fn(u32, &mut [f32]) + Sync,
{
    use rayon::prelude::*;
    let out_w = cx.out_len();
    let mut inter = vec![0.0f32; out_w * in_h as usize * 4];
    inter
        .par_chunks_exact_mut(out_w * 4)
        .enumerate()
        .for_each_init(
            || vec![0.0f32; in_w as usize * 4],
            |src_row, (y, dst_row)| {
                row(y as u32, src_row);
                cx.apply_line(src_row, dst_row);
            },
        );
    inter
}

/// Vertical pass: `inter` is `out_w × in_h`, returns the final `out_w × out_h`.
pub fn vertical(cy: &Contribs, inter: &[f32], out_w: u32) -> Vec<f32> {
    use rayon::prelude::*;
    let out_w = out_w as usize;
    let out_h = cy.out_len();
    let mut out = vec![0.0f32; out_w * out_h * 4];
    out.par_chunks_exact_mut(out_w * 4).enumerate().for_each(|(oy, dst_row)| {
        let c = &cy.items[oy];
        // accumulate contiguous intermediate rows — vectorizable
        for (k, &w) in c.weights.iter().enumerate() {
            let sy = c.start as usize + k;
            let src_row = &inter[sy * out_w * 4..(sy + 1) * out_w * 4];
            for (d, s) in dst_row.iter_mut().zip(src_row) {
                *d += w * s;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resample_flat(src: &[f32], in_w: u32, in_h: u32, out_w: u32, out_h: u32, f: Filter) -> Vec<f32> {
        let cx = Contribs::new(in_w, out_w, f);
        let cy = Contribs::new(in_h, out_h, f);
        let inter = horizontal(&cx, in_w, in_h, |y, dst| {
            let row = &src[y as usize * in_w as usize * 4..][..in_w as usize * 4];
            dst.copy_from_slice(row);
        });
        vertical(&cy, &inter, out_w)
    }

    fn solid(w: u32, h: u32, rgba: [f32; 4]) -> Vec<f32> {
        std::iter::repeat_n(rgba, (w * h) as usize).flatten().collect()
    }

    #[test]
    fn weights_sum_to_one() {
        for f in [Filter::Nearest, Filter::Bilinear, Filter::CatmullRom, Filter::Lanczos3] {
            for (a, b) in [(100u32, 37u32), (37, 100), (256, 256), (1000, 250)] {
                let c = Contribs::new(a, b, f);
                for item in &c.items {
                    let s: f32 = item.weights.iter().sum();
                    assert!((s - 1.0).abs() < 1e-4, "{f:?} {a}->{b}: sum {s}");
                }
            }
        }
    }

    #[test]
    fn solid_stays_solid_all_filters() {
        for f in [Filter::Nearest, Filter::Bilinear, Filter::CatmullRom, Filter::Lanczos3] {
            let src = solid(200, 150, [0.3, 0.6, 0.9, 1.0]);
            for (ow, oh) in [(80u32, 60u32), (400, 300), (37, 211)] {
                let out = resample_flat(&src, 200, 150, ow, oh, f);
                for (i, px) in out.chunks_exact(4).enumerate() {
                    assert!(
                        (px[0] - 0.3).abs() < 1e-3 && (px[1] - 0.6).abs() < 1e-3 && (px[2] - 0.9).abs() < 1e-3,
                        "{f:?} {ow}x{oh} px {i}: {px:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn identity_size_is_near_lossless() {
        // resampling to the same size should return essentially the input
        let mut src = vec![0.0f32; 16 * 16 * 4];
        for (i, p) in src.chunks_exact_mut(4).enumerate() {
            p[0] = (i % 16) as f32 / 15.0;
            p[3] = 1.0;
        }
        let out = resample_flat(&src, 16, 16, 16, 16, Filter::Lanczos3);
        for (a, b) in out.iter().zip(&src) {
            assert!((a - b).abs() < 1e-3, "identity drift {a} vs {b}");
        }
    }

    #[test]
    fn nearest_upsample_picks_source_pixels() {
        // 2x1 → 4x1 nearest: outputs mirror the two source pixels
        let src = vec![0.0, 0.0, 0.0, 1.0, /**/ 1.0, 1.0, 1.0, 1.0];
        let out = resample_flat(&src, 2, 1, 4, 1, Filter::Nearest);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[3], 1.0);
        assert_eq!(out[12], 1.0, "last output samples the right pixel: {out:?}");
    }
}
