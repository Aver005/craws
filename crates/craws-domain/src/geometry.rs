//! Integer pixel geometry. Coordinates are u32; an image dimension of 0 is
//! invalid everywhere in Craws, enforced by [`crate::pipeline`] validation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Axis-aligned rectangle in image space (origin top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// True when the rect lies fully inside an image of size `s`.
    /// Overflow-safe: compares in u64.
    pub const fn fits_in(self, s: Size) -> bool {
        !self.size().is_empty()
            && self.x as u64 + self.width as u64 <= s.width as u64
            && self.y as u64 + self.height as u64 <= s.height as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_fits() {
        let s = Size::new(100, 50);
        assert!(Rect::new(0, 0, 100, 50).fits_in(s));
        assert!(Rect::new(99, 49, 1, 1).fits_in(s));
        assert!(!Rect::new(99, 49, 2, 1).fits_in(s));
        assert!(!Rect::new(0, 0, 101, 1).fits_in(s));
        assert!(!Rect::new(0, 0, 0, 10).fits_in(s), "empty rect never fits");
        // near-overflow coordinates must not wrap
        assert!(!Rect::new(u32::MAX, 0, 2, 1).fits_in(Size::new(u32::MAX, 1)));
    }

    #[test]
    fn size_area_is_u64() {
        assert_eq!(Size::new(u32::MAX, 2).area(), u32::MAX as u64 * 2);
    }
}
