//! Craws domain core — pure data types shared by every layer.
//!
//! The one rule of the workspace: dependency arrows point inward, and this crate
//! is the innermost ring. It knows nothing about files, pixels-in-memory layout,
//! GPUs or UI — only *what* an edit is, not *how* it executes.

pub mod color;
pub mod geometry;
pub mod pipeline;

pub use color::Rgba8;
pub use geometry::{Rect, Size};
pub use pipeline::{AlignX, AlignY, Filter, FlipAxis, OpSpec, Pipeline, PipelineError, RedactMode};
