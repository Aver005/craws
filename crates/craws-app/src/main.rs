//! Craws GUI shell (M2 spike): three commands over the engine.
//! Pixels leave Rust exactly once per frame, as one raw binary payload
//! (`tauri::ipc::Response`) — 20-byte header + sRGB RGBA8. Never JSON.
//!
//! Console subsystem is kept on purpose: the auto-bench prints its result
//! line to stdout so headless runs can capture the numbers.

use craws_domain::{Filter, OpSpec, Pipeline, Size};
use craws_engine::{hash, Engine, TiledImage};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;

const FRAME_MAGIC: u32 = 0x5741_5243; // "CRAW" little-endian
/// The spike previews at a fixed viewport-class width; mip-matched tile
/// streaming replaces this constant in M3.
const PREVIEW_WIDTH: u32 = 1920;

struct AppState {
    engine: Engine,
    source: Mutex<Option<TiledImage>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceInfo {
    width: u32,
    height: u32,
    load_ms: f64,
}

/// Load an image file, or generate the synthetic 24MP sample when `path` is None.
#[tauri::command]
fn load_source(state: tauri::State<'_, AppState>, path: Option<String>) -> Result<SourceInfo, String> {
    let started = Instant::now();
    let img = match &path {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(|e| format!("reading {p}: {e}"))?;
            let decoded = craws_codecs::decode(&bytes).map_err(|e| e.to_string())?;
            TiledImage::from_srgb_rgba8(
                Size::new(decoded.width, decoded.height),
                &decoded.rgba8,
                hash::digest_bytes(&bytes),
            )
        }
        None => synthetic_24mp(),
    };
    let info = SourceInfo {
        width: img.size().width,
        height: img.size().height,
        load_ms: started.elapsed().as_secs_f64() * 1000.0,
    };
    *state.source.lock().unwrap_or_else(|e| e.into_inner()) = Some(img);
    Ok(info)
}

/// Run the preview pipeline and return the full frame as raw bytes.
#[tauri::command]
fn render(
    state: tauri::State<'_, AppState>,
    stops: f32,
    grayscale: bool,
) -> Result<tauri::ipc::Response, String> {
    let guard = state.source.lock().unwrap_or_else(|e| e.into_inner());
    let source = guard.as_ref().ok_or("no source loaded")?;

    let mut steps = vec![
        OpSpec::Resize { width: Some(PREVIEW_WIDTH), height: None, filter: Filter::Lanczos3 },
        OpSpec::Exposure { stops },
    ];
    if grayscale {
        steps.push(OpSpec::Grayscale);
    }

    let t_engine = Instant::now();
    let (out, _stats) = state
        .engine
        .run(source, &Pipeline { version: 0, steps })
        .map_err(|e| e.to_string())?;
    let engine_us = t_engine.elapsed().as_micros() as u32;

    let t_convert = Instant::now();
    let rgba = out.to_srgb_rgba8();
    let convert_us = t_convert.elapsed().as_micros() as u32;

    let size = out.size();
    let mut payload = Vec::with_capacity(20 + rgba.len());
    payload.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
    payload.extend_from_slice(&size.width.to_le_bytes());
    payload.extend_from_slice(&size.height.to_le_bytes());
    payload.extend_from_slice(&engine_us.to_le_bytes());
    payload.extend_from_slice(&convert_us.to_le_bytes());
    payload.extend_from_slice(&rgba);
    Ok(tauri::ipc::Response::new(payload))
}

/// The frontend's auto-bench posts its result here; stdout is the contract
/// for headless capture.
#[tauri::command]
fn report_bench(payload: String) {
    println!("BENCH_RESULT {payload}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Same look as `gen_sample`: gradients + rings + mild noise, 6000×4000.
fn synthetic_24mp() -> TiledImage {
    let (w, h) = (6000u32, 4000u32);
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
    TiledImage::from_srgb_rgba8(Size::new(w, h), &px, hash::digest_bytes(b"craws/sample-24mp/v1"))
}

fn main() {
    tauri::Builder::default()
        .manage(AppState { engine: Engine::new(), source: Mutex::new(None) })
        .invoke_handler(tauri::generate_handler![load_source, render, report_bench])
        .run(tauri::generate_context!())
        .expect("error while running craws-app");
}
