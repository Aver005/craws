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
        }
    }

    /// Validate against the incoming image size; return the outgoing size.
    pub fn output_size(&self, input: Size) -> Result<Size, PipelineError> {
        debug_assert!(!input.is_empty(), "engine never feeds empty images");
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
}
