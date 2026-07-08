//! End-to-end: drive the real `craws` binary over real files.

use std::path::Path;
use std::process::Command;

fn craws() -> Command {
    Command::new(env!("CARGO_BIN_EXE_craws"))
}

/// 640×400 gradient PNG spanning multiple tiles.
fn write_sample_png(path: &Path) {
    let (w, h) = (640u32, 400u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x + y) * 255 / (w + h)) as u8,
                255,
            ]);
        }
    }
    let bytes = craws_codecs::encode(w, h, &px, craws_codecs::ImageFormat::Png, None).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn full_pipeline_png_to_webp() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let output = dir.path().join("out.webp");
    let pipeline = dir.path().join("pipeline.json");
    write_sample_png(&input);
    std::fs::write(
        &pipeline,
        r#"{
            "version": 0,
            "steps": [
                { "op": "resize", "width": 320 },
                { "op": "exposure", "stops": 0.3 },
                { "op": "crop", "x": 10, "y": 10, "width": 300, "height": 180 },
                { "op": "grayscale" }
            ]
        }"#,
    )
    .unwrap();

    let out = craws()
        .args(["run"])
        .arg(&pipeline)
        .arg("--in").arg(&input)
        .arg("--out").arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let decoded = craws_codecs::decode(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!((decoded.width, decoded.height), (300, 180));
    // grayscale means R==G==B everywhere
    let p = &decoded.rgba8[..4];
    assert!(p[0] == p[1] && p[1] == p[2], "expected gray pixel, got {p:?}");
    // timing report lands on stderr by default
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("total"), "report expected, got: {stderr}");
}

#[test]
fn invalid_pipeline_fails_with_context() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let output = dir.path().join("out.png");
    let pipeline = dir.path().join("pipeline.json");
    write_sample_png(&input);
    std::fs::write(
        &pipeline,
        r#"{ "steps": [ { "op": "crop", "x": 0, "y": 0, "width": 9999, "height": 10 } ] }"#,
    )
    .unwrap();

    let out = craws()
        .args(["run"])
        .arg(&pipeline)
        .arg("--in").arg(&input)
        .arg("--out").arg(&output)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("crop"), "error should name the failing op: {stderr}");
    assert!(!output.exists(), "no output on failure");
}

#[test]
fn quiet_suppresses_report() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    let output = dir.path().join("out.png");
    let pipeline = dir.path().join("pipeline.json");
    write_sample_png(&input);
    std::fs::write(&pipeline, r#"{ "steps": [ { "op": "grayscale" } ] }"#).unwrap();

    let out = craws()
        .args(["run"])
        .arg(&pipeline)
        .arg("--in").arg(&input)
        .arg("--out").arg(&output)
        .arg("--quiet")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stderr.is_empty(), "quiet run must print nothing: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn ops_lists_every_op() {
    let out = craws().arg("ops").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for op in [
        "resize", "crop", "rotate", "flip", "pad", "exposure", "grayscale",
        "hue_rotate", "invert", "brightness_contrast", "saturation", "levels", "curves",
        "white_balance", "gradient_map", "blur", "sharpen", "vignette",
        "redact", "spotlight", "beautify",
        "draw_rect", "draw_ellipse", "draw_line", "draw_arrow", "draw_text",
    ] {
        assert!(stdout.contains(op), "missing {op}");
    }
}
