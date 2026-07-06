//! Craws execution engine.
//!
//! The engine executes a [`craws_domain::Pipeline`] over a [`TiledImage`]:
//! a grid of 256² tiles of **linear-light, premultiplied RGBA f32**. Every
//! tile is addressed by a [`ContentHash`] derived Merkle-style — source digest
//! for source tiles, `H(op params, input hashes)` downstream — so cache lookups
//! never touch pixel data. Tweak one parameter and only the affected tiles of
//! the affected steps recompute; everything else is an `Arc` clone.
//!
//! Invariants (see repo CLAUDE.md):
//! - pixels inside the engine are linear premultiplied f32, never sRGB;
//! - linear values are NOT clamped mid-pipeline (HDR headroom) — only u8 export clamps;
//! - determinism: same pipeline + same source ⇒ bit-identical output.

pub mod cache;
pub mod color;
pub mod engine;
pub mod hash;
pub mod ops;
pub mod resample;
pub mod tile;

pub use engine::{Engine, EngineError, NodeStat, RunStats, DEFAULT_CACHE_BYTES};
pub use hash::ContentHash;
pub use tile::{Tile, TiledImage, TILE_SIZE};
