//! Pipeline spec: an ordered chain of operations over one image.
//!
//! This is *data*, not behavior — the engine interprets it. The JSON form is the
//! public automation contract (`craws run pipeline.json`), so changes here are
//! format changes: bump [`Pipeline::CURRENT_VERSION`] and keep old versions parsing.

use crate::color::Rgba8;
use crate::geometry::{Rect, Size};
use serde::{Deserialize, Serialize};

/// Resampling filter for [`OpSpec::Resize`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Nearest,
    Bilinear,
    CatmullRom,
    #[default]
    Lanczos3,
}

/// Horizontal anchoring of text at its `(x, y)` point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignX {
    #[default]
    Left,
    Center,
    Right,
}

/// Vertical anchoring of text at its `(x, y)` point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignY {
    Top,
    Middle,
    Bottom,
    /// `y` is the baseline of the first line (the typographic default).
    #[default]
    Baseline,
}

/// Mirror axis for [`OpSpec::Flip`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlipAxis {
    /// Mirror left↔right (columns reversed).
    Horizontal,
    /// Mirror top↔bottom (rows reversed).
    Vertical,
}

/// How [`OpSpec::Redact`] obscures its region.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedactMode {
    /// Mosaic: average `block`×`block` cells (the reversible-proof censor look).
    Pixelate {
        #[serde(default = "default_redact_block")]
        block: u32,
    },
    /// Gaussian-blur the region by `radius`.
    Blur { radius: f32 },
    /// Paint a solid `color` over the region (a black bar, by default).
    Fill { color: Rgba8 },
}

impl Default for RedactMode {
    fn default() -> Self {
        RedactMode::Pixelate { block: default_redact_block() }
    }
}

