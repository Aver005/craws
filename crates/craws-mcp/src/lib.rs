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

pub mod session;

pub use session::{ImageRef, Session, SessionError};

use craws_domain::{Filter, OpSpec, Rgba8};
use craws_engine::compose::CollageOptions;
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
}

const DEFAULT_STROKE: f32 = 3.0;
const DEFAULT_HEAD: f32 = 18.0;

fn parse_color_opt(s: Option<&str>) -> Result<Option<Rgba8>, McpError> {
    s.map(parse_color).transpose()
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
        | SessionError::EmptyCollage => McpError::invalid_params(e.to_string(), None),
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
}
