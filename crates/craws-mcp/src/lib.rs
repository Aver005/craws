//! Craws MCP server — port #2 over `craws-engine`, on the official `rmcp` SDK.
//!
//! Exposes the engine as agent tools over stdio (newline-delimited JSON-RPC,
//! protocol `2024-11-05`). An agent opens images, applies operations (each
//! producing a new immutable handle) and exports results — the same automation
//! the CLI does, but driven conversationally.
//!
//! Contract notes (verified against the intended client, pooprusteek):
//! - stdout is the protocol channel — nothing else may be written there;
//!   all logging goes to stderr.
//! - tool results are returned as **text** content (image parts would be
//!   dropped by the client), carrying a compact JSON line so the model can
//!   read a result and feed its `image_id` into the next call.
//! - tool/server names avoid `__` (the client splits `mcp__server__tool` on it).

pub mod fonts;
pub mod session;

pub use session::{ImageRef, Session, SessionError};

use craws_domain::{AlignX, AlignY, Filter, FlipAxis, OpSpec, Pipeline, RedactMode, Rgba8};
use craws_engine::compose::{CollageOptions, DiffView};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

pub const SERVER_NAME: &str = "craws";

#[derive(Clone)]
pub struct Craws {
    session: Arc<Session>,
    tool_router: ToolRouter<Craws>,
}

impl Default for Craws {
    fn default() -> Self {
        Self::new()
    }
}

