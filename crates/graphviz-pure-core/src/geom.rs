use crate::math;

/// 2D point in Graphviz points (72 points = 1 inch).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn dist(self, other: Self) -> f64 {
        math::hypot(self.x - other.x, self.y - other.y)
    }

    /// Safe vector normalization avoiding `NaN` on zero-length vectors (Fixes GV-BUG-006).
    #[inline]
    pub fn normalized_or_zero(self) -> Self {
        let len = math::hypot(self.x, self.y);
        if len <= 1e-12 {
            Self::new(0.0, 0.0)
        } else {
            Self::new(self.x / len, self.y / len)
        }
    }
}

/// Axis-aligned 2D bounding box `(ll, ur)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BoundingBox {
    pub ll: Point,
    pub ur: Point,
}

impl BoundingBox {
    #[inline]
    pub const fn new(llx: f64, lly: f64, urx: f64, ury: f64) -> Self {
        Self {
            ll: Point::new(llx, lly),
            ur: Point::new(urx, ury),
        }
    }

    #[inline]
    pub fn width(&self) -> f64 {
        (self.ur.x - self.ll.x).max(0.0)
    }

    #[inline]
    pub fn height(&self) -> f64 {
        (self.ur.y - self.ll.y).max(0.0)
    }

    #[inline]
    pub fn expand_to_include(&mut self, pt: Point) {
        if pt.x < self.ll.x {
            self.ll.x = pt.x;
        }
        if pt.y < self.ll.y {
            self.ll.y = pt.y;
        }
        if pt.x > self.ur.x {
            self.ur.x = pt.x;
        }
        if pt.y > self.ur.y {
            self.ur.y = pt.y;
        }
    }
}

/// Cubic Bezier spline chain representing a routed edge (C's `bezier`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BezierSegment {
    /// Control points: `3n + 1` points for `n` cubic Bezier segments.
    pub points: Vec<Point>,
    /// `bz.sp` and `bz.ep`: the tips of the start and end arrowheads, set where `sflag` and
    /// `eflag` are not zero.
    pub start_arrow: Option<Point>,
    pub end_arrow: Option<Point>,
    /// `bz.sflag` and `bz.eflag`: the arrowhead types (`crate::arrows`) the layout clipped
    /// the spline for, as `arrow_clip` (splines.c) leaves them: after `swap_ends`, `ARR_NONE`
    /// at a concentrator merge point, with an opposing edge's types merged under
    /// `concentrate`. Renderers draw an arrowhead only where its flag is not zero, as
    /// `emit_edge_graphics` does, whatever `start_arrow`/`end_arrow` hold.
    pub sflag: u32,
    pub eflag: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spline {
    pub segments: Vec<BezierSegment>,
}
