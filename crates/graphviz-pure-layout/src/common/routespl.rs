//! Box-path spline routing, ported from `lib/common/routespl.c`
//! (`routesplines`, `routepolylines`, `simpleSplineRoute`, `limitBoxes`,
//! `checkpath`, `overlap`) plus the `path`/`pathend_t` types of
//! `lib/common/types.h`.
//!
//! A [`Path`] is a corridor of contiguous boxes from a start to an end point.
//! `routesplines_` turns the boxes into a polygon, finds the shortest path
//! inside it with `Pshortestpath` and fits a spline with `Proutespline`; it
//! then shrinks each box to the horizontal extent the spline actually uses,
//! so later edges can use the space (`recover_slack` in dot).

use graphviz_pure_core::diag::agerrorf;
use graphviz_pure_core::math;
use graphviz_pure_core::{BoundingBox, Point};
use graphviz_pure_geom::pathplan::{Pedge, make_polyline, proutespline, pshortestpath};

/// `port` end of a [`Path`] (`path.start` / `path.end`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PathPort {
    pub p: Point,
    pub theta: f64,
    pub constrained: bool,
}

/// `path` (types.h): the boxes the router may use.
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub start: PathPort,
    pub end: PathPort,
    /// `boxes[0..nbox]`; `boxes.len()` is the C `nbox`.
    pub boxes: Vec<BoundingBox>,
    /// `data`: the edge being routed.
    pub data: usize,
}

/// `pathend_t` (types.h).
#[derive(Debug, Clone, Default)]
pub struct PathEnd {
    /// Node box (`nb`).
    pub nb: BoundingBox,
    /// Node port (`np`).
    pub np: Point,
    pub sidemask: u8,
    /// `boxes[0..boxn]`.
    pub boxes: Vec<BoundingBox>,
}

/// `add_box(P, b)` (splines.c): append `b` unless it is empty.
pub fn add_box(p: &mut Path, b: BoundingBox) {
    if b.ll.x < b.ur.x && b.ll.y < b.ur.y {
        p.boxes.push(b);
    }
}

fn gvdump() -> bool {
    std::env::var_os("GVDUMP").is_some()
}

/// `simpleSplineRoute(tp, hp, poly, &n, polyline)`: route inside a simple polygon.
pub fn simple_spline_route(
    tp: Point,
    hp: Point,
    poly: &[Point],
    polyline: bool,
) -> Option<Vec<Point>> {
    let pl = pshortestpath(poly, [tp, hp]).ok()?;
    if polyline {
        Some(make_polyline(&pl))
    } else {
        let pn = poly.len();
        let edges: Vec<Pedge> = (0..pn)
            .map(|i| Pedge {
                a: poly[i],
                b: poly[(i + 1) % pn],
            })
            .collect();
        proutespline(&edges, &pl, [Point::default(), Point::default()]).ok()
    }
}

/// `limitBoxes`: widen each box to the x-extent of the spline samples within it.
fn limit_boxes(boxes: &mut [BoundingBox], pps: &[Point], delta: f64) {
    let boxn = boxes.len();
    let num_div = delta * boxn as f64;
    let pn = pps.len();
    let mut splinepi = 0;
    while splinepi + 3 < pn {
        let mut si = 0.0f64;
        while si <= num_div {
            let t = si / num_div;
            let mut sp = [
                pps[splinepi],
                pps[splinepi + 1],
                pps[splinepi + 2],
                pps[splinepi + 3],
            ];
            // `a += t * (b - a)`, contracted to fma(t, b - a, a) by clang.
            let lerp = |a: f64, b: f64| t.mul_add(b - a, a);
            sp[0].x = lerp(sp[0].x, sp[1].x);
            sp[0].y = lerp(sp[0].y, sp[1].y);
            sp[1].x = lerp(sp[1].x, sp[2].x);
            sp[1].y = lerp(sp[1].y, sp[2].y);
            sp[2].x = lerp(sp[2].x, sp[3].x);
            sp[2].y = lerp(sp[2].y, sp[3].y);
            sp[0].x = lerp(sp[0].x, sp[1].x);
            sp[0].y = lerp(sp[0].y, sp[1].y);
            sp[1].x = lerp(sp[1].x, sp[2].x);
            sp[1].y = lerp(sp[1].y, sp[2].y);
            sp[0].x = lerp(sp[0].x, sp[1].x);
            sp[0].y = lerp(sp[0].y, sp[1].y);
            for b in boxes.iter_mut() {
                // FUDGE .0001 (32-bit robustness in C).
                if sp[0].y <= b.ur.y + 0.0001 && sp[0].y >= b.ll.y - 0.0001 {
                    b.ll.x = b.ll.x.min(sp[0].x);
                    b.ur.x = b.ur.x.max(sp[0].x);
                }
            }
            si += 1.0;
        }
        splinepi += 3;
    }
}

