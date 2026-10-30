//! Geometry macros from `lib/common/geom.h` and `geomprocs.h` with the
//! floating-point contraction clang applies to them (`SQR(a) + SQR(b)` is
//! compiled as `fma(a, a, b * b)`).

use graphviz_pure_core::math;
use graphviz_pure_core::{BoundingBox, Point};

/// `DIST2(p, q)`.
#[inline]
pub fn dist2(p: Point, q: Point) -> f64 {
    let dx = p.x - q.x;
    let dy = p.y - q.y;
    math::fma(dx, dx, dy * dy)
}

/// `DIST(p, q)`.
#[inline]
pub fn dist(p: Point, q: Point) -> f64 {
    dist2(p, q).sqrt()
}

/// `APPROXEQPT(p, q, tol)`.
#[inline]
pub fn approxeqpt(p: Point, q: Point, tol: f64) -> bool {
    dist2(p, q) < tol * tol
}

pub const MILLIPOINT: f64 = 0.001;

pub fn add(p: Point, q: Point) -> Point {
    Point::new(p.x + q.x, p.y + q.y)
}

pub fn sub(p: Point, q: Point) -> Point {
    Point::new(p.x - q.x, p.y - q.y)
}

pub fn mid(p: Point, q: Point) -> Point {
    Point::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0)
}

/// `expandbp`: grow `b` to contain `p`.
pub fn expandbp(b: &mut BoundingBox, p: Point) {
    b.ll.x = b.ll.x.min(p.x);
    b.ll.y = b.ll.y.min(p.y);
    b.ur.x = b.ur.x.max(p.x);
    b.ur.y = b.ur.y.max(p.y);
}

/// `EXPANDBB(b0, b1)` for `boxf`.
pub fn expandbb(b0: &mut BoundingBox, b1: &BoundingBox) {
    b0.ll.x = b0.ll.x.min(b1.ll.x);
    b0.ll.y = b0.ll.y.min(b1.ll.y);
    b0.ur.x = b0.ur.x.max(b1.ur.x);
    b0.ur.y = b0.ur.y.max(b1.ur.y);
}
