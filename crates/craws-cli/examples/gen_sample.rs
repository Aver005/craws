//! Generate a synthetic "photo" for demos and manual smoke tests.
//! `cargo run --release -p craws-cli --example gen_sample -- [path] [width] [height]`

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "sample.png".into());
    let w: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(6000);
    let h: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4000);

    // gradients + rings + mild noise: looks organic, compresses like a photo
    let mut px = Vec::with_capacity((w as usize) * (h as usize) * 4);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut state = 0x5EEDu32;
    for y in 0..h {
        for x in 0..w {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (state >> 28) as f32;
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let ring = (d / 90.0).sin() * 0.5 + 0.5;
            let r = (x as f32 / w as f32) * 200.0 + ring * 40.0 + noise;
            let g = (y as f32 / h as f32) * 180.0 + ring * 60.0 + noise;
            let b = 120.0 + ring * 90.0 + noise;
            px.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        }
    }
    let bytes = craws_codecs::encode(w, h, &px, craws_codecs::ImageFormat::Png, None)
        .expect("encode sample");
    std::fs::write(&path, bytes).expect("write sample");
    println!("wrote {path} ({w}x{h})");
}