// ── tool parameter schemas ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenParams {
    /// Filesystem path to a .png / .jpg / .webp image.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResizeParams {
    /// Handle returned by a previous step (e.g. "img-1").
    pub image_id: String,
    /// Target width in pixels. Omit to derive from height, preserving aspect.
    #[serde(default)]
    pub width: Option<u32>,
    /// Target height in pixels. Omit to derive from width, preserving aspect.
    #[serde(default)]
    pub height: Option<u32>,
    /// nearest | bilinear | catmull_rom | lanczos3 (default lanczos3).
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CropParams {
    pub image_id: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExposureParams {
    pub image_id: String,
    /// Exposure in photographic stops (linear multiply by 2^stops).
    pub stops: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageIdParams {
    pub image_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    pub image_id: String,
    /// Output path; the format is chosen from the extension (.png/.jpg/.webp).
    pub path: String,
    /// JPEG quality 1–100 (ignored for png/webp).
    #[serde(default)]
    pub quality: Option<u8>,
}

/// Colors are hex (`#RGB`, `#RRGGBB`, `#RRGGBBAA`) or a common name
/// (red, green, blue, yellow, orange, white, black, gray, cyan, magenta, transparent).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrawRectParams {
    pub image_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Corner radius in pixels (0 = sharp corners).
    #[serde(default)]
    pub corner_radius: f32,
    /// Interior color. Omit for outline-only.
    #[serde(default)]
    pub fill: Option<String>,
    /// Outline color. Omit for fill-only.
    #[serde(default)]
    pub stroke: Option<String>,
    /// Outline width in pixels (default 3).
    #[serde(default)]
    pub stroke_width: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrawEllipseParams {
    pub image_id: String,
    /// Bounding box of the ellipse. Equal width/height draws a circle.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default)]
    pub stroke_width: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrawLineParams {
    pub image_id: String,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub color: String,
    /// Line width in pixels (default 3).
    #[serde(default)]
    pub thickness: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrawArrowParams {
    pub image_id: String,
    /// Tail (x1,y1) → head/tip (x2,y2).
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub color: String,
    /// Line width in pixels (default 3).
    #[serde(default)]
    pub thickness: Option<f32>,
    /// Arrowhead length in pixels (default 18).
    #[serde(default)]
    pub head_length: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrawTextParams {
    pub image_id: String,
    /// Anchor point; how the text sits on it is set by align_x/align_y.
    pub x: f32,
    pub y: f32,
    /// The text. `\n` starts a new line.
    pub text: String,
    /// Hex (#RRGGBB[AA]) or a color name.
    pub color: String,
    /// Pixel height (default 24).
    #[serde(default)]
    pub font_size: Option<f32>,
    /// Font family name (resolved from installed fonts) or a .ttf/.otf path.
    /// Omitted → the built-in default font.
    #[serde(default)]
    pub font: Option<String>,
    /// left | center | right (default left) — horizontal anchoring at x.
    #[serde(default)]
    pub align_x: Option<String>,
    /// top | middle | bottom | baseline (default baseline) — vertical anchoring at y.
    #[serde(default)]
    pub align_y: Option<String>,
    /// Baseline-to-baseline distance for multi-line text (default: font's natural height).
    #[serde(default)]
    pub line_height: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OverlayParams {
    /// The image drawn onto; the result keeps its size.
    pub base_id: String,
    /// The image pasted on top.
    pub top_id: String,
    /// Top-left position of `top` within `base` (may be negative to crop).
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    /// 0..1 (default 1 = fully opaque).
    #[serde(default)]
    pub opacity: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CollageParams {
    /// Images to arrange, in order.
    pub image_ids: Vec<String>,
    /// Canvas width in pixels (default 1600).
    #[serde(default)]
    pub target_width: Option<u32>,
    /// Nominal row height before justification (default 320).
    #[serde(default)]
    pub row_height: Option<u32>,
    /// Gap between cells and around the edges (default 12).
    #[serde(default)]
    pub gap: Option<u32>,
    /// Background color (default white).
    #[serde(default)]
    pub background: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RotateParams {
    pub image_id: String,
    /// Clockwise rotation in degrees. Multiples of 90 are lossless; other angles resample.
    pub degrees: f32,
    /// Grow the canvas to fit the rotated image (default true). False keeps the
    /// original size and clips the rotated corners.
    #[serde(default)]
    pub expand: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FlipParams {
    pub image_id: String,
    /// `horizontal` (mirror left↔right) or `vertical` (top↔bottom).
    pub axis: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PadParams {
    pub image_id: String,
    /// Padding applied to every side left unset — convenient uniform margin.
    #[serde(default)]
    pub all: Option<u32>,
    #[serde(default)]
    pub left: Option<u32>,
    #[serde(default)]
    pub right: Option<u32>,
    #[serde(default)]
    pub top: Option<u32>,
    #[serde(default)]
    pub bottom: Option<u32>,
    /// Border fill color (hex or name). Default transparent.
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TrimParams {
    pub image_id: String,
    /// Background color to remove (hex or name). Omit to auto-detect from the
    /// top-left pixel — trims solid or transparent margins alike.
    #[serde(default)]
    pub color: Option<String>,
    /// Match tolerance 0..1 in linear light (default 0.01) to absorb JPEG/AA noise.
    #[serde(default)]
    pub tolerance: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HueRotateParams {
    pub image_id: String,
    /// Hue rotation in degrees (0..360); luminance is preserved.
    pub degrees: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlurParams {
    pub image_id: String,
    /// Blur strength — the Gaussian sigma in pixels (0 = no-op).
    pub radius: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RedactParams {
    pub image_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// pixelate (default) | blur | fill.
    #[serde(default)]
    pub mode: Option<String>,
    /// Mosaic cell size for `pixelate` (default 12).
    #[serde(default)]
    pub block: Option<u32>,
    /// Blur sigma for `blur` (default 12).
    #[serde(default)]
    pub radius: Option<f32>,
    /// Bar color for `fill` (hex/name, default black).
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotlightParams {
    pub image_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Rounded window corners (default 0 = sharp).
    #[serde(default)]
    pub corner_radius: Option<f32>,
    /// Veil opacity outside the window, 0..1 (default 0.55).
    #[serde(default)]
    pub dim: Option<f32>,
    /// Veil color (hex/name, default black).
    #[serde(default)]
    pub color: Option<String>,
    /// Soft edge width in pixels (default 0 = crisp).
    #[serde(default)]
    pub feather: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BeautifyParams {
    pub image_id: String,
    /// Background margin around the image in pixels (default 64).
    #[serde(default)]
    pub padding: Option<u32>,
    /// Image corner rounding (default 16).
    #[serde(default)]
    pub corner_radius: Option<f32>,
    /// Drop-shadow softness (blur sigma, default 24).
    #[serde(default)]
    pub shadow_radius: Option<f32>,
    /// Drop-shadow opacity 0..1 (default 0.35).
    #[serde(default)]
    pub shadow_opacity: Option<f32>,
    /// Drop-shadow vertical offset in pixels (default 12).
    #[serde(default)]
    pub shadow_offset: Option<f32>,
    /// Frame background color (hex/name, default transparent).
    #[serde(default)]
    pub background: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffParams {
    /// Baseline image.
    pub a_id: String,
    /// Image to compare against the baseline.
    pub b_id: String,
    /// heatmap (default) | difference | side_by_side.
    #[serde(default)]
    pub view: Option<String>,
    /// Per-channel change threshold 0..1 for the metric (default 0 = any change).
    #[serde(default)]
    pub threshold: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunPipelineParams {
    pub image_id: String,
    /// A JSON pipeline: a bare array of op objects, or `{ "version": 0, "steps": [...] }`.
    /// Ops match the pipeline format (e.g. {"op":"resize","width":800}).
    pub pipeline: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrightnessContrastParams {
    pub image_id: String,
    /// Additive brightness in sRGB, ~ -1..1 (default 0).
    #[serde(default)]
    pub brightness: Option<f32>,
    /// Contrast around mid-gray, ~ -1..1 (default 0).
    #[serde(default)]
    pub contrast: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaturationParams {
    pub image_id: String,
    /// 1 = identity, 0 = grayscale, >1 boosts.
    pub amount: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LevelsParams {
    pub image_id: String,
    #[serde(default)]
    pub in_black: Option<f32>,
    #[serde(default)]
    pub in_white: Option<f32>,
    #[serde(default)]
    pub gamma: Option<f32>,
    #[serde(default)]
    pub out_black: Option<f32>,
    #[serde(default)]
    pub out_white: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CurvesParams {
    pub image_id: String,
    /// Control points `[x, y]` in 0..1, sorted by x. e.g. `[[0,0],[0.25,0.15],[1,1]]` (an S-curve).
    pub points: Vec<[f32; 2]>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WhiteBalanceParams {
    pub image_id: String,
    /// Blue↔amber, ~ -1..1 (default 0). Positive = warmer.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Green↔magenta, ~ -1..1 (default 0).
    #[serde(default)]
    pub tint: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GradientMapParams {
    pub image_id: String,
    /// Color for dark tones (hex/name).
    pub low: String,
    /// Color for bright tones (hex/name).
    pub high: String,
    /// Optional midtone color (hex/name) for a 3-stop gradient.
    #[serde(default)]
    pub mid: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SharpenParams {
    pub image_id: String,
    /// Strength (default 1).
    #[serde(default)]
    pub amount: Option<f32>,
    /// Blur radius/sigma for the unsharp mask (default 2).
    #[serde(default)]
    pub radius: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VignetteParams {
    pub image_id: String,
    /// Darkening strength 0..1 (default 0.5).
    #[serde(default)]
    pub amount: Option<f32>,
    /// Softness 0..1 (default 0.5); higher starts the falloff closer to center.
    #[serde(default)]
    pub feather: Option<f32>,
    /// Color to darken toward (hex/name, default black).
    #[serde(default)]
    pub color: Option<String>,
}

// ── tools ───────────────────────────────────────────────────────────────────

#[tool_router]
impl Craws {
    pub fn new() -> Self {
        Self { session: Arc::new(Session::new()), tool_router: Self::tool_router() }
    }

    #[tool(description = "Open an image file (png/jpg/webp) and return a handle (image_id) plus its dimensions.")]
    fn open_image(&self, Parameters(p): Parameters<OpenParams>) -> Result<CallToolResult, McpError> {
        self.session.open_path(Path::new(&p.path)).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "Resize an image. Give width and/or height; a missing dimension preserves aspect ratio. Returns a new handle.")]
    fn resize(&self, Parameters(p): Parameters<ResizeParams>) -> Result<CallToolResult, McpError> {
        let filter = parse_filter(p.filter.as_deref())?;
        self.session
            .apply(&p.image_id, OpSpec::Resize { width: p.width, height: p.height, filter })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Crop an image to a rectangle (x, y, width, height) that must lie fully inside it. Returns a new handle.")]
    fn crop(&self, Parameters(p): Parameters<CropParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::Crop { x: p.x, y: p.y, width: p.width, height: p.height })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Adjust exposure by a number of photographic stops (positive brightens, negative darkens). Returns a new handle.")]
    fn exposure(&self, Parameters(p): Parameters<ExposureParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::Exposure { stops: p.stops })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Convert an image to grayscale (Rec.709 luminance). Returns a new handle.")]
    fn grayscale(&self, Parameters(p): Parameters<ImageIdParams>) -> Result<CallToolResult, McpError> {
        self.session.apply(&p.image_id, OpSpec::Grayscale).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "Report the current width and height of an image handle without modifying it.")]
    fn image_info(&self, Parameters(p): Parameters<ImageIdParams>) -> Result<CallToolResult, McpError> {
        self.session.info(&p.image_id).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "Write an image handle to disk. The format follows the file extension (.png/.jpg/.webp).")]
    fn export(&self, Parameters(p): Parameters<ExportParams>) -> Result<CallToolResult, McpError> {
        let path = Path::new(&p.path);
        let bytes = self.session.export_path(&p.image_id, path, p.quality).map_err(to_mcp)?;
        let json = serde_json::json!({ "path": p.path, "bytes": bytes });
        Ok(CallToolResult::success(vec![ContentBlock::text(json.to_string())]))
    }

    // ── annotation ──
    #[tool(description = "Draw a rectangle (optionally rounded) with an optional fill and/or outline. \
                          Give at least one of fill/stroke. Colors: hex (#RRGGBB[AA]) or a name. Returns a new handle.")]
    fn draw_rect(&self, Parameters(p): Parameters<DrawRectParams>) -> Result<CallToolResult, McpError> {
        let fill = parse_color_opt(p.fill.as_deref())?;
        let stroke = parse_color_opt(p.stroke.as_deref())?;
        self.session
            .apply(
                &p.image_id,
                OpSpec::DrawRect {
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    corner_radius: p.corner_radius,
                    fill,
                    stroke,
                    stroke_width: p.stroke_width.unwrap_or(DEFAULT_STROKE),
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Draw an ellipse (circle when width==height) inside the given box, with optional fill and/or outline. \
                          Great for circling a region. Returns a new handle.")]
    fn draw_ellipse(&self, Parameters(p): Parameters<DrawEllipseParams>) -> Result<CallToolResult, McpError> {
        let fill = parse_color_opt(p.fill.as_deref())?;
        let stroke = parse_color_opt(p.stroke.as_deref())?;
        self.session
            .apply(
                &p.image_id,
                OpSpec::DrawEllipse {
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    fill,
                    stroke,
                    stroke_width: p.stroke_width.unwrap_or(DEFAULT_STROKE),
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Draw a straight line between two points. Returns a new handle.")]
    fn draw_line(&self, Parameters(p): Parameters<DrawLineParams>) -> Result<CallToolResult, McpError> {
        let color = parse_color(&p.color)?;
        self.session
            .apply(
                &p.image_id,
                OpSpec::DrawLine { x1: p.x1, y1: p.y1, x2: p.x2, y2: p.y2, color, thickness: p.thickness.unwrap_or(DEFAULT_STROKE) },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Draw an arrow pointing from (x1,y1) to the tip (x2,y2) — the key annotation for pointing at things. Returns a new handle.")]
    fn draw_arrow(&self, Parameters(p): Parameters<DrawArrowParams>) -> Result<CallToolResult, McpError> {
        let color = parse_color(&p.color)?;
        self.session
            .apply(
                &p.image_id,
                OpSpec::DrawArrow {
                    x1: p.x1,
                    y1: p.y1,
                    x2: p.x2,
                    y2: p.y2,
                    color,
                    thickness: p.thickness.unwrap_or(DEFAULT_STROKE),
                    head_length: p.head_length.unwrap_or(DEFAULT_HEAD),
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Draw a text label (font size, font by name or path, align x/y, multi-line via \\n). \
                          Use it to caption annotations. Returns a new handle.")]
    fn draw_text(&self, Parameters(p): Parameters<DrawTextParams>) -> Result<CallToolResult, McpError> {
        let color = parse_color(&p.color)?;
        let font = fonts::resolve_font_path(p.font.as_deref());
        self.session
            .apply(
                &p.image_id,
                OpSpec::DrawText {
                    x: p.x,
                    y: p.y,
                    text: p.text,
                    color,
                    font_size: p.font_size.unwrap_or(24.0),
                    font,
                    align_x: parse_align_x(p.align_x.as_deref())?,
                    align_y: parse_align_y(p.align_y.as_deref())?,
                    line_height: p.line_height,
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    // ── composition ──
    #[tool(description = "Composite one image on top of another at (x,y) with optional opacity. \
                          Result has the base's size. Returns a new handle.")]
    fn overlay(&self, Parameters(p): Parameters<OverlayParams>) -> Result<CallToolResult, McpError> {
        self.session
            .overlay(&p.base_id, &p.top_id, p.x, p.y, p.opacity.unwrap_or(1.0))
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Arrange several images into a smart justified-rows collage (auto-laid-out by aspect ratio, each full row filling the width). Returns a new handle.")]
    fn collage(&self, Parameters(p): Parameters<CollageParams>) -> Result<CallToolResult, McpError> {
        let background = match p.background.as_deref() {
            Some(s) => parse_color(s)?,
            None => Rgba8::rgb(255, 255, 255),
        };
        let opts = CollageOptions {
            target_width: p.target_width.unwrap_or(1600).max(1),
            row_height: p.row_height.unwrap_or(320).max(1),
            gap: p.gap.unwrap_or(12),
            background,
        };
        self.session.collage(&p.image_ids, opts).map(ref_result).map_err(to_mcp)
    }

    // ── geometry ──
    #[tool(description = "Rotate an image clockwise by `degrees`. Multiples of 90 are lossless; other angles \
                          resample with transparent corners. `expand` (default true) grows the canvas to fit. Returns a new handle.")]
    fn rotate(&self, Parameters(p): Parameters<RotateParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::Rotate { degrees: p.degrees, expand: p.expand.unwrap_or(true) })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Mirror an image across an axis: axis = horizontal (left↔right) or vertical (top↔bottom). Returns a new handle.")]
    fn flip(&self, Parameters(p): Parameters<FlipParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::Flip { axis: parse_axis(&p.axis)? })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Extend the canvas with a margin — `all` for uniform padding, or per-side left/right/top/bottom. \
                          Default color transparent. Handy for breathing room before a shadow/background. Returns a new handle.")]
    fn pad(&self, Parameters(p): Parameters<PadParams>) -> Result<CallToolResult, McpError> {
        let a = p.all.unwrap_or(0);
        let color = match p.color.as_deref() {
            Some(s) => parse_color(s)?,
            None => Rgba8::new(0, 0, 0, 0),
        };
        self.session
            .apply(
                &p.image_id,
                OpSpec::Pad {
                    left: p.left.unwrap_or(a),
                    right: p.right.unwrap_or(a),
                    top: p.top.unwrap_or(a),
                    bottom: p.bottom.unwrap_or(a),
                    color,
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Auto-crop a uniform border (whitespace, a solid background, or transparency). `color` overrides \
                          the detected background; `tolerance` (0..1) absorbs JPEG/anti-aliasing noise. Returns a new handle.")]
    fn trim(&self, Parameters(p): Parameters<TrimParams>) -> Result<CallToolResult, McpError> {
        let color = parse_color_opt(p.color.as_deref())?;
        self.session
            .trim(&p.image_id, color, p.tolerance.unwrap_or(DEFAULT_TRIM_TOLERANCE))
            .map(ref_result)
            .map_err(to_mcp)
    }

    // ── color ──
    #[tool(description = "Rotate the hue of every pixel by `degrees` (0..360), preserving luminance. Returns a new handle.")]
    fn hue_rotate(&self, Parameters(p): Parameters<HueRotateParams>) -> Result<CallToolResult, McpError> {
        self.session.apply(&p.image_id, OpSpec::HueRotate { degrees: p.degrees }).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "Invert colors (photographic negative), computed in perceptual sRGB space. Returns a new handle.")]
    fn invert(&self, Parameters(p): Parameters<ImageIdParams>) -> Result<CallToolResult, McpError> {
        self.session.apply(&p.image_id, OpSpec::Invert).map(ref_result).map_err(to_mcp)
    }

    // ── filter ──
    #[tool(description = "Gaussian-blur an image; `radius` is the blur strength (Gaussian sigma in pixels). Returns a new handle.")]
    fn blur(&self, Parameters(p): Parameters<BlurParams>) -> Result<CallToolResult, McpError> {
        self.session.apply(&p.image_id, OpSpec::Blur { radius: p.radius }).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "Censor a rectangular region — the secret-hiding tool. mode = pixelate (default) | blur | fill; \
                          give `block` for pixelate, `radius` for blur, `color` for fill. Returns a new handle.")]
    fn redact(&self, Parameters(p): Parameters<RedactParams>) -> Result<CallToolResult, McpError> {
        let mode = parse_redact_mode(p.mode.as_deref(), p.block, p.radius, p.color.as_deref())?;
        self.session
            .apply(&p.image_id, OpSpec::Redact { x: p.x, y: p.y, width: p.width, height: p.height, mode })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Spotlight a region: dim everything outside the (optionally rounded, feathered) rectangle to draw \
                          the eye to it. `dim` is the veil strength 0..1 (default 0.55). Returns a new handle.")]
    fn spotlight(&self, Parameters(p): Parameters<SpotlightParams>) -> Result<CallToolResult, McpError> {
        let color = match p.color.as_deref() {
            Some(s) => parse_color(s)?,
            None => Rgba8::rgb(0, 0, 0),
        };
        self.session
            .apply(
                &p.image_id,
                OpSpec::Spotlight {
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    corner_radius: p.corner_radius.unwrap_or(0.0),
                    dim: p.dim.unwrap_or(0.55),
                    color,
                    feather: p.feather.unwrap_or(0.0),
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Polish a screenshot: round its corners, add a soft drop shadow, and frame it with padding of a \
                          background color. The one-shot \"make this look good\" tool. Returns a new handle.")]
    fn beautify(&self, Parameters(p): Parameters<BeautifyParams>) -> Result<CallToolResult, McpError> {
        let background = match p.background.as_deref() {
            Some(s) => parse_color(s)?,
            None => Rgba8::new(0, 0, 0, 0),
        };
        self.session
            .apply(
                &p.image_id,
                OpSpec::Beautify {
                    padding: p.padding.unwrap_or(64),
                    corner_radius: p.corner_radius.unwrap_or(16.0),
                    shadow_radius: p.shadow_radius.unwrap_or(24.0),
                    shadow_opacity: p.shadow_opacity.unwrap_or(0.35),
                    shadow_offset: p.shadow_offset.unwrap_or(12.0),
                    background,
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    // ── tonal / color grade ──
    #[tool(description = "Adjust brightness and/or contrast (perceptual sRGB, ~ -1..1 each). Returns a new handle.")]
    fn brightness_contrast(&self, Parameters(p): Parameters<BrightnessContrastParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::BrightnessContrast { brightness: p.brightness.unwrap_or(0.0), contrast: p.contrast.unwrap_or(0.0) })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Adjust saturation: amount 1 = identity, 0 = grayscale, >1 boosts. Returns a new handle.")]
    fn saturation(&self, Parameters(p): Parameters<SaturationParams>) -> Result<CallToolResult, McpError> {
        self.session.apply(&p.image_id, OpSpec::Saturation { amount: p.amount }).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "Levels remap: input black/white points, gamma, output black/white (0..1, in sRGB). \
                          Omitted fields default to identity. Returns a new handle.")]
    fn levels(&self, Parameters(p): Parameters<LevelsParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(
                &p.image_id,
                OpSpec::Levels {
                    in_black: p.in_black.unwrap_or(0.0),
                    in_white: p.in_white.unwrap_or(1.0),
                    gamma: p.gamma.unwrap_or(1.0),
                    out_black: p.out_black.unwrap_or(0.0),
                    out_white: p.out_white.unwrap_or(1.0),
                },
            )
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Apply a tone curve from control points [x,y] in 0..1 (e.g. an S-curve for punch). Returns a new handle.")]
    fn curves(&self, Parameters(p): Parameters<CurvesParams>) -> Result<CallToolResult, McpError> {
        self.session.apply(&p.image_id, OpSpec::Curves { points: p.points }).map(ref_result).map_err(to_mcp)
    }

    #[tool(description = "White balance via temperature (blue↔amber) and tint (green↔magenta), ~ -1..1. Returns a new handle.")]
    fn white_balance(&self, Parameters(p): Parameters<WhiteBalanceParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::WhiteBalance { temperature: p.temperature.unwrap_or(0.0), tint: p.tint.unwrap_or(0.0) })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Map luminance to a color gradient: low (dark) → optional mid → high (bright). \
                          Duotone / heatmap / sepia looks. Returns a new handle.")]
    fn gradient_map(&self, Parameters(p): Parameters<GradientMapParams>) -> Result<CallToolResult, McpError> {
        let low = parse_color(&p.low)?;
        let high = parse_color(&p.high)?;
        let mid = parse_color_opt(p.mid.as_deref())?;
        self.session.apply(&p.image_id, OpSpec::GradientMap { low, high, mid }).map(ref_result).map_err(to_mcp)
    }

    // ── more filters ──
    #[tool(description = "Sharpen via unsharp mask: `amount` (default 1) and `radius` (default 2). Returns a new handle.")]
    fn sharpen(&self, Parameters(p): Parameters<SharpenParams>) -> Result<CallToolResult, McpError> {
        self.session
            .apply(&p.image_id, OpSpec::Sharpen { amount: p.amount.unwrap_or(1.0), radius: p.radius.unwrap_or(2.0) })
            .map(ref_result)
            .map_err(to_mcp)
    }

    #[tool(description = "Vignette: radial darkening toward a color (default black). `amount` 0..1, `feather` 0..1. Returns a new handle.")]
    fn vignette(&self, Parameters(p): Parameters<VignetteParams>) -> Result<CallToolResult, McpError> {
        let color = match p.color.as_deref() {
            Some(s) => parse_color(s)?,
            None => Rgba8::rgb(0, 0, 0),
        };
        self.session
            .apply(&p.image_id, OpSpec::Vignette { amount: p.amount.unwrap_or(0.5), feather: p.feather.unwrap_or(0.5), color })
            .map(ref_result)
            .map_err(to_mcp)
    }

    // ── compare / meta ──
    #[tool(description = "Compare two images: view = heatmap (default) | difference | side_by_side. Returns a new handle \
                          PLUS a change metric (fraction_changed, max_difference) — gold for visual-regression checks.")]
    fn diff(&self, Parameters(p): Parameters<DiffParams>) -> Result<CallToolResult, McpError> {
        let view = parse_diff_view(p.view.as_deref())?;
        let r = self.session.diff(&p.a_id, &p.b_id, view, p.threshold.unwrap_or(0.0)).map_err(to_mcp)?;
        let json = serde_json::json!({
            "image_id": r.image.id,
            "width": r.image.width,
            "height": r.image.height,
            "fraction_changed": r.fraction,
            "max_difference": r.max,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(json.to_string())]))
    }

    #[tool(description = "Run a whole pipeline on an image in one call — `pipeline` is a JSON array of op objects (or \
                          {\"steps\":[...]}). Chains many edits at once for automation. Returns the final handle.")]
    fn run_pipeline(&self, Parameters(p): Parameters<RunPipelineParams>) -> Result<CallToolResult, McpError> {
        let pipeline = parse_pipeline(&p.pipeline)?;
        self.session.run_pipeline(&p.image_id, &pipeline).map(ref_result).map_err(to_mcp)
    }
}

const DEFAULT_STROKE: f32 = 3.0;
const DEFAULT_HEAD: f32 = 18.0;
const DEFAULT_TRIM_TOLERANCE: f32 = 0.01;

fn parse_color_opt(s: Option<&str>) -> Result<Option<Rgba8>, McpError> {
    s.map(parse_color).transpose()
}

fn parse_align_x(s: Option<&str>) -> Result<AlignX, McpError> {
    Ok(match s.map(str::trim) {
        None | Some("") | Some("left") => AlignX::Left,
        Some("center") => AlignX::Center,
        Some("right") => AlignX::Right,
        Some(other) => return Err(McpError::invalid_params(format!("align_x must be left|center|right, got `{other}`"), None)),
    })
}

fn parse_align_y(s: Option<&str>) -> Result<AlignY, McpError> {
    Ok(match s.map(str::trim) {
        None | Some("") | Some("baseline") => AlignY::Baseline,
        Some("top") => AlignY::Top,
        Some("middle") => AlignY::Middle,
        Some("bottom") => AlignY::Bottom,
        Some(other) => return Err(McpError::invalid_params(format!("align_y must be top|middle|bottom|baseline, got `{other}`"), None)),
    })
}

fn parse_axis(s: &str) -> Result<FlipAxis, McpError> {
    Ok(match s.trim().to_ascii_lowercase().as_str() {
        "horizontal" | "h" | "x" => FlipAxis::Horizontal,
        "vertical" | "v" | "y" => FlipAxis::Vertical,
        other => return Err(McpError::invalid_params(format!("axis must be horizontal|vertical, got `{other}`"), None)),
    })
}

fn parse_redact_mode(
    mode: Option<&str>,
    block: Option<u32>,
    radius: Option<f32>,
    color: Option<&str>,
) -> Result<RedactMode, McpError> {
    Ok(match mode.unwrap_or("pixelate").trim().to_ascii_lowercase().as_str() {
        "" | "pixelate" | "mosaic" => RedactMode::Pixelate { block: block.unwrap_or(12).max(1) },
        "blur" => RedactMode::Blur { radius: radius.unwrap_or(12.0) },
        "fill" | "solid" | "bar" => RedactMode::Fill {
            color: match color {
                Some(s) => parse_color(s)?,
                None => Rgba8::rgb(0, 0, 0),
            },
        },
        other => return Err(McpError::invalid_params(format!("redact mode must be pixelate|blur|fill, got `{other}`"), None)),
    })
}

fn parse_diff_view(s: Option<&str>) -> Result<DiffView, McpError> {
    Ok(match s.map(str::trim) {
        None | Some("") | Some("heatmap") => DiffView::Heatmap,
        Some("difference") | Some("diff") => DiffView::Difference,
        Some("side_by_side") | Some("side-by-side") | Some("sxs") => DiffView::SideBySide,
        Some(other) => return Err(McpError::invalid_params(format!("view must be heatmap|difference|side_by_side, got `{other}`"), None)),
    })
}

/// Parse a pipeline given either as a bare array of ops or a `{version, steps}` object.
fn parse_pipeline(s: &str) -> Result<Pipeline, McpError> {
    let t = s.trim();
    if t.starts_with('[') {
        let steps: Vec<OpSpec> = serde_json::from_str(t)
            .map_err(|e| McpError::invalid_params(format!("bad pipeline steps: {e}"), None))?;
        Ok(Pipeline { version: Pipeline::CURRENT_VERSION, steps })
    } else {
        serde_json::from_str(t).map_err(|e| McpError::invalid_params(format!("bad pipeline JSON: {e}"), None))
    }
}

/// Parse `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, or a common color name.
fn parse_color(s: &str) -> Result<Rgba8, McpError> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        let bytes = hex.as_bytes();
        let hx = |i: usize| -> Option<u8> {
            (bytes[i] as char).to_digit(16).map(|v| v as u8)
        };
        let dup = |v: u8| v * 17; // 0xF → 0xFF
        let ok = |r, g, b, a| Ok(Rgba8::new(r, g, b, a));
        return match hex.len() {
            3 => match (hx(0), hx(1), hx(2)) {
                (Some(r), Some(g), Some(b)) => ok(dup(r), dup(g), dup(b), 255),
                _ => Err(bad_color(s)),
            },
            4 => match (hx(0), hx(1), hx(2), hx(3)) {
                (Some(r), Some(g), Some(b), Some(a)) => ok(dup(r), dup(g), dup(b), dup(a)),
                _ => Err(bad_color(s)),
            },
            6 | 8 => {
                let byte = |i: usize| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok();
                let a = if hex.len() == 8 { byte(3) } else { Some(255) };
                match (byte(0), byte(1), byte(2), a) {
                    (Some(r), Some(g), Some(b), Some(a)) => ok(r, g, b, a),
                    _ => Err(bad_color(s)),
                }
            }
            _ => Err(bad_color(s)),
        };
    }
    let c = match t.to_ascii_lowercase().as_str() {
        "red" => Rgba8::rgb(255, 0, 0),
        "green" => Rgba8::rgb(0, 176, 0),
        "blue" => Rgba8::rgb(0, 90, 255),
        "yellow" => Rgba8::rgb(255, 210, 0),
        "orange" => Rgba8::rgb(255, 140, 0),
        "white" => Rgba8::rgb(255, 255, 255),
        "black" => Rgba8::rgb(0, 0, 0),
        "gray" | "grey" => Rgba8::rgb(128, 128, 128),
        "cyan" => Rgba8::rgb(0, 200, 220),
        "magenta" => Rgba8::rgb(230, 0, 200),
        "transparent" => Rgba8::new(0, 0, 0, 0),
        _ => return Err(bad_color(s)),
    };
    Ok(c)
}

fn bad_color(s: &str) -> McpError {
    McpError::invalid_params(
        format!("unrecognized color `{s}` (use #RRGGBB[AA], #RGB, or a name like red/blue/yellow)"),
        None,
    )
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Craws {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Craws image engine. Workflow: open_image → (resize / crop / exposure / grayscale, \
                 chaining each step's returned image_id) → export. Images are immutable: every \
                 operation returns a NEW image_id, so you can branch and keep intermediates.",
            )
    }
}

/// Compact JSON text result carrying the handle for the next call.
fn ref_result(r: ImageRef) -> CallToolResult {
    let json = serde_json::json!({ "image_id": r.id, "width": r.width, "height": r.height });
    CallToolResult::success(vec![ContentBlock::text(json.to_string())])
}

fn parse_filter(s: Option<&str>) -> Result<Filter, McpError> {
    Ok(match s {
        None | Some("lanczos3") => Filter::Lanczos3,
        Some("nearest") => Filter::Nearest,
        Some("bilinear") => Filter::Bilinear,
        Some("catmull_rom") => Filter::CatmullRom,
        Some(other) => {
            return Err(McpError::invalid_params(
                format!("unknown filter `{other}` (use nearest|bilinear|catmull_rom|lanczos3)"),
                None,
            ));
        }
    })
}

fn to_mcp(e: SessionError) -> McpError {
    match e {
        // caller's fault → invalid_params (handle typo, out-of-bounds crop, bad format)
        SessionError::NotFound(_)
        | SessionError::Pipeline(_)
        | SessionError::UnknownFormat(_)
        | SessionError::EmptyCollage
        | SessionError::NothingToTrim
        | SessionError::SizeMismatch => McpError::invalid_params(e.to_string(), None),
        // environment / decode failures → internal_error
        SessionError::Read { .. }
        | SessionError::Write { .. }
        | SessionError::Codec(_) => McpError::internal_error(e.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_and_names() {
        assert_eq!(parse_color("#ff0000").unwrap(), Rgba8::new(255, 0, 0, 255));
        assert_eq!(parse_color("#f00").unwrap(), Rgba8::new(255, 0, 0, 255), "short hex expands");
        assert_eq!(parse_color("#00ff0080").unwrap(), Rgba8::new(0, 255, 0, 128), "8-digit carries alpha");
        assert_eq!(parse_color("  red ").unwrap(), Rgba8::rgb(255, 0, 0), "names, trimmed");
        assert_eq!(parse_color("transparent").unwrap().a, 0);
        assert!(parse_color("#zz").is_err());
        assert!(parse_color("chartreuse").is_err());
    }

    #[test]
    fn optional_color_passthrough() {
        assert!(parse_color_opt(None).unwrap().is_none());
        assert!(parse_color_opt(Some("blue")).unwrap().is_some());
        assert!(parse_color_opt(Some("nope")).is_err());
    }

    #[test]
    fn parses_flip_axis() {
        assert_eq!(parse_axis("horizontal").unwrap(), FlipAxis::Horizontal);
        assert_eq!(parse_axis(" Vertical ").unwrap(), FlipAxis::Vertical);
        assert_eq!(parse_axis("x").unwrap(), FlipAxis::Horizontal);
        assert_eq!(parse_axis("y").unwrap(), FlipAxis::Vertical);
        assert!(parse_axis("diagonal").is_err());
    }

    #[test]
    fn parses_redact_mode() {
        assert_eq!(parse_redact_mode(None, None, None, None).unwrap(), RedactMode::Pixelate { block: 12 });
        assert_eq!(parse_redact_mode(Some("pixelate"), Some(20), None, None).unwrap(), RedactMode::Pixelate { block: 20 });
        assert_eq!(parse_redact_mode(Some("blur"), None, Some(8.0), None).unwrap(), RedactMode::Blur { radius: 8.0 });
        assert_eq!(parse_redact_mode(Some("fill"), None, None, Some("red")).unwrap(), RedactMode::Fill { color: Rgba8::rgb(255, 0, 0) });
        assert_eq!(parse_redact_mode(Some("fill"), None, None, None).unwrap(), RedactMode::Fill { color: Rgba8::rgb(0, 0, 0) });
        assert!(parse_redact_mode(Some("scramble"), None, None, None).is_err());
        assert!(parse_redact_mode(Some("pixelate"), Some(0), None, None).unwrap() == RedactMode::Pixelate { block: 1 }, "block clamps to >=1");
    }

    #[test]
    fn parses_diff_view() {
        assert_eq!(parse_diff_view(None).unwrap(), DiffView::Heatmap);
        assert_eq!(parse_diff_view(Some("difference")).unwrap(), DiffView::Difference);
        assert_eq!(parse_diff_view(Some("side_by_side")).unwrap(), DiffView::SideBySide);
        assert!(parse_diff_view(Some("onion")).is_err());
    }

    #[test]
    fn parses_pipeline_array_and_object() {
        let a = parse_pipeline(r#"[{"op":"grayscale"},{"op":"blur","radius":2}]"#).unwrap();
        assert_eq!(a.steps.len(), 2);
        let o = parse_pipeline(r#"{"version":0,"steps":[{"op":"invert"}]}"#).unwrap();
        assert_eq!(o.steps, vec![OpSpec::Invert]);
        assert!(parse_pipeline("not json").is_err());
    }
}
