//! Plain geometry types shared by the engine and the FFI layer.
//!
//! Coordinates follow the macOS Accessibility convention: the origin is the
//! top-left corner of the primary display and `y` grows downwards.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[repr(C)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub const fn origin(&self) -> Point {
        Point { x: self.x, y: self.y }
    }

    pub const fn size(&self) -> Size {
        Size { width: self.width, height: self.height }
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }
}