/// One step of a pipeline. Serialized with an `op` tag:
/// `{ "op": "resize", "width": 1600 }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpSpec {
    /// Resample to a new size. At least one of `width`/`height` is required;
    /// a missing dimension preserves aspect ratio (rounded, min 1px).
    Resize {
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default)]
        filter: Filter,
    },
    /// Keep only `rect` (must lie fully inside the current image).
    Crop { x: u32, y: u32, width: u32, height: u32 },
    /// Exposure in photographic stops: linear multiply by `2^stops`.
    Exposure { stops: f32 },
    /// Rec.709 relative-luminance grayscale (in linear light).
    Grayscale,

    // ── annotation ops (single-input; draw onto the image, size unchanged) ──
    /// Rectangle with optional fill, optional stroke, optional rounded corners.
    DrawRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        #[serde(default)]
        corner_radius: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Rgba8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Rgba8>,
        #[serde(default = "default_stroke_width")]
        stroke_width: f32,
    },
    /// Ellipse inscribed in the (x, y, width, height) box. Circle = equal w/h.
    DrawEllipse {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Rgba8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Rgba8>,
        #[serde(default = "default_stroke_width")]
        stroke_width: f32,
    },
    /// Straight line segment.
    DrawLine {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: Rgba8,
        #[serde(default = "default_thickness")]
        thickness: f32,
    },
    /// Arrow: a segment from (x1,y1) to (x2,y2) with a V-head at the second point.
    DrawArrow {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: Rgba8,
        #[serde(default = "default_thickness")]
        thickness: f32,
        #[serde(default = "default_head_length")]
        head_length: f32,
    },
    /// Text, anchored at `(x, y)` per `align_x`/`align_y`. `\n` starts a new line.
    /// `font` is an explicit font-file path (the engine stays deterministic); a
    /// missing/unreadable path falls back to the embedded default. Ports resolve
    /// font *names* to paths before building this op.
    DrawText {
        x: f32,
        y: f32,
        text: String,
        color: Rgba8,
        #[serde(default = "default_font_size")]
        font_size: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        font: Option<String>,
        #[serde(default)]
        align_x: AlignX,
        #[serde(default)]
        align_y: AlignY,
        /// Baseline-to-baseline distance; defaults to the font's natural line height.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line_height: Option<f32>,
    },

    // ── geometry ops (single-input; change the output size) ──
    /// Rotate about the image center by `degrees` (positive = clockwise).
    /// Multiples of 90° are exact pixel permutations; other angles resample
    /// (bilinear, in premultiplied linear light) with transparent corners.
    /// `expand` grows the canvas to contain the rotated image (default);
    /// `false` keeps the original size and clips the corners.
    Rotate {
        degrees: f32,
        #[serde(default = "default_true")]
        expand: bool,
    },
    /// Mirror across an axis. Size unchanged.
    Flip { axis: FlipAxis },
    /// Extend the canvas by a margin on each side, filling the new border with
    /// `color` (default transparent). Source pixels are placed at `(left, top)`.
    Pad {
        #[serde(default)]
        left: u32,
        #[serde(default)]
        right: u32,
        #[serde(default)]
        top: u32,
        #[serde(default)]
        bottom: u32,
        #[serde(default = "transparent")]
        color: Rgba8,
    },

    // ── color ops (single-input, pointwise; size unchanged) ──
    /// Rotate hue by `degrees` about the luma axis (luminance-preserving),
    /// evaluated in linear light. 0 / 360 = identity.
    HueRotate { degrees: f32 },
    /// Photographic negative — invert RGB in perceptual (sRGB) space. Alpha kept.
    Invert,

    // ── filter ops (single-input; size unchanged unless noted) ──
    /// Separable Gaussian blur; `radius` ≈ the Gaussian sigma in pixels (0 = no-op).
    /// Runs in premultiplied linear light, so edges don't fringe.
    Blur { radius: f32 },
    /// Obscure a rectangular region — pixelate, blur, or solid-fill it. The
    /// signature "censor a secret in a screenshot" tool.
    Redact {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        #[serde(default)]
        mode: RedactMode,
    },
    /// Draw the eye to a region by dimming everything outside it. `dim` is the
    /// veil opacity (0..1) of `color`; the window can be rounded and feathered.
    Spotlight {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        #[serde(default)]
        corner_radius: f32,
        #[serde(default = "default_dim")]
        dim: f32,
        #[serde(default = "black")]
        color: Rgba8,
        #[serde(default)]
        feather: f32,
    },
    /// "Polish" a screenshot: round its corners, drop a soft shadow, and frame it
    /// with `padding` of `background`. Output grows by `2·padding` per dimension.
    Beautify {
        #[serde(default = "default_beautify_padding")]
        padding: u32,
        #[serde(default = "default_corner_radius")]
        corner_radius: f32,
        #[serde(default = "default_shadow_radius")]
        shadow_radius: f32,
        #[serde(default = "default_shadow_opacity")]
        shadow_opacity: f32,
        #[serde(default = "default_shadow_offset")]
        shadow_offset: f32,
        #[serde(default = "transparent")]
        background: Rgba8,
    },

    // ── tonal / color-grade ops (single-input, pointwise; size unchanged) ──
    /// Additive `brightness` and `contrast` (S-curve around mid-gray), in sRGB. 0 = identity.
    BrightnessContrast {
        #[serde(default)]
        brightness: f32,
        #[serde(default)]
        contrast: f32,
    },
    /// Saturation: 1 = identity, 0 = grayscale, >1 boosts. In sRGB (Rec.601 luma).
    Saturation { amount: f32 },
    /// Levels remap (per RGB, in sRGB): input black/white points, gamma, output black/white.
    Levels {
        #[serde(default)]
        in_black: f32,
        #[serde(default = "one")]
        in_white: f32,
        #[serde(default = "one")]
        gamma: f32,
        #[serde(default)]
        out_black: f32,
        #[serde(default = "one")]
        out_white: f32,
    },
    /// Tone curve from control points `[x, y]` in 0..1 (per channel, in sRGB).
    Curves { points: Vec<[f32; 2]> },
    /// White balance as linear per-channel gains from `temperature` (blue↔amber)
    /// and `tint` (green↔magenta). 0 / 0 = identity.
    WhiteBalance {
        #[serde(default)]
        temperature: f32,
        #[serde(default)]
        tint: f32,
    },
    /// Map luminance to a gradient: `low` (dark) → optional `mid` → `high` (bright).
    /// Duotone when `mid` is absent. In sRGB.
    GradientMap {
        low: Rgba8,
        high: Rgba8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mid: Option<Rgba8>,
    },

    // ── more filters (single-input; size unchanged) ──
    /// Unsharp mask: `in + amount·(in − blur(in))`, in linear light. `radius` is the blur sigma.
    Sharpen {
        #[serde(default = "one")]
        amount: f32,
        #[serde(default = "default_sharpen_radius")]
        radius: f32,
    },
    /// Radial darkening toward `color` (default black): `amount` strength (0..1),
    /// `feather` softness (0..1).
    Vignette {
        #[serde(default = "default_half")]
        amount: f32,
        #[serde(default = "default_half")]
        feather: f32,
        #[serde(default = "black")]
        color: Rgba8,
    },
}

fn one() -> f32 {
    1.0
}
fn default_half() -> f32 {
    0.5
}
fn default_sharpen_radius() -> f32 {
    2.0
}
fn default_stroke_width() -> f32 {
    3.0
}
fn default_thickness() -> f32 {
    3.0
}
fn default_head_length() -> f32 {
    18.0
}
fn default_font_size() -> f32 {
    24.0
}
fn default_true() -> bool {
    true
}
fn transparent() -> Rgba8 {
    Rgba8::new(0, 0, 0, 0)
}
fn black() -> Rgba8 {
    Rgba8::new(0, 0, 0, 255)
}
fn default_redact_block() -> u32 {
    12
}
fn default_dim() -> f32 {
    0.55
}
fn default_beautify_padding() -> u32 {
    64
}
fn default_corner_radius() -> f32 {
    16.0
}
fn default_shadow_radius() -> f32 {
    24.0
}
fn default_shadow_opacity() -> f32 {
    0.35
}
fn default_shadow_offset() -> f32 {
    12.0
}

