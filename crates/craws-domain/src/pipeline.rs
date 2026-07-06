//! Pipeline spec: an ordered chain of operations over one image.
//!
//! This is *data*, not behavior — the engine interprets it. The JSON form is the
//! public automation contract (`craws run pipeline.json`), so changes here are
//! format changes: bump [`Pipeline::CURRENT_VERSION`] and keep old versions parsing.

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
}

impl OpSpec {
    /// Stable machine name (matches the serde tag).
    pub fn name(&self) -> &'static str {
        match self {
            OpSpec::Resize { .. } => "resize",
            OpSpec::Crop { .. } => "crop",
            OpSpec::Exposure { .. } => "exposure",
            OpSpec::Grayscale => "grayscale",
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
        }
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