const INIT_DELTA: f64 = 10.0;
const LOOP_TRIES: i32 = 15;

/// `realedge`, the path's real edge (found through `ED_to_orig`), for the messages.
#[derive(Debug, Clone)]
pub struct RealEdge<'a> {
    pub tail: graphviz_pure_core::AgName<'a>,
    pub head: graphviz_pure_core::AgName<'a>,
    /// The edge is a self loop, which C refuses.
    pub is_loop: bool,
}

/// `routesplines_(pp, &npoints, polyline)`. Returns `None` where C returns NULL (and
/// `npoints == 0`).
pub fn routesplines_(pp: &mut Path, polyline: bool, real: RealEdge<'_>) -> Option<Vec<Point>> {
    ROUTE.with(|r| {
        let (depth, nedges, nboxes) = r.get();
        r.set((
            depth,
            nedges.wrapping_add(1),
            nboxes.wrapping_add(pp.boxes.len()),
        ));
    });
    if checkpath(pp) {
        return None;
    }
    let boxn = pp.boxes.len();
    if gvdump() {
        let mut s = format!("RS boxes {boxn}:");
        for b in &pp.boxes {
            s.push_str(&format!(
                " [{:.6} {:.6} {:.6} {:.6}]",
                b.ll.x, b.ll.y, b.ur.x, b.ur.y
            ));
        }
        graphviz_pure_core::diag::dump(&s);
        graphviz_pure_core::diag::dump(&format!(
            "RS ends {:.6},{:.6} {:.6} {} -> {:.6},{:.6} {:.6} {}",
            pp.start.p.x,
            pp.start.p.y,
            pp.start.theta,
            i32::from(pp.start.constrained),
            pp.end.p.x,
            pp.end.p.y,
            pp.end.theta,
            i32::from(pp.end.constrained)
        ));
    }
    let boxes = &mut pp.boxes;
    let mut polypoints: Vec<Point> = Vec::with_capacity(boxn * 8);
    let flip = if boxn > 1 && boxes[0].ll.y > boxes[1].ll.y {
        for b in boxes.iter_mut() {
            let v = b.ur.y;
            b.ur.y = -b.ll.y;
            b.ll.y = -v;
        }
        true
    } else {
        false
    };
    if real.is_loop {
        agerrorf(&format!(
            "in routesplines, edge is a loop at {}\n",
            real.head.name
        ));
        return None;
    }
    let pt = |x: f64, y: f64| Point::new(x, y);
    for bi in 0..boxn {
        let mut next = 0;
        let mut prev = 0;
        if bi > 0 {
            prev = if boxes[bi].ll.y > boxes[bi - 1].ll.y {
                -1
            } else {
                1
            };
        }
        if bi + 1 < boxn {
            next = if boxes[bi + 1].ll.y > boxes[bi].ll.y {
                1
            } else {
                -1
            };
        }
        let b = boxes[bi];
        if prev != next {
            if next == -1 || prev == 1 {
                polypoints.push(pt(b.ll.x, b.ur.y));
                polypoints.push(pt(b.ll.x, b.ll.y));
            } else {
                polypoints.push(pt(b.ur.x, b.ll.y));
                polypoints.push(pt(b.ur.x, b.ur.y));
            }
        } else if prev == 0 {
            polypoints.push(pt(b.ll.x, b.ur.y));
            polypoints.push(pt(b.ll.x, b.ll.y));
        } else if !(prev == -1 && next == -1) {
            agerrorf(&format!(
                "in routesplines, illegal values of prev {prev} and next {next}, line 378\n"
            ));
            return None;
        }
    }
    for bi in (0..boxn).rev() {
        let mut next = 0;
        let mut prev = 0;
        if bi + 1 < boxn {
            prev = if boxes[bi].ll.y > boxes[bi + 1].ll.y {
                -1
            } else {
                1
            };
        }
        if bi > 0 {
            next = if boxes[bi - 1].ll.y > boxes[bi].ll.y {
                1
            } else {
                -1
            };
        }
        let b = boxes[bi];
        if prev != next {
            if next == -1 || prev == 1 {
                polypoints.push(pt(b.ll.x, b.ur.y));
                polypoints.push(pt(b.ll.x, b.ll.y));
            } else {
                polypoints.push(pt(b.ur.x, b.ll.y));
                polypoints.push(pt(b.ur.x, b.ur.y));
            }
        } else if prev == 0 {
            polypoints.push(pt(b.ur.x, b.ll.y));
            polypoints.push(pt(b.ur.x, b.ur.y));
        } else {
            if !(prev == -1 && next == -1) {
                agerrorf(&format!(
                    "in routesplines, illegal values of prev {prev} and next {next}, line 412\n"
                ));
                return None;
            }
            polypoints.push(pt(b.ur.x, b.ll.y));
            polypoints.push(pt(b.ur.x, b.ur.y));
            polypoints.push(pt(b.ll.x, b.ur.y));
            polypoints.push(pt(b.ll.x, b.ll.y));
        }
    }
    if flip {
        for b in boxes.iter_mut() {
            let v = b.ur.y;
            b.ur.y = -b.ll.y;
            b.ll.y = -v;
        }
        for p in &mut polypoints {
            p.y *= -1.0;
        }
    }
    for b in boxes.iter_mut() {
        b.ll.x = f64::MAX;
        b.ur.x = -f64::MAX;
    }
    let eps = [pp.start.p, pp.end.p];
    let pl = match pshortestpath(&polypoints, eps) {
        Ok(pl) => pl,
        Err(_) => {
            if gvdump() {
                graphviz_pure_core::diag::dump("RS Pshortestpath failed");
            }
            agerrorf("in routesplines, Pshortestpath failed\n");
            return None;
        }
    };
    if gvdump() {
        let mut s = format!("RS poly {}:", polypoints.len());
        for p in &polypoints {
            s.push_str(&format!(" {:.6},{:.6}", p.x, p.y));
        }
        graphviz_pure_core::diag::dump(&s);
        let mut s = format!("RS sp {}:", pl.len());
        for p in &pl {
            s.push_str(&format!(" {:.6},{:.6}", p.x, p.y));
        }
        graphviz_pure_core::diag::dump(&s);
    }
    let spl = if polyline {
        make_polyline(&pl)
    } else {
        let pn = polypoints.len();
        let edges: Vec<Pedge> = (0..pn)
            .map(|i| Pedge {
                a: polypoints[i],
                b: polypoints[(i + 1) % pn],
            })
            .collect();
        let mut evs = [Point::default(); 2];
        if pp.start.constrained {
            evs[0] = Point::new(math::cos(pp.start.theta), math::sin(pp.start.theta));
        }
        if pp.end.constrained {
            evs[1] = Point::new(-math::cos(pp.end.theta), -math::sin(pp.end.theta));
        }
        match proutespline(&edges, &pl, evs) {
            Ok(s) => s,
            Err(_) => {
                if gvdump() {
                    graphviz_pure_core::diag::dump("RS Proutespline failed");
                }
                agerrorf("in routesplines, Proutespline failed\n");
                return None;
            }
        }
    };
    let ps = spl;
    let boxes = &mut pp.boxes;
    let mut unbounded = true;
    {
        let is_horizontal = is_straight(&ps, |p| p.y);
        let is_vertical = is_straight(&ps, |p| p.x);
        if let (Some(a), Some(b)) = (ps.first(), ps.last()) {
            use graphviz_pure_core::diag::fmt_f;
            let f = |v: f64| fmt_f(v, 3);
            let ends = format!("[{}, {}] -- [{}, {}]", f(a.x), f(a.y), f(b.x), f(b.y));
            if is_horizontal {
                graphviz_pure_core::gv_info!(
                    "routespl.c",
                    520,
                    "spline {ends} is horizontal; will be trivially bounded"
                );
            }
            if is_vertical {
                graphviz_pure_core::gv_info!(
                    "routespl.c",
                    534,
                    "spline {ends} is vertical; will be trivially bounded"
                );
            }
        }
        if is_horizontal || is_vertical {
            for b in boxes.iter_mut() {
                b.ll.x = ps[0].x;
                b.ur.x = ps[0].x;
            }
            unbounded = false;
        }
    }
    let mut delta = INIT_DELTA;
    let mut loopcnt = 0;
    while unbounded && loopcnt < LOOP_TRIES {
        limit_boxes(boxes, &ps, delta);
        let mut all = true;
        for b in boxes.iter() {
            if b.ll.x == f64::MAX || b.ur.x == -f64::MAX {
                delta *= 2.0;
                all = false;
                break;
            }
        }
        if all {
            unbounded = false;
        }
        loopcnt += 1;
    }
    if unbounded {
        // routespl.c:578: both names share agnameof's static buffer when anonymous.
        let [t, h] = graphviz_pure_core::agnameof_buffer([real.tail, real.head]);
        graphviz_pure_core::diag::agwarningf(&format!(
            "Unable to reclaim box space in spline routing for edge \"{t}\" -> \"{h}\". \
             Something is probably seriously wrong.\n"
        ));
        let polyspl = make_polyline(&pl);
        limit_boxes(boxes, &polyspl, INIT_DELTA);
    }
    if gvdump() {
        let mut s = format!("RS out {}:", ps.len());
        for p in &ps {
            s.push_str(&format!(" {:.6},{:.6}", p.x, p.y));
        }
        graphviz_pure_core::diag::dump(&s);
    }
    Some(ps)
}

