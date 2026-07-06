//! Codec benchmarks on a 24MP synthetic photo-ish image.
//! Run: `cargo bench -p craws-codecs` (append `-- --quick` for a fast pass).

use craws_codecs::{decode, encode, ImageFormat};
use criterion::{criterion_group, criterion_main, Criterion};

/// Smooth gradients + a little deterministic noise — compresses like a photo,
/// not like random static.
fn synthetic_photo(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    let mut state = 0x5EEDu32;
    for y in 0..h {
        for x in 0..w {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (state >> 28) as u8; // 0..15 noise
            v.push(((x * 255 / w) as u8).saturating_add(n));
            v.push(((y * 255 / h) as u8).saturating_add(n));
            v.push((((x + y) * 255 / (w + h)) as u8).saturating_add(n));
            v.push(255);
        }
    }
    v
}

fn bench_codecs(c: &mut Criterion) {
    let (w, h) = (6000, 4000);
    let px = synthetic_photo(w, h);

    let jpeg = encode(w, h, &px, ImageFormat::Jpeg, Some(90)).unwrap();
    let png = encode(w, h, &px, ImageFormat::Png, None).unwrap();

    let mut g = c.benchmark_group("codecs_24mp");
    g.sample_size(10);
    g.bench_function("decode_jpeg", |b| b.iter(|| decode(&jpeg).unwrap()));
    g.bench_function("decode_png", |b| b.iter(|| decode(&png).unwrap()));
    g.bench_function("encode_jpeg_q90", |b| {
        b.iter(|| encode(w, h, &px, ImageFormat::Jpeg, Some(90)).unwrap())
    });
    g.bench_function("encode_png", |b| b.iter(|| encode(w, h, &px, ImageFormat::Png, None).unwrap()));
    g.finish();
}

criterion_group!(benches, bench_codecs);
criterion_main!(benches);