impl OpSpec {
    /// Stable machine name (matches the serde tag).
    pub fn name(&self) -> &'static str {
        match self {
            OpSpec::Resize { .. } => "resize",
            OpSpec::Crop { .. } => "crop",
            OpSpec::Exposure { .. } => "exposure",
            OpSpec::Grayscale => "grayscale",
            OpSpec::DrawRect { .. } => "draw_rect",
            OpSpec::DrawEllipse { .. } => "draw_ellipse",
            OpSpec::DrawLine { .. } => "draw_line",
            OpSpec::DrawArrow { .. } => "draw_arrow",
            OpSpec::DrawText { .. } => "draw_text",
            OpSpec::Rotate { .. } => "rotate",
            OpSpec::Flip { .. } => "flip",
            OpSpec::Pad { .. } => "pad",
            OpSpec::HueRotate { .. } => "hue_rotate",
            OpSpec::Invert => "invert",
            OpSpec::Blur { .. } => "blur",
            OpSpec::Redact { .. } => "redact",
            OpSpec::Spotlight { .. } => "spotlight",
            OpSpec::Beautify { .. } => "beautify",
            OpSpec::BrightnessContrast { .. } => "brightness_contrast",
            OpSpec::Saturation { .. } => "saturation",
            OpSpec::Levels { .. } => "levels",
            OpSpec::Curves { .. } => "curves",
            OpSpec::WhiteBalance { .. } => "white_balance",
            OpSpec::GradientMap { .. } => "gradient_map",
            OpSpec::Sharpen { .. } => "sharpen",
            OpSpec::Vignette { .. } => "vignette",
        }
    }

    /// Validate against the incoming image size; return the outgoing size.
    pub fn output_size(&self, input: Size) -> Result<Size, PipelineError> {
        debug_assert!(!input.is_empty(), "engine never feeds empty images");
        // `Curves` holds a non-Copy Vec — validate it by ref before the `match *self`.
        if let OpSpec::Curves { points } = self {
            for pt in points {
                if !pt[0].is_finite() || !pt[1].is_finite() {
                    return Err(PipelineError::NonFiniteParam { op: "curves", param: "points" });
                }
            }
            return Ok(input);
        }
        match *self {
            OpSpec::Resize { width, height, .. } => {
                let out = match (width, height) {
                    (None, None) => return Err(PipelineError::ResizeMissingDims),
                    (Some(w), Some(h)) => Size::new(w, h),
                    // Preserve aspect: round to nearest, never collapse to 0.
                    (Some(w), None) => Size::new(
                        w,
                        keep_aspect(w, input.height, input.width),
                    ),
                    (None, Some(h)) => Size::new(
                        keep_aspect(h, input.width, input.height),
                        h,
                    ),
                };
                if out.is_empty() {
                    return Err(PipelineError::ZeroSize { op: "resize" });
                }
                Ok(out)
            }
            OpSpec::Crop { x, y, width, height } => {
                let rect = Rect::new(x, y, width, height);
                if !rect.fits_in(input) {
                    return Err(PipelineError::CropOutOfBounds { rect, image: input });
                }
                Ok(rect.size())
            }
            OpSpec::Exposure { stops } => {
                if !stops.is_finite() {
                    return Err(PipelineError::NonFiniteParam { op: "exposure", param: "stops" });
                }
                Ok(input)
            }
            OpSpec::Grayscale => Ok(input),

            OpSpec::Rotate { degrees, expand } => {
                if !degrees.is_finite() {
                    return Err(PipelineError::NonFiniteParam { op: "rotate", param: "degrees" });
                }
                Ok(rotated_size(input, degrees, expand))
            }
            OpSpec::Flip { .. } => Ok(input),
            OpSpec::Pad { left, right, top, bottom, .. } => {
                let w = input.width as u64 + left as u64 + right as u64;
                let h = input.height as u64 + top as u64 + bottom as u64;
                if w > u32::MAX as u64 || h > u32::MAX as u64 {
                    return Err(PipelineError::ResultTooLarge { op: "pad" });
                }
                Ok(Size::new(w as u32, h as u32))
            }

            OpSpec::HueRotate { degrees } => {
                if !degrees.is_finite() {
                    return Err(PipelineError::NonFiniteParam { op: "hue_rotate", param: "degrees" });
                }
                Ok(input)
            }
            OpSpec::Invert => Ok(input),
            OpSpec::Blur { radius } => {
                if !radius.is_finite() {
                    return Err(PipelineError::NonFiniteParam { op: "blur", param: "radius" });
                }
                if radius < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "blur" });
                }
                Ok(input)
            }
            OpSpec::Redact { x, y, width, height, mode } => {
                check_finite("redact", &[x, y, width, height])?;
                if width < 0.0 || height < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "redact" });
                }
                match mode {
                    RedactMode::Pixelate { block: 0 } => {
                        return Err(PipelineError::NegativeParam { op: "redact" });
                    }
                    RedactMode::Blur { radius } if !radius.is_finite() || radius < 0.0 => {
                        return Err(PipelineError::NegativeParam { op: "redact" });
                    }
                    _ => {}
                }
                Ok(input)
            }
            OpSpec::Spotlight { x, y, width, height, corner_radius, dim, feather, .. } => {
                check_finite("spotlight", &[x, y, width, height, corner_radius, dim, feather])?;
                if width < 0.0 || height < 0.0 || corner_radius < 0.0 || feather < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "spotlight" });
                }
                Ok(input)
            }
            OpSpec::Beautify { padding, corner_radius, shadow_radius, shadow_opacity, shadow_offset, .. } => {
                check_finite("beautify", &[corner_radius, shadow_radius, shadow_opacity, shadow_offset])?;
                if corner_radius < 0.0 || shadow_radius < 0.0 || shadow_opacity < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "beautify" });
                }
                let w = input.width as u64 + 2 * padding as u64;
                let h = input.height as u64 + 2 * padding as u64;
                if w > u32::MAX as u64 || h > u32::MAX as u64 {
                    return Err(PipelineError::ResultTooLarge { op: "beautify" });
                }
                Ok(Size::new(w as u32, h as u32))
            }

            OpSpec::BrightnessContrast { brightness, contrast } => {
                check_finite("brightness_contrast", &[brightness, contrast])?;
                Ok(input)
            }
            OpSpec::Saturation { amount } => {
                if !amount.is_finite() {
                    return Err(PipelineError::NonFiniteParam { op: "saturation", param: "amount" });
                }
                Ok(input)
            }
            OpSpec::Levels { in_black, in_white, gamma, out_black, out_white } => {
                check_finite("levels", &[in_black, in_white, gamma, out_black, out_white])?;
                if gamma <= 0.0 {
                    return Err(PipelineError::NegativeParam { op: "levels" });
                }
                Ok(input)
            }
            OpSpec::Curves { .. } => Ok(input), // validated by ref above
            OpSpec::WhiteBalance { temperature, tint } => {
                check_finite("white_balance", &[temperature, tint])?;
                Ok(input)
            }
            OpSpec::GradientMap { .. } => Ok(input),
            OpSpec::Sharpen { amount, radius } => {
                check_finite("sharpen", &[amount, radius])?;
                if radius < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "sharpen" });
                }
                Ok(input)
            }
            OpSpec::Vignette { amount, feather, .. } => {
                check_finite("vignette", &[amount, feather])?;
                if amount < 0.0 || feather < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "vignette" });
                }
                Ok(input)
            }

            OpSpec::DrawRect { x, y, width, height, corner_radius, fill, stroke, stroke_width } => {
                check_finite("draw_rect", &[x, y, width, height, corner_radius, stroke_width])?;
                if width < 0.0 || height < 0.0 || corner_radius < 0.0 || stroke_width < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "draw_rect" });
                }
                if fill.is_none() && stroke.is_none() {
                    return Err(PipelineError::NothingToDraw { op: "draw_rect" });
                }
                Ok(input)
            }
            OpSpec::DrawEllipse { x, y, width, height, fill, stroke, stroke_width } => {
                check_finite("draw_ellipse", &[x, y, width, height, stroke_width])?;
                if width < 0.0 || height < 0.0 || stroke_width < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "draw_ellipse" });
                }
                if fill.is_none() && stroke.is_none() {
                    return Err(PipelineError::NothingToDraw { op: "draw_ellipse" });
                }
                Ok(input)
            }
            OpSpec::DrawLine { x1, y1, x2, y2, thickness, .. } => {
                check_finite("draw_line", &[x1, y1, x2, y2, thickness])?;
                if thickness < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "draw_line" });
                }
                Ok(input)
            }
            OpSpec::DrawArrow { x1, y1, x2, y2, thickness, head_length, .. } => {
                check_finite("draw_arrow", &[x1, y1, x2, y2, thickness, head_length])?;
                if thickness < 0.0 || head_length < 0.0 {
                    return Err(PipelineError::NegativeParam { op: "draw_arrow" });
                }
                Ok(input)
            }
            OpSpec::DrawText { x, y, font_size, line_height, .. } => {
                check_finite("draw_text", &[x, y, font_size])?;
                if line_height.is_some_and(|lh| !lh.is_finite()) {
                    return Err(PipelineError::NonFiniteParam { op: "draw_text", param: "line_height" });
                }
                if font_size <= 0.0 {
                    return Err(PipelineError::NegativeParam { op: "draw_text" });
                }
                Ok(input)
            }
        }
    }
}

