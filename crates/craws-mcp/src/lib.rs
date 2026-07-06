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

use craws_domain::{Filter, OpSpec};
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
        | SessionError::UnknownFormat(_) => McpError::invalid_params(e.to_string(), None),
        // environment / decode failures → internal_error
        SessionError::Read { .. }
        | SessionError::Write { .. }
        | SessionError::Codec(_) => McpError::internal_error(e.to_string(), None),
    }
}