/// `is_horizontal`/`is_vertical` of `routesplines_` (routespl.c:508-545): true unless some
/// point's coordinate is more than FUDGE from the first's. `fabs(...) > FUDGE` is false for
/// NaN, so a spline whose coordinate is NaN counts as straight, its boxes are bounded at once
/// and C prints no warning (#396). `sameport`'s 0/0 direction gives such splines; see
/// `docs/porting/crashes.md`, "sameport's 0/0 direction".
fn is_straight(ps: &[Point], coord: impl Fn(&Point) -> f64) -> bool {
    !ps.is_empty() && !ps.iter().any(|p| (coord(&ps[0]) - coord(p)).abs() > 0.0001)
}

/// `routesplines(pp, &npoints)`.
pub fn routesplines(pp: &mut Path, real: RealEdge<'_>) -> Option<Vec<Point>> {
    routesplines_(pp, false, real)
}

/// `routepolylines(pp, &npoints)`.
pub fn routepolylines(pp: &mut Path, real: RealEdge<'_>) -> Option<Vec<Point>> {
    routesplines_(pp, true, real)
}

/// `overlap(i0, i1, j0, j1)`: length of the intersection of two intervals.
fn overlap(i0: f64, i1: f64, j0: f64, j1: f64) -> f64 {
    if i1 <= j0 {
        return 0.0;
    }
    if i0 >= j1 {
        return 0.0;
    }
    if i0 <= j0 && i1 >= j1 {
        return i1 - i0;
    }
    if j0 <= i0 && j1 >= i1 {
        return j1 - j0;
    }
    if j0 <= i0 && i0 <= j1 {
        return j1 - i0;
    }
    i1 - j0
}

