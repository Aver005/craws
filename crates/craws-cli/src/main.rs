//! `craws` — headless pipeline runner. The first of the three equal ports
//! (CLI / MCP / GUI) over `craws-engine`, and the engine's perpetual smoke test.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use craws_codecs::ImageFormat;
use craws_domain::{Pipeline, Size};
use craws_engine::{hash, Engine, RunStats, TiledImage};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "craws", version, about = "Blazing-fast, automation-first image pipelines")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a pipeline JSON over an image
    Run {
        /// Path to pipeline.json
        pipeline: PathBuf,
        /// Input image (png / jpg / webp)
        #[arg(long = "in", value_name = "FILE")]
        input: PathBuf,
        /// Output image; format follows the extension
        #[arg(long = "out", value_name = "FILE")]
        output: PathBuf,
        /// JPEG quality 1–100 (ignored for png/webp)
        #[arg(long, default_value_t = 90)]
        quality: u8,
        /// Suppress the timing report
        #[arg(long, short)]
        quiet: bool,
    },
    /// List available pipeline operations
    Ops,
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Run { pipeline, input, output, quality, quiet } => {
            run(&pipeline, &input, &output, quality, quiet)
        }
        Cmd::Ops => {
            print!("{}", OPS_HELP);
            Ok(())
        }
    }
}

fn run(pipeline_path: &Path, input: &Path, output: &Path, quality: u8, quiet: bool) -> Result<()> {
    let total = Instant::now();

    let format = ImageFormat::from_path(output)
        .with_context(|| format!("unsupported output extension: {}", output.display()))?;
    let pipeline: Pipeline = serde_json::from_str(
        &std::fs::read_to_string(pipeline_path)
            .with_context(|| format!("reading {}", pipeline_path.display()))?,
    )
    .with_context(|| format!("parsing {}", pipeline_path.display()))?;

    let bytes =
        std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;

    let t = Instant::now();
    let decoded = craws_codecs::decode(&bytes).context("decoding input")?;
    let d_decode = t.elapsed();

    let in_size = Size::new(decoded.width, decoded.height);
    let t = Instant::now();
    let img = TiledImage::from_srgb_rgba8(in_size, &decoded.rgba8, hash::digest_bytes(&bytes));
    let d_ingest = t.elapsed();
    drop(decoded);

    let engine = Engine::new();
    let (result, stats) = engine.run(&img, &pipeline).context("running pipeline")?;
    let out_size = result.size();

    let t = Instant::now();
    let rgba = result.to_srgb_rgba8();
    let encoded = craws_codecs::encode(out_size.width, out_size.height, &rgba, format, Some(quality))
        .context("encoding output")?;
    std::fs::write(output, &encoded).with_context(|| format!("writing {}", output.display()))?;
    let d_encode = t.elapsed();

    if !quiet {
        report(input, output, in_size, out_size, d_decode, d_ingest, &stats, d_encode, total.elapsed());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn report(
    input: &Path,
    output: &Path,
    in_size: Size,
    out_size: Size,
    decode: Duration,
    ingest: Duration,
    stats: &RunStats,
    encode: Duration,
    total: Duration,
) {
    let name = |p: &Path| p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    eprintln!(
        "craws: {} {}x{} -> {} {}x{}",
        name(input), in_size.width, in_size.height, name(output), out_size.width, out_size.height,
    );
    eprintln!("  {:<22} {}", "decode", fmt_ms(decode));
    eprintln!("  {:<22} {}   (sRGB -> linear f32 tiles)", "ingest", fmt_ms(ingest));
    for (i, n) in stats.nodes.iter().enumerate() {
        eprintln!(
            "  {:<22} {}   {} tiles, {} computed",
            format!("{} {}", i + 1, n.name), fmt_ms(n.duration), n.tiles, n.computed,
        );
    }
    eprintln!("  {:<22} {}   (linear -> sRGB, encode, write)", "encode", fmt_ms(encode));
    eprintln!("  {:<22} {}", "total", fmt_ms(total));
}

fn fmt_ms(d: Duration) -> String {
    format!("{:>9.1} ms", d.as_secs_f64() * 1000.0)
}

const OPS_HELP: &str = r#"pipeline.json: { "version": 0, "steps": [ <op>, ... ] }

ops:
  resize      { "op": "resize", "width": 1600, "height": null, "filter": "lanczos3" }
              at least one of width/height; the other preserves aspect ratio
              filters: nearest | bilinear | catmull_rom | lanczos3 (default)
  crop        { "op": "crop", "x": 0, "y": 0, "width": 800, "height": 600 }
              rect must lie fully inside the image
  exposure    { "op": "exposure", "stops": 0.5 }
              photographic stops, linear-light multiply by 2^stops
  grayscale   { "op": "grayscale" }
              Rec.709 relative luminance, computed in linear light
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }
}
