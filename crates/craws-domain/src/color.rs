//! Colors as authored by users: straight-alpha 8-bit sRGB. The engine converts
//! to its internal linear-premultiplied form at draw time — the domain only
//! carries the value, never interprets pixels.

use serde::{Deserialize, Serialize};

/// Straight-alpha sRGB color, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    #[serde(default = "opaque")]
    pub a: u8,
}

fn opaque() -> u8 {
    255
}

impl Rgba8 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}