/// Reject non-finite geometry before it reaches the rasterizer.
fn check_finite(op: &'static str, vals: &[f32]) -> Result<(), PipelineError> {
    if vals.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(PipelineError::NonFiniteParam { op, param: "geometry" })
    }
}

/// `known * num / den`, rounded to nearest, clamped to ≥ 1.
fn keep_aspect(known: u32, num: u32, den: u32) -> u32 {
    let v = (known as u64 * num as u64 + den as u64 / 2) / den as u64;
    (v.max(1)).min(u32::MAX as u64) as u32
}

/// Output size of a rotation. Multiples of 90° swap or keep dimensions exactly;
/// other angles either grow to the rotated bounding box (`expand`) or keep the
/// input size. Kept in sync with `craws_engine::ops::rotate`.
fn rotated_size(input: Size, degrees: f32, expand: bool) -> Size {
    let norm = degrees.rem_euclid(360.0);
    let q = (norm / 90.0).round();
    let is_ortho = (norm - q * 90.0).abs() < 1e-3;
    if is_ortho {
        let k = (q as i64).rem_euclid(4);
        return if k == 1 || k == 3 { Size::new(input.height, input.width) } else { input };
    }
    if !expand {
        return input;
    }
    let r = norm.to_radians();
    let (s, c) = (r.sin().abs(), r.cos().abs());
    let w = (input.width as f32 * c + input.height as f32 * s).ceil().max(1.0);
    let h = (input.width as f32 * s + input.height as f32 * c).ceil().max(1.0);
    Size::new(w as u32, h as u32)
}

