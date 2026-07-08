//! Engine benchmarks on a 24MP (6000×4000) synthetic image.
//! Run: `cargo bench -p craws-engine` (append `-- --quick` for a fast pass).
//! Record headline numbers in `.memories/JOURNAL/`.

use craws_domain::{Filter, OpSpec, Pipeline, RedactMode, Size};
use craws_engine::{compose, hash, Engine, TiledImage};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

fn synthetic_rgba(size: Size) -> Vec<u8> {
    let mut rgba = vec![0u8; size.area() as usize * 4];
    let mut state = 0xBEEFCAFEu32;
    for (i, b) in rgba.iter_mut().enumerate() {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *b = if i % 4 == 3 { 255 } else { (state >> 24) as u8 };
    }
    rgba
}

fn pipeline(steps: Vec<OpSpec>) -> Pipeline {
    Pipeline { version: 0, steps }
}

fn bench_engine(c: &mut Criterion) {
    let size = Size::new(6000, 4000);
    let rgba = synthetic_rgba(size);
    let src = hash::digest_bytes(&rgba);
    let img = TiledImage::from_srgb_rgba8(size, &rgba, src);

    // ── boundary conversions (also proxies for the future viewport bridge) ──
    let mut g = c.benchmark_group("convert_24mp");
    g.sample_size(10);
    g.bench_function("srgb8_to_tiles", |b| {
        b.iter(|| TiledImage::from_srgb_rgba8(size, &rgba, src))
    });
    g.bench_function("tiles_to_srgb8", |b| b.iter(|| img.to_srgb_rgba8()));
    g.finish();
    drop(rgba);

    // ── pipeline execution ──
    let mut g = c.benchmark_group("engine_24mp");
    g.sample_size(10);

    let exposure = pipeline(vec![OpSpec::Exposure { stops: 0.5 }]);
    g.bench_function("exposure_cold", |b| {
        b.iter_batched(
            Engine::new,
            |e| e.run(&img, &exposure).unwrap(),
            BatchSize::PerIteration,
        )
    });
    g.bench_function("exposure_cached", |b| {
        let e = Engine::new();
        e.run(&img, &exposure).unwrap();
        b.iter(|| e.run(&img, &exposure).unwrap())
    });

    let resize = pipeline(vec![OpSpec::Resize {
        width: Some(1920),
        height: None,
        filter: Filter::Lanczos3,
    }]);
    g.bench_function("resize_to_1920_lanczos3_cold", |b| {
        b.iter_batched(Engine::new, |e| e.run(&img, &resize).unwrap(), BatchSize::PerIteration)
    });

    // whole-image Gaussian blur: tile-row banded/streaming (no full-image flat copies)
    let blur = pipeline(vec![OpSpec::Blur { radius: 8.0 }]);
    g.bench_function("blur_r8_cold", |b| {
        b.iter_batched(Engine::new, |e| e.run(&img, &blur).unwrap(), BatchSize::PerIteration)
    });

    // redact a large region by mosaic (tile-direct region read, no pixel())
    let redact = pipeline(vec![OpSpec::Redact {
        x: 500.0,
        y: 400.0,
        width: 2000.0,
        height: 1500.0,
        mode: RedactMode::Pixelate { block: 16 },
    }]);
    g.bench_function("redact_pixelate_cold", |b| {
        b.iter_batched(Engine::new, |e| e.run(&img, &redact).unwrap(), BatchSize::PerIteration)
    });

    let chain = pipeline(vec![
        OpSpec::Resize { width: Some(1920), height: None, filter: Filter::Lanczos3 },
        OpSpec::Exposure { stops: 0.5 },
        OpSpec::Crop { x: 160, y: 100, width: 1600, height: 1000 },
        OpSpec::Grayscale,
    ]);
    g.bench_function("chain_resize_exposure_crop_gray_cold", |b| {
        b.iter_batched(Engine::new, |e| e.run(&img, &chain).unwrap(), BatchSize::PerIteration)
    });
    // interactive scenario: chain warm, one slider (exposure) changes per frame
    g.bench_function("chain_slider_tweak_warm", |b| {
        let e = Engine::new();
        e.run(&img, &chain).unwrap();
        let mut stops = 0.5f32;
        b.iter(|| {
            stops = if stops > 2.0 { 0.5 } else { stops + 0.01 };
            let p = pipeline(vec![
                OpSpec::Resize { width: Some(1920), height: None, filter: Filter::Lanczos3 },
                OpSpec::Exposure { stops },
                OpSpec::Crop { x: 160, y: 100, width: 1600, height: 1000 },
                OpSpec::Grayscale,
            ]);
            e.run(&img, &p).unwrap()
        })
    });
    g.finish();

    // ── composition (multi-input; tile-direct blend, no pixel()) ──
    let top = TiledImage::from_srgb_rgba8(Size::new(1600, 1200), &synthetic_rgba(Size::new(1600, 1200)), hash::digest_bytes(b"top"));
    let mut g = c.benchmark_group("compose_24mp");
    g.sample_size(10);
    g.bench_function("overlay_2mp_on_24mp", |b| b.iter(|| compose::overlay(&img, &top, 200, 150, 0.9)));
    g.finish();
}

criterion_group!(benches, bench_engine);
criterion_main!(benches);