thread_local! {
    /// `routeinit`, `nedges` and `nboxes`: the nesting of `routesplinesinit` and the edges
    /// and boxes routed since the outermost one.
    static ROUTE: std::cell::Cell<(i32, i32, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
}

/// `routesplinesinit()`: counts the edges and boxes routed until the matching
/// `routesplinesterm()`, which dropping the returned guard runs, and starts the clock (`-v`).
pub fn routesplines_init() -> RouteSplines {
    let depth = ROUTE.with(|r| r.get().0).wrapping_add(1);
    if depth > 1 {
        ROUTE.with(|r| r.set((depth, r.get().1, r.get().2)));
    } else {
        ROUTE.with(|r| r.set((depth, 0, 0)));
        if graphviz_pure_core::diag::verbose_on() {
            graphviz_pure_core::diag::start_timer();
        }
    }
    RouteSplines(())
}

/// A [`routesplines_init`] level, closed by `routesplinesterm()` when dropped.
#[must_use]
pub struct RouteSplines(());

impl Drop for RouteSplines {
    /// `routesplinesterm()`: at the outermost level, the `-v` summary.
    fn drop(&mut self) {
        let (depth, nedges, nboxes) = ROUTE.with(|r| r.get());
        let depth = depth.wrapping_sub(1);
        ROUTE.with(|r| r.set((depth, nedges, nboxes)));
        if depth > 0 {
            return;
        }
        graphviz_pure_core::gv_info!(
            "routespl.c",
            235,
            "routesplines: {nedges} edges, {nboxes} boxes {} sec",
            graphviz_pure_core::diag::fmt_f(graphviz_pure_core::diag::elapsed_sec(), 2)
        );
    }
}

/// `printpath(pp)` (`routespl.c:758-771`): every one of the `nbox` boxes (the caller's
/// count, so after `checkpath` squeezes out degenerate boxes the tail repeats the last
/// originals) and the end ports.
fn printpath(pp: &Path) {
    graphviz_pure_core::diag::fprintf_stderr(&printpath_text(pp));
}

/// The text [`printpath`] writes to stderr.
fn printpath_text(pp: &Path) -> String {
    use graphviz_pure_core::diag::fmt_g;
    let g = |v: f64| fmt_g(v, 5);
    let mut s = format!("{} boxes:\n", pp.boxes.len());
    for (bi, b) in pp.boxes.iter().enumerate() {
        s += &format!(
            "{bi} ({}, {}), ({}, {})\n",
            g(b.ll.x),
            g(b.ll.y),
            g(b.ur.x),
            g(b.ur.y)
        );
    }
    for (name, e) in [("start", &pp.start), ("end", &pp.end)] {
        s += &format!(
            "{name} port: ({}, {}), tangent angle: {}, {}\n",
            g(e.p.x),
            g(e.p.y),
            g(e.theta),
            if e.constrained {
                "constrained"
            } else {
                "not constrained"
            }
        );
    }
    s
}

/// `checkpath(boxn, boxes, thepath)`: drop degenerate boxes, repair boxes that
/// do not touch or that overlap, and clamp the end points into the end boxes.
/// Returns true on failure.
pub fn checkpath(thepath: &mut Path) -> bool {
    let boxes = &mut thepath.boxes;
    // Degenerate boxes are squeezed out in place, but C only shortens its local
    // `boxn`: the caller keeps the original count, so the tail of the array
    // still holds the last original boxes.
    let mut i = 0;
    for bi in 0..boxes.len() {
        let b = boxes[bi];
        if (b.ll.y - b.ur.y).abs() < 0.01 || (b.ll.x - b.ur.x).abs() < 0.01 {
            continue;
        }
        boxes[i] = b;
        i += 1;
    }
    let boxn = i;
    if boxn == 0 {
        return true;
    }
    let ba = boxes[0];
    if ba.ll.x > ba.ur.x || ba.ll.y > ba.ur.y {
        agerrorf("in checkpath, box 0 has LL coord > UR coord\n");
        printpath(thepath);
        return true;
    }
    for bi in 0..boxn.saturating_sub(1) {
        let next = bi.wrapping_add(1);
        let (Some(&a), Some(&b)) = (thepath.boxes.get(bi), thepath.boxes.get(next)) else {
            break;
        };
        if b.ll.x > b.ur.x || b.ll.y > b.ur.y {
            agerrorf(&format!(
                "in checkpath, box {next} has LL coord > UR coord\n"
            ));
            printpath(thepath);
            return true;
        }
        let mut l = i32::from(a.ur.x < b.ll.x);
        let mut r = i32::from(a.ll.x > b.ur.x);
        let mut d = i32::from(a.ur.y < b.ll.y);
        let mut u = i32::from(a.ll.y > b.ur.y);
        let errs = l + r + d + u;
        if errs > 0 && graphviz_pure_core::diag::verbose_on() {
            graphviz_pure_core::diag::fprintf_stderr(&format!(
                "in checkpath, boxes {bi} and {next} don't touch\n"
            ));
            printpath(thepath);
        }
        let (lo, hi) = thepath.boxes.split_at_mut(next);
        let ba = &mut lo[bi];
        let bb = &mut hi[0];
        if errs > 0 {
            if l == 1 {
                std::mem::swap(&mut ba.ur.x, &mut bb.ll.x);
                l = 0;
            } else if r == 1 {
                std::mem::swap(&mut ba.ll.x, &mut bb.ur.x);
                r = 0;
            } else if d == 1 {
                std::mem::swap(&mut ba.ur.y, &mut bb.ll.y);
                d = 0;
            } else if u == 1 {
                std::mem::swap(&mut ba.ll.y, &mut bb.ur.y);
                u = 0;
            }
            for _ in 0..errs - 1 {
                if l == 1 {
                    let xy = (ba.ur.x + bb.ll.x) / 2.0 + 0.5;
                    ba.ur.x = xy;
                    bb.ll.x = xy;
                    l = 0;
                } else if r == 1 {
                    let xy = (ba.ll.x + bb.ur.x) / 2.0 + 0.5;
                    ba.ll.x = xy;
                    bb.ur.x = xy;
                    r = 0;
                } else if d == 1 {
                    let xy = (ba.ur.y + bb.ll.y) / 2.0 + 0.5;
                    ba.ur.y = xy;
                    bb.ll.y = xy;
                    d = 0;
                } else if u == 1 {
                    let xy = (ba.ll.y + bb.ur.y) / 2.0 + 0.5;
                    ba.ll.y = xy;
                    bb.ur.y = xy;
                    u = 0;
                }
            }
        }
        let xoverlap = overlap(ba.ll.x, ba.ur.x, bb.ll.x, bb.ur.x);
        let yoverlap = overlap(ba.ll.y, ba.ur.y, bb.ll.y, bb.ur.y);
        if xoverlap > 0.0 && yoverlap > 0.0 {
            if xoverlap < yoverlap {
                if ba.ur.x - ba.ll.x > bb.ur.x - bb.ll.x {
                    if ba.ur.x < bb.ur.x {
                        ba.ur.x = bb.ll.x;
                    } else {
                        ba.ll.x = bb.ur.x;
                    }
                } else if ba.ur.x < bb.ur.x {
                    bb.ll.x = ba.ur.x;
                } else {
                    bb.ur.x = ba.ll.x;
                }
            } else if ba.ur.y - ba.ll.y > bb.ur.y - bb.ll.y {
                if ba.ur.y < bb.ur.y {
                    ba.ur.y = bb.ll.y;
                } else {
                    ba.ll.y = bb.ur.y;
                }
            } else if ba.ur.y < bb.ur.y {
                bb.ll.y = ba.ur.y;
            } else {
                bb.ur.y = ba.ll.y;
            }
        }
    }
    let boxes = &mut thepath.boxes;
    let b0 = boxes[0];
    let sp = &mut thepath.start.p;
    if sp.x < b0.ll.x || sp.x > b0.ur.x || sp.y < b0.ll.y || sp.y > b0.ur.y {
        sp.x = sp.x.max(b0.ll.x);
        sp.x = sp.x.min(b0.ur.x);
        sp.y = sp.y.max(b0.ll.y);
        sp.y = sp.y.min(b0.ur.y);
    }
    let bl = boxes[boxn - 1];
    let ep = &mut thepath.end.p;
    if ep.x < bl.ll.x || ep.x > bl.ur.x || ep.y < bl.ll.y || ep.y > bl.ur.y {
        ep.x = ep.x.max(bl.ll.x);
        ep.x = ep.x.min(bl.ur.x);
        ep.y = ep.y.max(bl.ll.y);
        ep.y = ep.y.min(bl.ur.y);
    }
    false
}

/// `bend(spl, centroid)`: pull both control points away from the centroid.
pub fn bend(spl: &mut [Point; 4], centroid: Point) {
    let midpt = super::geom::mid(spl[0], spl[3]);
    let dist = super::geom::dist(spl[3], spl[0]);
    let r = dist / 5.0;
    let vx = centroid.x - midpt.x;
    let vy = centroid.y - midpt.y;
    let mag_v = math::hypot(vx, vy);
    if mag_v == 0.0 {
        return;
    }
    // C (clang) fuses `midpt - v / magV * r`.
    let a = Point::new(
        (-(vx / mag_v)).mul_add(r, midpt.x),
        (-(vy / mag_v)).mul_add(r, midpt.y),
    );
    spl[1] = a;
    spl[2] = a;
}

/// `makeStraightEdges(g, edge_list, e_cnt, et, sinfo)`: straight (or, for
/// `splines=curved`, bent) edges, spread `GD_nodesep(g->root)` apart.
/// `centroid` is `get_cycle_centroid(g, e)` of the caller's graph.
pub fn make_straight_edges<S: super::splines::SplineGraph + ?Sized>(
    g: &mut S,
    edge_list: &[usize],
    et: super::types::EdgeType,
    nodesep: i64,
    concentrate: bool,
    centroid: &dyn Fn(&S, usize) -> Point,
) {
    use super::geom::{MILLIPOINT, add, approxeqpt};
    use super::splines::{add_edge_labels, clip_and_install};
    use super::types::EdgeType;
    let e = edge_list[0];
    let n = g.tail(e);
    let head = g.head(e);
    let p0 = add(g.coord(n), g.tail_port(e).p);
    let p3 = add(g.coord(head), g.head_port(e).p);
    let mut dumb = [p0, p0, p3, p3];
    let e_cnt = edge_list.len();
    if e_cnt == 1 || concentrate {
        if et == EdgeType::Curved {
            let c = centroid(g, e);
            bend(&mut dumb, c);
        }
        clip_and_install(g, e, head, &dumb);
        add_edge_labels(g, e);
        return;
    }
    let del = if approxeqpt(dumb[0], dumb[3], MILLIPOINT) {
        dumb[1] = dumb[0];
        dumb[2] = dumb[3];
        Point::new(0.0, 0.0)
    } else {
        let perp = Point::new(dumb[0].y - dumb[3].y, dumb[3].x - dumb[0].x);
        let l_perp = math::hypot(perp.x, perp.y);
        let xstep = nodesep;
        let dx = (xstep * (e_cnt as i64 - 1) / 2) as f64;
        dumb[1].x = dumb[0].x + dx * perp.x / l_perp;
        dumb[1].y = dumb[0].y + dx * perp.y / l_perp;
        dumb[2].x = dumb[3].x + dx * perp.x / l_perp;
        dumb[2].y = dumb[3].y + dx * perp.y / l_perp;
        Point::new(
            -(xstep as f64) * perp.x / l_perp,
            -(xstep as f64) * perp.y / l_perp,
        )
    };
    for &e0 in edge_list {
        let dumber = if g.head(e0) == head {
            dumb
        } else {
            [dumb[3], dumb[2], dumb[1], dumb[0]]
        };
        let h0 = g.head(e0);
        if et == EdgeType::Pline {
            let spl = make_polyline(&dumber);
            clip_and_install(g, e0, h0, &spl);
        } else {
            clip_and_install(g, e0, h0, &dumber);
        }
        add_edge_labels(g, e0);
        dumb[1] = super::geom::add(dumb[1], del);
        dumb[2] = super::geom::add(dumb[2], del);
    }
}

#[cfg(test)]
mod tests {

    /// Issue #416: a dropped guard closes its routesplinesinit level, on any way out.
    #[test]
    fn routesplines_levels_close_when_dropped() {
        let depth = || ROUTE.with(|r| r.get().0);
        let before = depth();
        {
            let _outer = routesplines_init();
            let inner = routesplines_init();
            assert_eq!(depth(), before + 2);
            drop(inner);
            assert_eq!(depth(), before + 1);
        }
        assert_eq!(depth(), before);
        let failing = || -> Result<(), ()> {
            let _route = routesplines_init();
            Err(())
        };
        assert!(failing().is_err());
        assert_eq!(depth(), before);
    }
    use super::*;

    /// #396: C's straightness test is true for a NaN coordinate, on each axis alone.
    #[test]
    fn nan_coordinate_counts_as_straight_like_c() {
        let nan = f64::NAN;
        let x_nan = [
            Point::new(nan, 0.0),
            Point::new(nan, 10.0),
            Point::new(nan, 20.0),
        ];
        assert!(is_straight(&x_nan, |p| p.x));
        assert!(!is_straight(&x_nan, |p| p.y));
        let y_nan = [
            Point::new(0.0, nan),
            Point::new(10.0, nan),
            Point::new(20.0, nan),
        ];
        assert!(is_straight(&y_nan, |p| p.y));
        assert!(!is_straight(&y_nan, |p| p.x));
        let first_nan = [Point::new(nan, nan), Point::new(5.0, 19.0)];
        assert!(is_straight(&first_nan, |p| p.x) && is_straight(&first_nan, |p| p.y));
        let within = [Point::new(1.0, 2.0), Point::new(1.00005, 7.0)];
        assert!(is_straight(&within, |p| p.x));
        assert!(!is_straight(&within, |p| p.y));
        assert!(!is_straight(&[], |p| p.x));
    }

    #[test]
    fn overlap_intervals() {
        assert_eq!(overlap(0.0, 1.0, 2.0, 3.0), 0.0);
        assert_eq!(overlap(0.0, 4.0, 1.0, 2.0), 4.0);
        assert_eq!(overlap(1.0, 2.0, 0.0, 4.0), 4.0);
        assert_eq!(overlap(1.0, 3.0, 0.0, 2.0), 1.0);
        assert_eq!(overlap(0.0, 2.0, 1.0, 3.0), 1.0);
    }

    /// `checkpath` failures dump the path as C's `printpath` does (`%.5g` numbers, the
    /// caller's box count). Issue #30.
    #[test]
    fn checkpath_failure_prints_the_path() {
        let mut p = Path {
            start: PathPort {
                p: Point::new(10.25, 100.0),
                theta: 1.25,
                constrained: true,
            },
            end: PathPort {
                p: Point::new(10.0, -0.000012345),
                ..PathPort::default()
            },
            boxes: vec![
                BoundingBox::new(0.0, 60.0, 20.0, 100.0),
                BoundingBox::new(0.0, 60.0, 20.0, 60.0),
                BoundingBox::new(30.0, 0.0, 20.0, 123456.7),
            ],
            data: 0,
        };
        assert!(checkpath(&mut p));
        assert_eq!(
            printpath_text(&p),
            "3 boxes:\n\
             0 (0, 60), (20, 100)\n\
             1 (30, 0), (20, 1.2346e+05)\n\
             2 (30, 0), (20, 1.2346e+05)\n\
             start port: (10.25, 100), tangent angle: 1.25, constrained\n\
             end port: (10, -1.2345e-05), tangent angle: 0, not constrained\n"
        );
    }

    #[test]
    fn straight_vertical_route() {
        let mut p = Path {
            start: PathPort {
                p: Point::new(10.0, 100.0),
                ..PathPort::default()
            },
            end: PathPort {
                p: Point::new(10.0, 0.0),
                ..PathPort::default()
            },
            boxes: vec![
                BoundingBox::new(0.0, 60.0, 20.0, 100.0),
                BoundingBox::new(0.0, 40.0, 20.0, 60.0),
                BoundingBox::new(0.0, 0.0, 20.0, 40.0),
            ],
            data: 0,
        };
        let real = RealEdge {
            tail: graphviz_pure_core::AgName::own("a"),
            head: graphviz_pure_core::AgName::own("b"),
            is_loop: false,
        };
        let ps = routesplines(&mut p, real).unwrap();
        assert_eq!(ps.len(), 4);
        assert!(ps.iter().all(|q| (q.x - 10.0).abs() < 1e-9));
        assert!(p.boxes.iter().all(|b| b.ll.x == 10.0 && b.ur.x == 10.0));
    }
}