/// An ordered, linear chain of operations. (The domain model is designed to grow
/// into a DAG later; v0 executes strictly top-to-bottom.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pipeline {
    #[serde(default)]
    pub version: u32,
    pub steps: Vec<OpSpec>,
}

impl Pipeline {
    pub const CURRENT_VERSION: u32 = 0;

    /// Validate every step against the flowing image size.
    /// Returns the size after each step (same length as `steps`).
    pub fn validate(&self, input: Size) -> Result<Vec<Size>, PipelineError> {
        if self.version > Self::CURRENT_VERSION {
            return Err(PipelineError::UnsupportedVersion(self.version));
        }
        if input.is_empty() {
            return Err(PipelineError::ZeroSize { op: "input" });
        }
        let mut cur = input;
        let mut sizes = Vec::with_capacity(self.steps.len());
        for (i, step) in self.steps.iter().enumerate() {
            cur = step
                .output_size(cur)
                .map_err(|e| PipelineError::AtStep { index: i, source: Box::new(e) })?;
            sizes.push(cur);
        }
        Ok(sizes)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PipelineError {
    #[error("pipeline version {0} is newer than this build supports")]
    UnsupportedVersion(u32),
    #[error("resize needs at least one of width/height")]
    ResizeMissingDims,
    #[error("{op}: result would have a zero dimension")]
    ZeroSize { op: &'static str },
    #[error("{op}: result dimensions exceed the u32 pixel limit")]
    ResultTooLarge { op: &'static str },
    #[error("crop {rect:?} does not fit inside image {image:?}")]
    CropOutOfBounds { rect: Rect, image: Size },
    #[error("{op}: parameter `{param}` must be finite")]
    NonFiniteParam { op: &'static str, param: &'static str },
    #[error("{op}: dimensions and widths must be non-negative")]
    NegativeParam { op: &'static str },
    #[error("{op}: needs a fill or a stroke (both are absent)")]
    NothingToDraw { op: &'static str },
    #[error("step {index}: {source}")]
    AtStep { index: usize, source: Box<PipelineError> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(w: u32, h: u32) -> Size {
        Size::new(w, h)
    }

    #[test]
    fn json_roundtrip_and_shape() {
        let p = Pipeline {
            version: 0,
            steps: vec![
                OpSpec::Resize { width: Some(1600), height: None, filter: Filter::default() },
                OpSpec::Exposure { stops: 0.5 },
                OpSpec::Crop { x: 1, y: 2, width: 3, height: 4 },
                OpSpec::Grayscale,
            ],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""op":"resize""#), "tagged form: {json}");
        let back: Pipeline = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn parses_handwritten_json() {
        let p: Pipeline = serde_json::from_str(
            r#"{ "steps": [ { "op": "resize", "height": 720, "filter": "bilinear" } ] }"#,
        )
        .unwrap();
        assert_eq!(p.version, 0, "version defaults to 0");
        assert_eq!(
            p.steps,
            vec![OpSpec::Resize { width: None, height: Some(720), filter: Filter::Bilinear }]
        );
    }

    #[test]
    fn resize_keeps_aspect() {
        let op = OpSpec::Resize { width: Some(1600), height: None, filter: Filter::default() };
        assert_eq!(op.output_size(px(3200, 2000)).unwrap(), px(1600, 1000));
        // rounding, and the ≥1 clamp on extreme ratios
        let op = OpSpec::Resize { width: Some(1), height: None, filter: Filter::default() };
        assert_eq!(op.output_size(px(10000, 100)).unwrap(), px(1, 1));
    }

    #[test]
    fn draw_ops_keep_size_and_validate() {
        use crate::color::Rgba8;
        let red = Rgba8::rgb(255, 0, 0);
        // draw ops never change the image size
        let ops = [
            OpSpec::DrawRect { x: 1.0, y: 1.0, width: 10.0, height: 8.0, corner_radius: 2.0, fill: None, stroke: Some(red), stroke_width: 3.0 },
            OpSpec::DrawEllipse { x: 0.0, y: 0.0, width: 20.0, height: 20.0, fill: Some(red), stroke: None, stroke_width: 3.0 },
            OpSpec::DrawLine { x1: 0.0, y1: 0.0, x2: 5.0, y2: 5.0, color: red, thickness: 2.0 },
            OpSpec::DrawArrow { x1: 0.0, y1: 0.0, x2: 9.0, y2: 0.0, color: red, thickness: 2.0, head_length: 6.0 },
        ];
        for op in ops {
            assert_eq!(op.output_size(px(100, 80)).unwrap(), px(100, 80), "{}", op.name());
        }

        // a rect with neither fill nor stroke is nothing to draw
        let empty = OpSpec::DrawRect { x: 0.0, y: 0.0, width: 5.0, height: 5.0, corner_radius: 0.0, fill: None, stroke: None, stroke_width: 3.0 };
        assert!(matches!(empty.output_size(px(10, 10)), Err(PipelineError::NothingToDraw { .. })));

        // non-finite geometry is rejected
        let nan = OpSpec::DrawLine { x1: f32::NAN, y1: 0.0, x2: 1.0, y2: 1.0, color: red, thickness: 1.0 };
        assert!(matches!(nan.output_size(px(10, 10)), Err(PipelineError::NonFiniteParam { .. })));
    }

    #[test]
    fn draw_text_defaults_and_validation() {
        use crate::color::Rgba8;
        let op: OpSpec = serde_json::from_str(
            r#"{ "op": "draw_text", "x": 10, "y": 20, "text": "Hello\nмир", "color": { "r": 0, "g": 0, "b": 0 } }"#,
        )
        .unwrap();
        match &op {
            OpSpec::DrawText { text, font_size, align_x, align_y, font, .. } => {
                assert_eq!(text, "Hello\nмир", "unicode text preserved");
                assert_eq!(*font_size, 24.0, "font_size defaults");
                assert_eq!(*align_x, AlignX::Left);
                assert_eq!(*align_y, AlignY::Baseline);
                assert!(font.is_none());
            }
            _ => panic!("wrong variant"),
        }
        assert_eq!(op.name(), "draw_text");
        assert_eq!(op.output_size(px(200, 100)).unwrap(), px(200, 100), "text keeps size");

        let bad = OpSpec::DrawText { x: 0.0, y: 0.0, text: "x".into(), color: Rgba8::rgb(0, 0, 0), font_size: 0.0, font: None, align_x: AlignX::Left, align_y: AlignY::Baseline, line_height: None };
        assert!(matches!(bad.output_size(px(10, 10)), Err(PipelineError::NegativeParam { .. })), "font_size must be > 0");
    }

    #[test]
    fn draw_rect_json_shape() {
        use crate::color::Rgba8;
        // agent-authored JSON with a partial color (alpha defaults to 255)
        let op: OpSpec = serde_json::from_str(
            r#"{ "op": "draw_rect", "x": 10, "y": 20, "width": 100, "height": 50,
                 "corner_radius": 8, "stroke": { "r": 255, "g": 0, "b": 0 } }"#,
        )
        .unwrap();
        match op {
            OpSpec::DrawRect { stroke: Some(c), stroke_width, fill, .. } => {
                assert_eq!(c, Rgba8::new(255, 0, 0, 255), "alpha defaults to opaque");
                assert_eq!(stroke_width, 3.0, "stroke_width defaults");
                assert!(fill.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn validation_errors() {
        let bad_crop = Pipeline {
            version: 0,
            steps: vec![OpSpec::Crop { x: 0, y: 0, width: 200, height: 1 }],
        };
        assert!(matches!(
            bad_crop.validate(px(100, 100)),
            Err(PipelineError::AtStep { index: 0, .. })
        ));

        let no_dims = OpSpec::Resize { width: None, height: None, filter: Filter::default() };
        assert_eq!(no_dims.output_size(px(1, 1)), Err(PipelineError::ResizeMissingDims));

        let nan = OpSpec::Exposure { stops: f32::NAN };
        assert!(matches!(nan.output_size(px(1, 1)), Err(PipelineError::NonFiniteParam { .. })));

        let future = Pipeline { version: 999, steps: vec![] };
        assert_eq!(future.validate(px(1, 1)), Err(PipelineError::UnsupportedVersion(999)));
    }

    #[test]
    fn sizes_flow_through_chain() {
        let p = Pipeline {
            version: 0,
            steps: vec![
                OpSpec::Resize { width: Some(800), height: Some(600), filter: Filter::default() },
                OpSpec::Crop { x: 0, y: 0, width: 640, height: 480 },
                OpSpec::Grayscale,
            ],
        };
        assert_eq!(p.validate(px(4000, 3000)).unwrap(), vec![px(800, 600), px(640, 480), px(640, 480)]);
    }

    #[test]
    fn rotate_output_size() {
        let img = px(800, 600);
        // orthogonal: 90/270 swap, 0/180 keep — expand is irrelevant here
        for (deg, want) in [(0.0, px(800, 600)), (90.0, px(600, 800)), (180.0, px(800, 600)), (270.0, px(600, 800)), (-90.0, px(600, 800))] {
            let op = OpSpec::Rotate { degrees: deg, expand: true };
            assert_eq!(op.output_size(img).unwrap(), want, "{deg}°");
        }
        // 45° expand grows to the rotated bbox: (800+600)/√2 ≈ 989.9 → 990 both sides
        let op = OpSpec::Rotate { degrees: 45.0, expand: true };
        assert_eq!(op.output_size(img).unwrap(), px(990, 990));
        // 45° no-expand keeps the canvas
        let op = OpSpec::Rotate { degrees: 45.0, expand: false };
        assert_eq!(op.output_size(img).unwrap(), img);
        // non-finite angle is rejected
        let nan = OpSpec::Rotate { degrees: f32::NAN, expand: true };
        assert!(matches!(nan.output_size(img), Err(PipelineError::NonFiniteParam { .. })));
    }

    #[test]
    fn flip_keeps_size_and_pad_grows() {
        let img = px(100, 80);
        assert_eq!(OpSpec::Flip { axis: FlipAxis::Horizontal }.output_size(img).unwrap(), img);
        assert_eq!(OpSpec::Flip { axis: FlipAxis::Vertical }.output_size(img).unwrap(), img);

        let pad = OpSpec::Pad { left: 10, right: 20, top: 5, bottom: 15, color: transparent() };
        assert_eq!(pad.output_size(img).unwrap(), px(130, 100));

        // absurd padding overflows u32 and is rejected, not silently wrapped
        let huge = OpSpec::Pad { left: u32::MAX, right: 0, top: 0, bottom: 0, color: transparent() };
        assert!(matches!(huge.output_size(img), Err(PipelineError::ResultTooLarge { .. })));
    }

    #[test]
    fn geometry_json_shapes_and_defaults() {
        // rotate: expand defaults to true
        let op: OpSpec = serde_json::from_str(r#"{ "op": "rotate", "degrees": 90 }"#).unwrap();
        assert_eq!(op, OpSpec::Rotate { degrees: 90.0, expand: true });
        assert_eq!(op.name(), "rotate");
        // flip axis is snake_case
        let op: OpSpec = serde_json::from_str(r#"{ "op": "flip", "axis": "horizontal" }"#).unwrap();
        assert_eq!(op, OpSpec::Flip { axis: FlipAxis::Horizontal });
        // pad: omitted sides default to 0, color to transparent
        let op: OpSpec = serde_json::from_str(r#"{ "op": "pad", "left": 8, "top": 8 }"#).unwrap();
        assert_eq!(op, OpSpec::Pad { left: 8, right: 0, top: 8, bottom: 0, color: Rgba8::new(0, 0, 0, 0) });
        // full round-trip through the tagged form
        let back: OpSpec = serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn color_and_filter_ops_keep_size() {
        let img = px(200, 150);
        for op in [
            OpSpec::HueRotate { degrees: 90.0 },
            OpSpec::Invert,
            OpSpec::Blur { radius: 4.0 },
            OpSpec::Redact { x: 10.0, y: 10.0, width: 50.0, height: 30.0, mode: RedactMode::default() },
            OpSpec::Spotlight { x: 20.0, y: 20.0, width: 80.0, height: 60.0, corner_radius: 8.0, dim: 0.6, color: black(), feather: 4.0 },
        ] {
            assert_eq!(op.output_size(img).unwrap(), img, "{} keeps size", op.name());
        }
        // beautify grows by 2·padding
        let b = OpSpec::Beautify { padding: 40, corner_radius: 16.0, shadow_radius: 24.0, shadow_opacity: 0.35, shadow_offset: 12.0, background: transparent() };
        assert_eq!(b.output_size(img).unwrap(), px(280, 230));
    }

    #[test]
    fn color_filter_validation_and_json() {
        // non-finite / negative params are rejected
        assert!(matches!(OpSpec::HueRotate { degrees: f32::NAN }.output_size(px(4, 4)), Err(PipelineError::NonFiniteParam { .. })));
        assert!(matches!(OpSpec::Blur { radius: -1.0 }.output_size(px(4, 4)), Err(PipelineError::NegativeParam { .. })));
        let bad_block = OpSpec::Redact { x: 0.0, y: 0.0, width: 4.0, height: 4.0, mode: RedactMode::Pixelate { block: 0 } };
        assert!(matches!(bad_block.output_size(px(4, 4)), Err(PipelineError::NegativeParam { .. })));

        // invert is a bare unit tag
        let op: OpSpec = serde_json::from_str(r#"{ "op": "invert" }"#).unwrap();
        assert_eq!(op, OpSpec::Invert);
        // redact mode defaults to pixelate(block=12); nested tagged enum round-trips
        let op: OpSpec = serde_json::from_str(r#"{ "op": "redact", "x": 0, "y": 0, "width": 10, "height": 10 }"#).unwrap();
        assert_eq!(op, OpSpec::Redact { x: 0.0, y: 0.0, width: 10.0, height: 10.0, mode: RedactMode::Pixelate { block: 12 } });
        let op: OpSpec = serde_json::from_str(r#"{ "op": "redact", "x": 0, "y": 0, "width": 10, "height": 10, "mode": { "type": "blur", "radius": 8 } }"#).unwrap();
        assert_eq!(op, OpSpec::Redact { x: 0.0, y: 0.0, width: 10.0, height: 10.0, mode: RedactMode::Blur { radius: 8.0 } });
        let back: OpSpec = serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(op, back);
        // beautify defaults
        let op: OpSpec = serde_json::from_str(r#"{ "op": "beautify" }"#).unwrap();
        assert_eq!(op, OpSpec::Beautify { padding: 64, corner_radius: 16.0, shadow_radius: 24.0, shadow_opacity: 0.35, shadow_offset: 12.0, background: transparent() });
    }

    #[test]
    fn tonal_ops_keep_size_and_validate() {
        let img = px(200, 150);
        for op in [
            OpSpec::BrightnessContrast { brightness: 0.1, contrast: 0.2 },
            OpSpec::Saturation { amount: 1.4 },
            OpSpec::Levels { in_black: 0.05, in_white: 0.95, gamma: 1.2, out_black: 0.0, out_white: 1.0 },
            OpSpec::Curves { points: vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]] },
            OpSpec::WhiteBalance { temperature: 0.3, tint: -0.1 },
            OpSpec::GradientMap { low: black(), high: Rgba8::rgb(255, 255, 255), mid: None },
            OpSpec::Sharpen { amount: 1.0, radius: 2.0 },
            OpSpec::Vignette { amount: 0.5, feather: 0.5, color: black() },
        ] {
            assert_eq!(op.output_size(img).unwrap(), img, "{} keeps size", op.name());
        }
        // rejections
        assert!(matches!(OpSpec::Levels { in_black: 0.0, in_white: 1.0, gamma: 0.0, out_black: 0.0, out_white: 1.0 }.output_size(img), Err(PipelineError::NegativeParam { .. })), "gamma > 0");
        assert!(matches!(OpSpec::Curves { points: vec![[f32::NAN, 0.0]] }.output_size(img), Err(PipelineError::NonFiniteParam { .. })));
        assert!(matches!(OpSpec::Vignette { amount: -1.0, feather: 0.5, color: black() }.output_size(img), Err(PipelineError::NegativeParam { .. })));
    }

    #[test]
    fn tonal_json_shapes_and_defaults() {
        // brightness_contrast: both default to 0 (identity)
        let op: OpSpec = serde_json::from_str(r#"{ "op": "brightness_contrast", "contrast": 0.3 }"#).unwrap();
        assert_eq!(op, OpSpec::BrightnessContrast { brightness: 0.0, contrast: 0.3 });
        // levels: omitted fields default to identity
        let op: OpSpec = serde_json::from_str(r#"{ "op": "levels", "gamma": 1.5 }"#).unwrap();
        assert_eq!(op, OpSpec::Levels { in_black: 0.0, in_white: 1.0, gamma: 1.5, out_black: 0.0, out_white: 1.0 });
        // curves round-trips its points
        let op: OpSpec = serde_json::from_str(r#"{ "op": "curves", "points": [[0,0],[1,1]] }"#).unwrap();
        assert_eq!(op.name(), "curves");
        let back: OpSpec = serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(op, back);
        // gradient_map: mid optional
        let op: OpSpec = serde_json::from_str(r#"{ "op": "gradient_map", "low": { "r": 0, "g": 0, "b": 0 }, "high": { "r": 255, "g": 255, "b": 255 } }"#).unwrap();
        assert!(matches!(op, OpSpec::GradientMap { mid: None, .. }));
        // sharpen / vignette defaults
        assert_eq!(serde_json::from_str::<OpSpec>(r#"{ "op": "sharpen" }"#).unwrap(), OpSpec::Sharpen { amount: 1.0, radius: 2.0 });
        assert_eq!(serde_json::from_str::<OpSpec>(r#"{ "op": "vignette" }"#).unwrap(), OpSpec::Vignette { amount: 0.5, feather: 0.5, color: black() });
    }
}
