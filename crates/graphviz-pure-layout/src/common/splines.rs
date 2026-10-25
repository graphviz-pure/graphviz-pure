//! Edge spline installation shared by all engines, ported from
//! `lib/common/splines.c` (`bezier_clip`, `shape_clip0`, `clip_and_install`,
//! `arrow_clip`, `beginpath`/`endpath`, `conc_slope`, `makeSelfEdge` and the
//! `self*` loop routers, port labels, `edgeMidpoint`, `getsplinepoints`),
//! `lib/common/utils.c` (`Bezier`, `dotneato_closest`) and
//! `lib/common/emit.c` (`update_bb_bz`).
//!
//! The code reaches the layout state through [`SplineGraph`], implemented by
//! the neato family's [`LGraph`] and by dot (`dotgen::splines_host`).

use super::arrows::{arrow_end_clip, arrow_flags, arrow_start_clip};
use super::geom::{MILLIPOINT, add, approxeqpt, dist, dist2, mid};
use super::ports::{BOTTOM, LEFT, RIGHT, TOP};
use super::routespl::{Path, PathEnd};
use super::shapes::{InsideCtx, NodeShape};
use super::types::{Bezier, EdgeType, LGraph, PortInfo, TextLabel};
use super::utils::late_double;
use graphviz_pure_core::cnum::{c_max, c_min, is_exactly_zero};
use graphviz_pure_core::math;
use graphviz_pure_core::{AttrMap, BoundingBox, Point};

/// `Bezier(V, t, Left, Right)`: de Casteljau evaluation and subdivision.
pub fn bezier_split(v: &[Point; 4], t: f64) -> (Point, [Point; 4], [Point; 4]) {
    let mut vt = [[Point::default(); 4]; 4];
    vt[0] = *v;
    for i in 1..=3 {
        for j in 0..=(3 - i) {
            // (1.0 - t) * a + t * b, contracted to fma(1 - t, a, t * b).
            vt[i][j].x = (1.0 - t).mul_add(vt[i - 1][j].x, t * vt[i - 1][j + 1].x);
            vt[i][j].y = (1.0 - t).mul_add(vt[i - 1][j].y, t * vt[i - 1][j + 1].y);
        }
    }
    let left = [vt[0][0], vt[1][0], vt[2][0], vt[3][0]];
    let right = [vt[3][0], vt[2][1], vt[1][2], vt[0][3]];
    (vt[3][0], left, right)
}

/// `Bezier(V, t, NULL, NULL)`.
pub fn bezier_pt(v: &[Point; 4], t: f64) -> Point {
    bezier_split(v, t).0
}

/// `bezier_clip`: binary search for where the curve crosses the boundary
/// described by `inside`. With `left_inside`, `sp[0]` is inside, else `sp[3]`.
pub fn bezier_clip(inside: &mut dyn FnMut(Point) -> bool, sp: &mut [Point; 4], left_inside: bool) {
    let mut best = [Point::default(); 4];
    let mut seg;
    let mut pt = if left_inside { sp[0] } else { sp[3] };
    let mut found = false;
    let mut low = 0.0f64;
    let mut high = 1.0f64;
    loop {
        let opt = pt;
        let t = (high + low) / 2.0;
        let (p, l, r) = bezier_split(sp, t);
        pt = p;
        seg = if left_inside { r } else { l };
        if inside(pt) {
            if left_inside {
                low = t;
            } else {
                high = t;
            }
            best = seg;
            found = true;
        } else if left_inside {
            high = t;
        } else {
            low = t;
        }
        if !((opt.x - pt.x).abs() > 0.5 || (opt.y - pt.y).abs() > 0.5) {
            break;
        }
    }
    *sp = if found { best } else { seg };
}

/// Which label of an edge (`ED_label`, `ED_head_label`, `ED_tail_label`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelKind {
    Main,
    Head,
    Tail,
    /// `ED_xlabel`.
    Xlabel,
}

/// Snapshot of a `textlabel_t` (`dimen`, `pos`, `set`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LabelView {
    pub dimen: Point,
    pub pos: Point,
    pub set: bool,
}

/// `splineInfo` flags of the caller (`ignoreSwap`, `isOrtho`); the callbacks
/// (`swapEnds`, `splineMerge`) are [`SplineGraph`] methods.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplineInfo {
    pub ignore_swap: bool,
    pub is_ortho: bool,
}

/// The layout state the shared spline code reads and writes: the `ND_*` /
/// `ED_*` / `GD_*` fields `splines.c` touches plus the `splineInfo`
/// callbacks. Implemented by the neato family's [`LGraph`] and by dot.
pub trait SplineGraph {
    fn tail(&self, e: usize) -> usize;
    fn head(&self, e: usize) -> usize;
    /// `ND_coord(n)`.
    fn coord(&self, n: usize) -> Point;
    fn lw(&self, n: usize) -> f64;
    fn rw(&self, n: usize) -> f64;
    fn ht(&self, n: usize) -> f64;
    /// `ND_shape(n)` when it has an `insidefn` (`None` for virtual nodes).
    fn shape(&self, n: usize) -> Option<&NodeShape>;
    /// `GD_rankdir(agraphof(n))`.
    fn rankdir(&self) -> i32 {
        0
    }
    /// `GD_flip(g)`.
    fn flip(&self) -> bool {
        false
    }
    /// `GD_ranksep(agraphof(n))`.
    fn ranksep(&self) -> f64 {
        0.0
    }
    fn rank(&self, _n: usize) -> i32 {
        0
    }
    fn order(&self, _n: usize) -> i32 {
        0
    }
    /// `ND_node_type(n) == NORMAL`.
    fn is_normal_node(&self, _n: usize) -> bool {
        true
    }
    /// `ED_to_orig(e)`.
    fn to_orig(&self, _e: usize) -> Option<usize> {
        None
    }
    /// `ED_edge_type(e) == NORMAL`.
    fn is_normal_edge(&self, _e: usize) -> bool {
        true
    }
    /// `ED_edge_type(e) == IGNORED`.
    fn is_ignored_edge(&self, _e: usize) -> bool {
        false
    }
    fn tail_port(&self, e: usize) -> &PortInfo;
    fn head_port(&self, e: usize) -> &PortInfo;
    fn tail_port_mut(&mut self, e: usize) -> &mut PortInfo;
    fn head_port_mut(&mut self, e: usize) -> &mut PortInfo;
    /// Attributes of a real edge (`agxget`).
    fn edge_attrs(&self, e: usize) -> &AttrMap;
    /// `arrow_flags(e, &sflag, &eflag)`.
    fn arrow_flags(&self, e: usize) -> (u32, u32);
    /// `splineInfo.swapEnds`.
    fn swap_ends(&self, _e: usize) -> bool {
        false
    }
    /// `splineInfo.splineMerge`.
    fn spline_merge(&self, _n: usize) -> bool {
        false
    }
    /// `ND_in(n)` / `ND_out(n)` of the fast graph (concentrator slopes).
    fn fast_in(&self, _n: usize) -> &[usize] {
        &[]
    }
    fn fast_out(&self, _n: usize) -> &[usize] {
        &[]
    }
    /// `ED_spl(e)`.
    fn spl(&self, e: usize) -> Option<&Vec<Bezier>>;
    fn spl_mut(&mut self, e: usize) -> &mut Option<Vec<Bezier>>;
    /// `GD_bb(g)` of the graph being routed.
    fn bb_mut(&mut self) -> &mut BoundingBox;
    fn label(&self, e: usize, k: LabelKind) -> Option<LabelView>;
    fn set_label_pos(&mut self, e: usize, k: LabelKind, pos: Point);
    /// `E_labelangle || E_labeldistance` (the attributes are declared).
    fn has_port_label_attrs(&self) -> bool;
    /// `resolvePort(n, other, oldport)` for ports with `dyna` set.
    fn resolve_port(&self, _n: usize, _other: usize, old: &PortInfo) -> PortInfo {
        old.clone()
    }
    /// `ND_shape(n)->fns->pboxfn(n, port, side, rv, &k)`: the side mask and the
    /// boxes from a port to the node border, if the shape has a `pboxfn`. The boxes
    /// are `None` where C returns a mask without writing `rv` or `k` (`record_path`
    /// finding no field for the port, #360).
    fn pboxfn(
        &self,
        _n: usize,
        _port: &PortInfo,
        _side: u8,
    ) -> Option<(u8, Option<Vec<BoundingBox>>)> {
        None
    }
}

/// The `inside_t` context for node `n` (`.s.n = n, .s.bp = bp`): the node's `ND_lw`,
/// `ND_rw` and `ND_ht` as the layout has them, and `poly_inside`'s scale from them.
pub fn inside_ctx<S: SplineGraph + ?Sized>(g: &S, n: usize, bp: Option<BoundingBox>) -> InsideCtx {
    let (lw, rw, ht) = (g.lw(n), g.rw(n), g.ht(n));
    let mut ctx = InsideCtx {
        last: 0,
        bp,
        rankdir: g.rankdir(),
        scale: None,
        extent: (lw, rw, ht),
    };
    if let Some(shape) = g.shape(n) {
        let (xsize, ysize) = if g.flip() {
            (ht, lw + rw)
        } else {
            (lw + rw, ht)
        };
        let sx = if is_exactly_zero(xsize) {
            shape.width
        } else {
            shape.width / xsize
        };
        let sy = if is_exactly_zero(ysize) {
            shape.height
        } else {
            shape.height / ysize
        };
        if sx != 1.0 || sy != 1.0 {
            ctx.scale = Some(Point::new(sx, sy));
        }
    }
    ctx
}

/// `shape_clip0`: clip `curve` (absolute coordinates) to node `n`.
fn shape_clip0<S: SplineGraph + ?Sized>(
    g: &S,
    ctx: &mut InsideCtx,
    n: usize,
    curve: &mut [Point],
    left_inside: bool,
) {
    let c0 = g.coord(n);
    let mut c = [Point::default(); 4];
    for i in 0..4 {
        c[i] = Point::new(curve[i].x - c0.x, curve[i].y - c0.y);
    }
    let shape = g.shape(n).expect("shape_clip0 needs an insidefn");
    bezier_clip(&mut |p| shape.inside(ctx, p), &mut c, left_inside);
    for i in 0..4 {
        curve[i] = Point::new(c[i].x + c0.x, c[i].y + c0.y);
    }
}

/// `shape_clip(n, curve)`: clips using `curve[0]` to decide which end is inside.
pub fn shape_clip<S: SplineGraph + ?Sized>(g: &S, n: usize, curve: &mut [Point; 4]) {
    let Some(shape) = g.shape(n) else { return };
    if shape.no_clip {
        return;
    }
    let mut ctx = inside_ctx(g, n, None);
    let c0 = g.coord(n);
    let left_inside = shape.inside(&mut ctx, Point::new(curve[0].x - c0.x, curve[0].y - c0.y));
    shape_clip0(g, &mut ctx, n, curve, left_inside);
}

/// `ED_to_orig` walk used by `new_spline`, `clip_and_install` and `beginpath`:
/// follow `to_orig` while the edge is not `NORMAL`.
pub fn orig_edge<S: SplineGraph + ?Sized>(g: &S, fe: usize) -> usize {
    let mut e = fe;
    while !g.is_normal_edge(e) {
        match g.to_orig(e) {
            Some(o) => e = o,
            None => break,
        }
    }
    e
}

fn has_insidefn<S: SplineGraph + ?Sized>(g: &S, n: usize) -> bool {
    g.shape(n).is_some_and(|s| !s.no_clip)
}

/// `arrow_clip(fe, hn, ps, &start, &end, spl, info)`.
fn arrow_clip<S: SplineGraph + ?Sized>(
    g: &S,
    fe: usize,
    hn: usize,
    ps: &mut [Point],
    start: &mut usize,
    end: &mut usize,
    spl: &mut Bezier,
    info: SplineInfo,
) {
    let mut e = fe;
    while let Some(o) = g.to_orig(e) {
        e = o;
    }
    let j = if info.ignore_swap {
        false
    } else {
        g.swap_ends(e)
    };
    let (mut sflag, mut eflag) = g.arrow_flags(e);
    if g.spline_merge(hn) {
        eflag = 0;
    }
    if g.spline_merge(g.tail(fe)) {
        sflag = 0;
    }
    if j {
        std::mem::swap(&mut sflag, &mut eflag);
    }
    let attrs = g.edge_attrs(e);
    if info.is_ortho {
        if eflag != 0 || sflag != 0 {
            super::arrows::arrow_ortho_clip(attrs, ps, *start, *end, spl, sflag, eflag);
        }
    } else {
        if sflag != 0 {
            *start = arrow_start_clip(attrs, ps, *start, *end, spl, sflag);
        }
        if eflag != 0 {
            *end = arrow_end_clip(attrs, ps, *start, *end, spl, eflag);
        }
    }
}

/// `clip_and_install(fe, hn, ps, pn, &sinfo)` with the default `splineInfo`
/// (`ignoreSwap = isOrtho = false`).
pub fn clip_and_install<S: SplineGraph + ?Sized>(g: &mut S, fe: usize, hn: usize, ps_in: &[Point]) {
    clip_and_install_info(g, fe, hn, ps_in, SplineInfo::default());
}

/// `clip_and_install(fe, hn, ps, pn, info)`: given a raw spline from the tail
/// of `fe` ending in node `hn`, clip the ends to the node boundaries and the
/// arrowheads, attach the result to the edge and grow the graph's bounding box.
pub fn clip_and_install_info<S: SplineGraph + ?Sized>(
    g: &mut S,
    fe: usize,
    hn_in: usize,
    ps_in: &[Point],
    info: SplineInfo,
) {
    let mut ps = ps_in.to_vec();
    let pn = ps.len();
    let mut tn = g.tail(fe);
    let mut hn = hn_in;
    let orig = orig_edge(g, fe);
    // may be a reversed flat edge
    if !info.ignore_swap && g.rank(tn) == g.rank(hn) && g.order(tn) > g.order(hn) {
        std::mem::swap(&mut hn, &mut tn);
    }
    let (clip_tail, clip_head, tbox, hbox) = if tn == g.tail(orig) {
        let (tp, hp) = (g.tail_port(orig), g.head_port(orig));
        (tp.clip, hp.clip, tp.bp, hp.bp)
    } else {
        let (tp, hp) = (g.tail_port(orig), g.head_port(orig));
        (hp.clip, tp.clip, hp.bp, tp.bp)
    };
    let mut start = 0usize;
    if clip_tail && has_insidefn(g, tn) {
        let mut ctx = inside_ctx(g, tn, tbox);
        let c = g.coord(tn);
        let shape = g.shape(tn).unwrap();
        while start < pn - 4 {
            let p2 = Point::new(ps[start + 3].x - c.x, ps[start + 3].y - c.y);
            if !shape.inside(&mut ctx, p2) {
                break;
            }
            start += 3;
        }
        shape_clip0(g, &mut ctx, tn, &mut ps[start..start + 4], true);
    }
    let mut end = pn - 4;
    if clip_head && has_insidefn(g, hn) {
        let mut ctx = inside_ctx(g, hn, hbox);
        let c = g.coord(hn);
        let shape = g.shape(hn).unwrap();
        while end > 0 {
            let p2 = Point::new(ps[end].x - c.x, ps[end].y - c.y);
            if !shape.inside(&mut ctx, p2) {
                break;
            }
            end -= 3;
        }
        shape_clip0(g, &mut ctx, hn, &mut ps[end..end + 4], false);
    }
    while start < pn - 4 {
        if !approxeqpt(ps[start], ps[start + 3], MILLIPOINT) {
            break;
        }
        start += 3;
    }
    while end > 0 {
        if !approxeqpt(ps[end], ps[end + 3], MILLIPOINT) {
            break;
        }
        end -= 3;
    }
    // When every point tests inside the end nodes (zero-size shapes, coincident ends), the
    // tail-side walks pass the head-side ones. One segment apart (`start == end + 3`), C
    // installs a one-point bezier and draws it, and so does the port. Further apart, C's
    // `size_t` `end - start + 4` wraps, and C asserts in `bezier_bb` (`size % 3 == 1`) or
    // crashes wherever the bezier is read (#359). The port keeps the one segment where the
    // walks meet (`start == end`), the last state in which C installs a whole segment:
    // every reader handles it, where a single point is a corner case for some.
    if start.saturating_sub(end) > 3 {
        start = end;
    }
    let mut newspl = Bezier::default();
    arrow_clip(g, fe, hn, &mut ps, &mut start, &mut end, &mut newspl, info);
    let mut list = Vec::with_capacity((end + 4).saturating_sub(start));
    let mut i = start;
    let bb = g.bb_mut();
    while i < end + 4 {
        let mut cp = [Point::default(); 4];
        list.push(ps[i]);
        cp[0] = ps[i];
        i += 1;
        if i >= end + 4 {
            break;
        }
        list.push(ps[i]);
        cp[1] = ps[i];
        i += 1;
        list.push(ps[i]);
        cp[2] = ps[i];
        i += 1;
        cp[3] = ps[i];
        update_bb_bz(bb, &cp);
    }
    newspl.list = list;
    new_spline(g, fe, newspl);
}

/// `new_spline(e, sz)`: attach `bz` to the edge reached through `ED_to_orig`.
pub fn new_spline<S: SplineGraph + ?Sized>(g: &mut S, e: usize, bz: Bezier) {
    let mut e = e;
    while let Some(o) = g.to_orig(e) {
        if g.is_normal_edge(e) {
            break;
        }
        e = o;
    }
    g.spl_mut(e).get_or_insert_with(Vec::new).push(bz);
}

/// `conc_slope(n)`: mean slope through a concentrator node.
pub fn conc_slope<S: SplineGraph + ?Sized>(g: &S, n: usize) -> f64 {
    let mut s_in = 0.0;
    let mut s_out = 0.0;
    let ins = g.fast_in(n);
    let outs = g.fast_out(n);
    for &e in ins {
        s_in += g.coord(g.tail(e)).x;
    }
    for &e in outs {
        s_out += g.coord(g.head(e)).x;
    }
    let c = g.coord(n);
    let x1 = c.x - s_in / ins.len() as f64;
    let y1 = c.y - g.coord(g.tail(ins[0])).y;
    let m_in = math::atan2(y1, x1);
    let x2 = s_out / outs.len() as f64 - c.x;
    let y2 = g.coord(g.head(outs[0])).y - c.y;
    let m_out = math::atan2(y2, x2);
    (m_in + m_out) / 2.0
}

/// `getsplinepoints(e)`: the splines of `e` or of the edge it stands for.
pub fn getsplinepoints<S: SplineGraph + ?Sized>(g: &S, e: usize) -> Option<&Vec<Bezier>> {
    let mut le = e;
    loop {
        if let Some(sp) = g.spl(le) {
            return Some(sp);
        }
        if g.is_normal_edge(le) {
            return None;
        }
        le = g.to_orig(le)?;
    }
}

/// Edge kinds for `beginpath`/`endpath` (`REGULAREDGE`, `FLATEDGE`, `SELFEDGE`).
pub const REGULAREDGE: i32 = 1;
pub const FLATEDGE: i32 = 2;
pub const SELFWPEDGE: i32 = 4;
pub const SELFNPEDGE: i32 = 8;
pub const SELFEDGE: i32 = 8;
pub const EDGETYPEMASK: i32 = 15;

fn ht2<S: SplineGraph + ?Sized>(g: &S, n: usize) -> f64 {
    g.ht(n) / 2.0
}

fn clear_orig_clip<S: SplineGraph + ?Sized>(g: &mut S, e: usize, n: usize, at_tail: bool) {
    let orig = orig_edge(g, e);
    let on_tail = if at_tail {
        n == g.tail(orig)
    } else {
        n != g.head(orig)
    };
    if on_tail {
        g.tail_port_mut(orig).clip = false;
    } else {
        g.head_port_mut(orig).clip = false;
    }
}

/// `if (pboxfn && (mask = pboxfn(n, port, side, &endp->boxes[0], &endp->boxn)))
/// endp->sidemask = mask;` in `beginpath` and `endpath`: the side mask and the
/// boxes `endp` ends up with, or `None` where the caller uses the node box.
///
/// Where `pboxfn` returns a mask but writes no box (`record_path` finding no
/// field for the port, #360), C's `endp` keeps the boxes it held. The caller
/// says what those are in `kept`; with `None` the node box is used.
fn port_boxes(
    r: Option<(u8, Option<Vec<BoundingBox>>)>,
    kept: Option<Vec<BoundingBox>>,
) -> Option<(u8, Vec<BoundingBox>)> {
    match r {
        Some((mask, Some(boxes))) if mask != 0 => Some((mask, boxes)),
        Some((mask, None)) if mask != 0 => kept.map(|b| (mask, b)),
        _ => None,
    }
}

/// `beginpath(P, e, et, endp, merge)`: set up the boxes near the tail node. `kept` is
/// what C's `endp->boxes` hold on entry, used where `pboxfn` writes none (see
/// `port_boxes`); `None` uses the node box there.
pub fn beginpath<S: SplineGraph + ?Sized>(
    g: &mut S,
    p: &mut Path,
    e: usize,
    et: i32,
    endp: &mut PathEnd,
    merge: bool,
    kept: Option<Vec<BoundingBox>>,
) {
    let n = g.tail(e);
    if g.tail_port(e).dyna {
        let np = g.resolve_port(n, g.head(e), g.tail_port(e));
        *g.tail_port_mut(e) = np;
    }
    let tport = g.tail_port(e).clone();
    let nc = g.coord(n);
    p.start.p = add(nc, tport.p);
    if merge {
        p.start.theta = conc_slope(g, g.tail(e));
        p.start.constrained = true;
    } else if tport.constrained {
        p.start.theta = tport.theta;
        p.start.constrained = true;
    } else {
        p.start.constrained = false;
    }
    p.boxes.clear();
    p.data = e;
    endp.np = p.start.p;
    let side = tport.side;
    // GD_ranksep is an int: GD_ranksep(g)/2 truncates.
    let half_ranksep = f64::from(g.ranksep() as i32 / 2);
    let (lw, rw) = (g.lw(n), g.rw(n));
    let h2 = ht2(g, n);
    if et == REGULAREDGE && g.is_normal_node(n) && side != 0 {
        let mut b = endp.nb;
        let mut b0 = BoundingBox::default();
        if side & TOP != 0 {
            endp.sidemask = TOP;
            if p.start.p.x < nc.x {
                b0.ll.x = b.ll.x - 1.0;
                b0.ll.y = p.start.p.y;
                b0.ur.x = b.ur.x;
                b0.ur.y = nc.y + h2 + half_ranksep;
                b.ur.x = nc.x - lw - (FUDGE - 2.0);
                b.ur.y = b0.ll.y;
                b.ll.y = nc.y - h2;
                b.ll.x -= 1.0;
            } else {
                b0.ll.x = b.ll.x;
                b0.ll.y = p.start.p.y;
                b0.ur.x = b.ur.x + 1.0;
                b0.ur.y = nc.y + h2 + half_ranksep;
                b.ll.x = nc.x + rw + (FUDGE - 2.0);
                b.ur.y = b0.ll.y;
                b.ll.y = nc.y - h2;
                b.ur.x += 1.0;
            }
            endp.boxes = vec![b0, b];
            p.start.p.y += 1.0;
        } else if side & BOTTOM != 0 {
            endp.sidemask = BOTTOM;
            b.ur.y = c_max(b.ur.y, p.start.p.y);
            endp.boxes = vec![b];
            p.start.p.y -= 1.0;
        } else if side & LEFT != 0 {
            endp.sidemask = LEFT;
            b.ur.x = p.start.p.x;
            b.ll.y = nc.y - h2;
            b.ur.y = p.start.p.y;
            endp.boxes = vec![b];
            p.start.p.x -= 1.0;
        } else {
            endp.sidemask = RIGHT;
            b.ll.x = p.start.p.x;
            b.ll.y = nc.y - h2;
            b.ur.y = p.start.p.y;
            endp.boxes = vec![b];
            p.start.p.x += 1.0;
        }
        clear_orig_clip(g, e, n, true);
        return;
    }
    if et == FLATEDGE && side != 0 {
        let mut b = endp.nb;
        let mut b0 = BoundingBox::default();
        if side & TOP != 0 {
            b.ll.y = c_min(b.ll.y, p.start.p.y);
            endp.boxes = vec![b];
            p.start.p.y += 1.0;
        } else if side & BOTTOM != 0 {
            if endp.sidemask == TOP {
                b0.ur.y = nc.y - h2;
                b0.ur.x = b.ur.x + 1.0;
                b0.ll.x = p.start.p.x;
                b0.ll.y = b0.ur.y - half_ranksep;
                b.ll.x = nc.x + rw + (FUDGE - 2.0);
                b.ll.y = b0.ur.y;
                b.ur.y = nc.y + h2;
                b.ur.x += 1.0;
                endp.boxes = vec![b0, b];
            } else {
                b.ur.y = c_max(b.ur.y, p.start.p.y);
                endp.boxes = vec![b];
            }
            p.start.p.y -= 1.0;
        } else if side & LEFT != 0 {
            b.ur.x = p.start.p.x + 1.0;
            if endp.sidemask == TOP {
                b.ur.y = nc.y + h2;
                b.ll.y = p.start.p.y - 1.0;
            } else {
                b.ll.y = nc.y - h2;
                b.ur.y = p.start.p.y + 1.0;
            }
            endp.boxes = vec![b];
            p.start.p.x -= 1.0;
        } else {
            b.ll.x = p.start.p.x;
            if endp.sidemask == TOP {
                b.ur.y = nc.y + h2;
                b.ll.y = p.start.p.y;
            } else {
                b.ll.y = nc.y - h2;
                b.ur.y = p.start.p.y + 1.0;
            }
            endp.boxes = vec![b];
            p.start.p.x += 1.0;
        }
        clear_orig_clip(g, e, n, true);
        endp.sidemask = side;
        return;
    }
    let side = if et == REGULAREDGE {
        BOTTOM
    } else {
        endp.sidemask
    };
    if let Some((mask, boxes)) = port_boxes(g.pboxfn(n, &tport, side), kept) {
        endp.boxes = boxes;
        endp.sidemask = mask;
    } else {
        endp.boxes = vec![endp.nb];
        match et {
            SELFEDGE => unreachable!("beginpath is not used for self edges"),
            FLATEDGE => {
                if endp.sidemask == TOP {
                    endp.boxes[0].ll.y = p.start.p.y;
                } else {
                    endp.boxes[0].ur.y = p.start.p.y;
                }
            }
            REGULAREDGE => {
                endp.boxes[0].ur.y = p.start.p.y;
                endp.sidemask = BOTTOM;
                p.start.p.y -= 1.0;
            }
            _ => {}
        }
    }
}

/// `endpath(P, e, et, endp, merge)`: set up the boxes near the head node. `kept` is
/// what C's `endp->boxes` hold on entry, used where `pboxfn` writes none (see
/// `port_boxes`); `None` uses the node box there.
pub fn endpath<S: SplineGraph + ?Sized>(
    g: &mut S,
    p: &mut Path,
    e: usize,
    et: i32,
    endp: &mut PathEnd,
    merge: bool,
    kept: Option<Vec<BoundingBox>>,
) {
    let n = g.head(e);
    if g.head_port(e).dyna {
        let np = g.resolve_port(n, g.tail(e), g.head_port(e));
        *g.head_port_mut(e) = np;
    }
    let hport = g.head_port(e).clone();
    let nc = g.coord(n);
    p.end.p = add(nc, hport.p);
    if merge {
        p.end.theta = conc_slope(g, g.head(e)) + std::f64::consts::PI;
        p.end.constrained = true;
    } else if hport.constrained {
        p.end.theta = hport.theta;
        p.end.constrained = true;
    } else {
        p.end.constrained = false;
    }
    endp.np = p.end.p;
    let side = hport.side;
    // GD_ranksep is an int: GD_ranksep(g)/2 truncates.
    let half_ranksep = f64::from(g.ranksep() as i32 / 2);
    let (lw, rw) = (g.lw(n), g.rw(n));
    let h2 = ht2(g, n);
    if et == REGULAREDGE && g.is_normal_node(n) && side != 0 {
        let mut b = endp.nb;
        let mut b0 = BoundingBox::default();
        if side & TOP != 0 {
            endp.sidemask = TOP;
            b.ll.y = c_min(b.ll.y, p.end.p.y);
            endp.boxes = vec![b];
            p.end.p.y += 1.0;
        } else if side & BOTTOM != 0 {
            endp.sidemask = BOTTOM;
            if p.end.p.x < nc.x {
                b0.ll.x = b.ll.x - 1.0;
                b0.ur.y = p.end.p.y;
                b0.ur.x = b.ur.x;
                b0.ll.y = nc.y - h2 - half_ranksep;
                b.ur.x = nc.x - lw - (FUDGE - 2.0);
                b.ll.y = b0.ur.y;
                b.ur.y = nc.y + h2;
                b.ll.x -= 1.0;
            } else {
                b0.ll.x = b.ll.x;
                b0.ur.y = p.end.p.y;
                b0.ur.x = b.ur.x + 1.0;
                b0.ll.y = nc.y - h2 - half_ranksep;
                b.ll.x = nc.x + rw + (FUDGE - 2.0);
                b.ll.y = b0.ur.y;
                b.ur.y = nc.y + h2;
                b.ur.x += 1.0;
            }
            endp.boxes = vec![b0, b];
            p.end.p.y -= 1.0;
        } else if side & LEFT != 0 {
            endp.sidemask = LEFT;
            b.ur.x = p.end.p.x;
            b.ur.y = nc.y + h2;
            b.ll.y = p.end.p.y;
            endp.boxes = vec![b];
            p.end.p.x -= 1.0;
        } else {
            endp.sidemask = RIGHT;
            b.ll.x = p.end.p.x;
            b.ur.y = nc.y + h2;
            b.ll.y = p.end.p.y;
            endp.boxes = vec![b];
            p.end.p.x += 1.0;
        }
        clear_orig_clip(g, e, n, false);
        endp.sidemask = side;
        return;
    }
    if et == FLATEDGE && side != 0 {
        let mut b = endp.nb;
        let mut b0 = BoundingBox::default();
        if side & TOP != 0 {
            b.ll.y = c_min(b.ll.y, p.end.p.y);
            endp.boxes = vec![b];
            p.end.p.y += 1.0;
        } else if side & BOTTOM != 0 {
            if endp.sidemask == TOP {
                b0.ll.x = b.ll.x - 1.0;
                b0.ur.y = nc.y - h2;
                b0.ur.x = p.end.p.x;
                b0.ll.y = b0.ur.y - half_ranksep;
                b.ur.x = nc.x - lw - 2.0;
                b.ll.y = b0.ur.y;
                b.ur.y = nc.y + h2;
                b.ll.x -= 1.0;
                endp.boxes = vec![b0, b];
            } else {
                b.ur.y = c_max(b.ur.y, p.start.p.y);
                endp.boxes = vec![b];
            }
            p.end.p.y -= 1.0;
        } else if side & LEFT != 0 {
            b.ur.x = p.end.p.x + 1.0;
            if endp.sidemask == TOP {
                b.ur.y = nc.y + h2;
                b.ll.y = p.end.p.y - 1.0;
            } else {
                b.ll.y = nc.y - h2;
                b.ur.y = p.end.p.y + 1.0;
            }
            endp.boxes = vec![b];
            p.end.p.x -= 1.0;
        } else {
            b.ll.x = p.end.p.x - 1.0;
            if endp.sidemask == TOP {
                b.ur.y = nc.y + h2;
                b.ll.y = p.end.p.y - 1.0;
            } else {
                b.ll.y = nc.y - h2;
                b.ur.y = p.end.p.y;
            }
            endp.boxes = vec![b];
            p.end.p.x += 1.0;
        }
        clear_orig_clip(g, e, n, false);
        endp.sidemask = side;
        return;
    }
    let side = if et == REGULAREDGE {
        TOP
    } else {
        endp.sidemask
    };
    if let Some((mask, boxes)) = port_boxes(g.pboxfn(n, &hport, side), kept) {
        endp.boxes = boxes;
        endp.sidemask = mask;
    } else {
        endp.boxes = vec![endp.nb];
        match et {
            SELFEDGE => unreachable!("endpath is not used for self edges"),
            FLATEDGE => {
                if endp.sidemask == TOP {
                    endp.boxes[0].ll.y = p.end.p.y;
                } else {
                    endp.boxes[0].ur.y = p.end.p.y;
                }
            }
            REGULAREDGE => {
                endp.boxes[0].ll.y = p.end.p.y;
                endp.sidemask = TOP;
                p.end.p.y += 1.0;
            }
            _ => {}
        }
    }
}

/// `FUDGE` in `beginpath`/`endpath`.
const FUDGE: f64 = 2.0;

const HW: f64 = 2.0;
const SMALL: f64 = 0.000_000_000_1;

/// `ptToLine2`: squared distance from `p` to the line through `a` and `b`.
pub fn pt_to_line2(a: Point, b: Point, p: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut a2 = (p.y - a.y).mul_add(dx, -((p.x - a.x) * dy));
    a2 *= a2;
    if a2 < SMALL {
        return 0.0;
    }
    a2 / dx.mul_add(dx, dy * dy)
}

fn check_control_points(cp: &[Point; 4]) -> bool {
    let d1 = pt_to_line2(cp[0], cp[3], cp[1]);
    let d2 = pt_to_line2(cp[0], cp[3], cp[2]);
    d1 < HW * HW && d2 < HW * HW
}

/// `update_bb_bz`: grow `bb` to contain a Bézier segment.
pub fn update_bb_bz(bb: &mut BoundingBox, cp: &[Point; 4]) {
    BZ_OUT_OF_SPLITS.with(|out| {
        let mut budget = if out.get() { 0 } else { BZ_SPLIT_BUDGET };
        if update_bb_bz_r(bb, cp, &mut budget) {
            out.set(true);
        }
    });
}

/// Not in C: the most times [`update_bb_bz`] splits one segment (#227), about 5 s of C's
/// work. C refines a segment until its control points are within 2pt of its chord, which
/// takes about `sqrt(length)` splits at a given magnitude, and more as the coordinates'
/// rounding nears 2pt: 1.3e5 for pins 1e10 inches apart, 5.7e7 at 1.5e15. Further out the
/// 2pt test falls below the rounding and C never finishes (#227: curved edges between pins
/// 1e16 inches apart). The count grows continuously, so this is a cutoff, as #177's packing
/// budget is: a segment under it is refined as in C; past it, its remaining pieces are taken
/// whole, as `check_control_points` accepts a piece, and their control points still enclose
/// the curve. A segment C finishes but that needs more (seconds of C per segment) can then
/// give a bb a little larger than C's.
const BZ_SPLIT_BUDGET: u64 = 1 << 28;

thread_local! {
    /// Set once a segment of this layout ran out of [`BZ_SPLIT_BUDGET`]: the later segments
    /// are taken whole, so many such segments cannot add up to a hang.
    static BZ_OUT_OF_SPLITS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clears [`BZ_OUT_OF_SPLITS`] for the next layout on this thread. Called by
/// `layout_graph_with_options`, the entry point of every engine.
pub(crate) fn reset_bz_split_budget() {
    BZ_OUT_OF_SPLITS.with(|out| out.set(false));
}

/// Not in C: the deepest [`update_bb_bz`] refines one piece (#269). C recurses on both
/// halves, left first, with no depth bound. At about 1e300 points the pieces stop shrinking
/// (the left half of a piece's right half rounds back to the piece) while `ptToLine2`
/// overflows to NaN, so `check_control_points` never accepts them and C recurses until its
/// stack overflows (`ratio` or `ranksep` at 1e300). A piece stops shrinking after about 2100
/// halvings (the double exponent range and 53 bits), so a deeper one is such a cycle, which C
/// never finishes whatever its stack. The bound is also above C's stack: C's frame is 176
/// bytes (Homebrew 16.1.0, arm64), so 8 MiB holds about 47,600 levels. A piece this deep ends
/// the segment as running out of [`BZ_SPLIT_BUDGET`] does.
const BZ_MAX_DEPTH: usize = 1 << 16;

/// `update_bb_bz`'s recursion, on an explicit stack in C's order (#269): a piece's left half,
/// then its right half. Returns whether a piece needed a split the budget refused.
fn update_bb_bz_r(bb: &mut BoundingBox, cp: &[Point; 4], budget: &mut u64) -> bool {
    let mut refused = false;
    let mut stack = vec![(*cp, 0_usize)];
    while let Some((cp, depth)) = stack.pop() {
        let out = cp
            .iter()
            .any(|p| p.x > bb.ur.x || p.x < bb.ll.x || p.y > bb.ur.y || p.y < bb.ll.y);
        if !out {
            continue;
        }
        // C has no finiteness test. Every half `Bezier` splits off keeps the curve's midpoint,
        // which is non-finite when any control point is, and `ptToLine2` gives inf or NaN for
        // such a point, so `check_control_points` never accepts the segment: C either stops on
        // a half that is not `out`, leaving `bb` as it was, or recurses until its stack
        // overflows (#173: osage gives a repeated cluster a NaN box, and its edges inherit it;
        // #201: neato pins a node at NaN). Leave `bb` as it was.
        if !cp.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            continue;
        }
        if !check_control_points(&cp) {
            if depth >= BZ_MAX_DEPTH {
                // Where C's stack overflows: take this piece and the rest whole.
                *budget = 0;
            }
            if let Some(left) = budget.checked_sub(1) {
                *budget = left;
                let (_, l, r) = bezier_split(&cp, 0.5);
                let depth = depth.saturating_add(1);
                stack.push((r, depth));
                stack.push((l, depth));
                continue;
            }
            refused = true;
        }
        for p in &cp {
            if p.x > bb.ur.x {
                bb.ur.x = p.x;
            } else if p.x < bb.ll.x {
                bb.ll.x = p.x;
            }
            if p.y > bb.ur.y {
                bb.ur.y = p.y;
            } else if p.y < bb.ll.y {
                bb.ll.y = p.y;
            }
        }
    }
    refused
}

/// `addLabelBB` / `updateBB`: grow `bb` by a positioned label.
pub fn add_label_bb(bb: &mut BoundingBox, lp: &TextLabel, flip: bool) {
    let (w, h) = if flip {
        (lp.dimen.y, lp.dimen.x)
    } else {
        (lp.dimen.x, lp.dimen.y)
    };
    let p = lp.pos;
    let min = p.x - w / 2.0;
    let max = p.x + w / 2.0;
    if min < bb.ll.x {
        bb.ll.x = min;
    }
    if max > bb.ur.x {
        bb.ur.x = max;
    }
    let min = p.y - h / 2.0;
    let max = p.y + h / 2.0;
    if min < bb.ll.y {
        bb.ll.y = min;
    }
    if max > bb.ur.y {
        bb.ur.y = max;
    }
}

/// `addLabelBB` with explicit `dimen`/`pos`.
pub fn add_label_bb_dims(bb: &mut BoundingBox, dimen: Point, p: Point, flip: bool) {
    let (w, h) = if flip {
        (dimen.y, dimen.x)
    } else {
        (dimen.x, dimen.y)
    };
    let min = p.x - w / 2.0;
    let max = p.x + w / 2.0;
    if min < bb.ll.x {
        bb.ll.x = min;
    }
    if max > bb.ur.x {
        bb.ur.x = max;
    }
    let min = p.y - h / 2.0;
    let max = p.y + h / 2.0;
    if min < bb.ll.y {
        bb.ll.y = min;
    }
    if max > bb.ur.y {
        bb.ur.y = max;
    }
}

/// `updateBB(g, lp)` for label `k` of edge `e`.
pub fn update_bb_label<S: SplineGraph + ?Sized>(g: &mut S, e: usize, k: LabelKind) {
    if let Some(l) = g.label(e, k) {
        let flip = g.flip();
        add_label_bb_dims(g.bb_mut(), l.dimen, l.pos, flip);
    }
}

fn convert_sides_to_points(tail_side: u8, head_side: u8) -> i32 {
    const VERTICES: [u8; 8] = [12, 4, 6, 2, 3, 1, 9, 8];
    let ti = VERTICES.iter().position(|&v| v == tail_side);
    let hi = VERTICES.iter().position(|&v| v == head_side);
    match (ti, hi) {
        (Some(t), Some(h)) => ((t + 1) * 10 + h + 1) as i32,
        _ => 0,
    }
}

/// Label extent along x (`width`) or y (`height`), honouring `GD_flip`.
fn label_extent<S: SplineGraph + ?Sized>(g: &S, e: usize, along_x: bool) -> Option<f64> {
    let l = g.label(e, LabelKind::Main)?;
    Some(if along_x != g.flip() {
        l.dimen.x
    } else {
        l.dimen.y
    })
}

struct SelfEnds {
    n: usize,
    np: Point,
    tp: Point,
    hp: Point,
    pp: i32,
}

fn self_ends<S: SplineGraph + ?Sized>(g: &S, e: usize) -> SelfEnds {
    let n = g.tail(e);
    let np = g.coord(n);
    let tp = add(g.tail_port(e).p, np);
    let hp = add(g.head_port(e).p, np);
    let pp = convert_sides_to_points(g.tail_port(e).side, g.head_port(e).side);
    SelfEnds { n, np, tp, hp, pp }
}

fn self_right<S: SplineGraph + ?Sized>(g: &mut S, edges: &[usize], stepx: f64, sizey: f64) {
    let SelfEnds { n, np, tp, hp, pp } = self_ends(g, edges[0]);
    let cnt = edges.len();
    let stepy = (sizey / 2.0 / cnt as f64).max(2.0);
    let mut sgn = if tp.y >= hp.y { 1.0 } else { -1.0 };
    let mut dx = g.rw(n);
    let mut dy = 0.0;
    if (pp == 32 || pp == 65) && tp.y == hp.y {
        sgn = -sgn;
    }
    let mut tx = dx.min(3.0 * (np.x + dx - tp.x));
    let mut hx = dx.min(3.0 * (np.x + dx - hp.x));
    for &e in edges {
        dx += stepx;
        tx += stepx;
        hx += stepx;
        dy += sgn * stepy;
        let points = [
            tp,
            Point::new(tp.x + tx / 3.0, tp.y + dy),
            Point::new(np.x + dx, tp.y + dy),
            Point::new(np.x + dx, (tp.y + hp.y) / 2.0),
            Point::new(np.x + dx, hp.y - dy),
            Point::new(hp.x + hx / 3.0, hp.y - dy),
            hp,
        ];
        if let Some(width) = label_extent(g, e, true) {
            let c = g.coord(n);
            g.set_label_pos(e, LabelKind::Main, Point::new(c.x + dx + width / 2.0, c.y));
            if width > stepx {
                dx += width - stepx;
            }
        }
        let h = g.head(e);
        clip_and_install(g, e, h, &points);
    }
}

fn self_left<S: SplineGraph + ?Sized>(g: &mut S, edges: &[usize], stepx: f64, sizey: f64) {
    let SelfEnds { n, np, tp, hp, pp } = self_ends(g, edges[0]);
    let cnt = edges.len();
    let stepy = (sizey / 2.0 / cnt as f64).max(2.0);
    let mut sgn = if tp.y >= hp.y { 1.0 } else { -1.0 };
    let mut dx = g.lw(n);
    let mut dy = 0.0;
    if (pp == 12 || pp == 67) && tp.y == hp.y {
        sgn = -sgn;
    }
    let mut tx = dx.min(3.0 * (tp.x + dx - np.x));
    let mut hx = dx.min(3.0 * (hp.x + dx - np.x));
    for &e in edges {
        dx += stepx;
        tx += stepx;
        hx += stepx;
        dy += sgn * stepy;
        let points = [
            tp,
            Point::new(tp.x - tx / 3.0, tp.y + dy),
            Point::new(np.x - dx, tp.y + dy),
            Point::new(np.x - dx, (tp.y + hp.y) / 2.0),
            Point::new(np.x - dx, hp.y - dy),
            Point::new(hp.x - hx / 3.0, hp.y - dy),
            hp,
        ];
        if let Some(width) = label_extent(g, e, true) {
            let c = g.coord(n);
            g.set_label_pos(e, LabelKind::Main, Point::new(c.x - dx - width / 2.0, c.y));
            if width > stepx {
                dx += width - stepx;
            }
        }
        let h = g.head(e);
        clip_and_install(g, e, h, &points);
    }
}

fn self_top<S: SplineGraph + ?Sized>(g: &mut S, edges: &[usize], sizex: f64, stepy: f64) {
    let SelfEnds { n, np, tp, hp, pp } = self_ends(g, edges[0]);
    let cnt = edges.len();
    let stepx = (sizex / 2.0 / cnt as f64).max(2.0);
    let sgn = if tp.x >= hp.x { 1.0 } else { -1.0 };
    let (lw, rw) = (g.lw(n), g.rw(n));
    let mut dy = g.ht(n) / 2.0;
    let mut dx = 0.0;
    match pp {
        15 => dx = sgn * (rw - (hp.x - np.x) + stepx),
        38 => dx = sgn * (lw - (np.x - hp.x) + stepx),
        41 | 48 => dx = sgn * (rw - (tp.x - np.x) + stepx),
        14 | 37 | 47 | 51 | 57 | 58 => {
            dx = sgn * ((lw - (np.x - tp.x) + (rw - (hp.x - np.x))) / 3.0)
        }
        73 => dx = sgn * (lw - (np.x - tp.x) + stepx),
        83 => dx = sgn * (lw - (np.x - tp.x)),
        84 => dx = sgn * ((lw - (np.x - tp.x) + (rw - (hp.x - np.x))) / 2.0 + stepx),
        74 | 75 | 85 => {
            dx = sgn * ((lw - (np.x - tp.x) + (rw - (hp.x - np.x))) / 2.0 + 2.0 * stepx)
        }
        _ => {}
    }
    let mut ty = dy.min(3.0 * (np.y + dy - tp.y));
    let mut hy = dy.min(3.0 * (np.y + dy - hp.y));
    for &e in edges {
        dy += stepy;
        ty += stepy;
        hy += stepy;
        dx += sgn * stepx;
        let points = [
            tp,
            Point::new(tp.x + dx, tp.y + ty / 3.0),
            Point::new(tp.x + dx, np.y + dy),
            Point::new((tp.x + hp.x) / 2.0, np.y + dy),
            Point::new(hp.x - dx, np.y + dy),
            Point::new(hp.x - dx, hp.y + hy / 3.0),
            hp,
        ];
        if let Some(height) = label_extent(g, e, false) {
            let c = g.coord(n);
            g.set_label_pos(e, LabelKind::Main, Point::new(c.x, c.y + dy + height / 2.0));
            if height > stepy {
                dy += height - stepy;
            }
        }
        let h = g.head(e);
        clip_and_install(g, e, h, &points);
    }
}

fn self_bottom<S: SplineGraph + ?Sized>(g: &mut S, edges: &[usize], sizex: f64, stepy: f64) {
    let SelfEnds { n, np, tp, hp, pp } = self_ends(g, edges[0]);
    let cnt = edges.len();
    let stepx = (sizex / 2.0 / cnt as f64).max(2.0);
    let mut sgn = if tp.x >= hp.x { 1.0 } else { -1.0 };
    let mut dy = g.ht(n) / 2.0;
    let mut dx = 0.0;
    if pp == 67 {
        sgn = -sgn;
    }
    let mut ty = dy.min(3.0 * (tp.y + dy - np.y));
    let mut hy = dy.min(3.0 * (hp.y + dy - np.y));
    for &e in edges {
        dy += stepy;
        ty += stepy;
        hy += stepy;
        dx += sgn * stepx;
        let points = [
            tp,
            Point::new(tp.x + dx, tp.y - ty / 3.0),
            Point::new(tp.x + dx, np.y - dy),
            Point::new((tp.x + hp.x) / 2.0, np.y - dy),
            Point::new(hp.x - dx, np.y - dy),
            Point::new(hp.x - dx, hp.y - hy / 3.0),
            hp,
        ];
        if let Some(height) = label_extent(g, e, false) {
            let c = g.coord(n);
            g.set_label_pos(e, LabelKind::Main, Point::new(c.x, c.y - dy - height / 2.0));
            if height > stepy {
                dy += height - stepy;
            }
        }
        let h = g.head(e);
        clip_and_install(g, e, h, &points);
    }
}

/// `makeSelfEdge(edges, cnt, sizex, sizey, sinfo)`.
pub fn make_self_edge<S: SplineGraph + ?Sized>(g: &mut S, edges: &[usize], sizex: f64, sizey: f64) {
    let e = edges[0];
    let tp = g.tail_port(e);
    let hp = g.head_port(e);
    let (ts, hs) = (tp.side, hp.side);
    if (!tp.defined && !hp.defined)
        || (ts & LEFT == 0 && hs & LEFT == 0 && (ts != hs || ts & (TOP | BOTTOM) == 0))
    {
        self_right(g, edges, sizex, sizey);
    } else if ts & LEFT != 0 || hs & LEFT != 0 {
        if ts & RIGHT != 0 || hs & RIGHT != 0 {
            self_top(g, edges, sizex, sizey);
        } else {
            self_left(g, edges, sizex, sizey);
        }
    } else if ts & TOP != 0 {
        self_top(g, edges, sizex, sizey);
    } else if ts & BOTTOM != 0 {
        self_bottom(g, edges, sizex, sizey);
    }
}

pub const PORT_LABEL_DISTANCE: f64 = 10.0;
pub const PORT_LABEL_ANGLE: f64 = -25.0;

/// `place_portlabel(e, head_p)`; returns whether the label was placed.
pub fn place_portlabel<S: SplineGraph + ?Sized>(g: &mut S, e: usize, head_p: bool) -> bool {
    if g.is_ignored_edge(e) {
        return false;
    }
    let attrs = g.edge_attrs(e);
    let la = attrs.get("labelangle").filter(|s| !s.is_empty());
    let ld = attrs.get("labeldistance").filter(|s| !s.is_empty());
    if la.is_none() && ld.is_none() {
        return false;
    }
    let angle_attr = late_double(attrs, "labelangle", PORT_LABEL_ANGLE, -180.0);
    let dist_attr = late_double(attrs, "labeldistance", 1.0, 0.0);
    let Some(spl) = getsplinepoints(g, e) else {
        return false;
    };
    let (pe, pf) = if !head_p {
        let bez = &spl[0];
        if bez.sflag != 0 {
            (bez.sp, bez.list[0])
        } else {
            let c = [bez.list[0], bez.list[1], bez.list[2], bez.list[3]];
            (bez.list[0], bezier_pt(&c, 0.1))
        }
    } else {
        let bez = &spl[spl.len() - 1];
        let n = bez.list.len();
        if bez.eflag != 0 {
            (bez.ep, bez.list[n - 1])
        } else {
            let c = [
                bez.list[n - 4],
                bez.list[n - 3],
                bez.list[n - 2],
                bez.list[n - 1],
            ];
            (bez.list[n - 1], bezier_pt(&c, 0.9))
        }
    };
    // atan2(..) + RADIANS(a), with RADIANS(d) = d/180.0 * M_PI fused into the sum
    let angle =
        (angle_attr / 180.0).mul_add(std::f64::consts::PI, math::atan2(pf.y - pe.y, pf.x - pe.x));
    let d = PORT_LABEL_DISTANCE * dist_attr;
    let pos = Point::new(
        d.mul_add(math::cos(angle), pe.x),
        d.mul_add(math::sin(angle), pe.y),
    );
    let k = if head_p {
        LabelKind::Head
    } else {
        LabelKind::Tail
    };
    g.set_label_pos(e, k, pos);
    true
}

/// `makePortLabels(e)`: place head/tail labels when `labelangle` or
/// `labeldistance` is declared, growing the root bounding box.
pub fn make_port_labels<S: SplineGraph + ?Sized>(g: &mut S, e: usize) {
    if !g.has_port_label_attrs() {
        return;
    }
    if g.label(e, LabelKind::Head).is_some_and(|l| !l.set) && place_portlabel(g, e, true) {
        update_bb_label(g, e, LabelKind::Head);
    }
    if g.label(e, LabelKind::Tail).is_some_and(|l| !l.set) && place_portlabel(g, e, false) {
        update_bb_label(g, e, LabelKind::Tail);
    }
}

/// `addEdgeLabels(e)`.
pub fn add_edge_labels<S: SplineGraph + ?Sized>(g: &mut S, e: usize) {
    make_port_labels(g, e);
}

impl crate::ortho::OrthoGraph for LGraph<'_> {
    fn ortho_nodes(&self) -> Vec<usize> {
        (0..self.nodes.len()).collect()
    }
    fn ortho_out_edges(&self, n: usize) -> Vec<usize> {
        self.out[n].clone()
    }
    fn ortho_concentrate(&self) -> bool {
        self.concentrate
    }
    fn ortho_keeps_spline(&self, e: usize) -> bool {
        crate::neatogen::neatosplines::nop() == 2
            && self.edges.get(e).is_some_and(|ed| ed.spl.is_some())
    }
    fn node_seq(&self, n: usize) -> usize {
        n
    }
    fn xl_name(&self, n: usize) -> String {
        self.src.nodes[n].name.clone()
    }
}

impl SplineGraph for LGraph<'_> {
    fn tail(&self, e: usize) -> usize {
        self.edges[e].tail
    }
    fn head(&self, e: usize) -> usize {
        self.edges[e].head
    }
    fn coord(&self, n: usize) -> Point {
        self.nodes[n].coord
    }
    fn lw(&self, n: usize) -> f64 {
        self.nodes[n].lw
    }
    fn rw(&self, n: usize) -> f64 {
        self.nodes[n].rw
    }
    fn ht(&self, n: usize) -> f64 {
        self.nodes[n].ht
    }
    fn shape(&self, n: usize) -> Option<&NodeShape> {
        Some(&self.nodes[n].shape)
    }
    fn tail_port(&self, e: usize) -> &PortInfo {
        &self.edges[e].tail_port
    }
    fn head_port(&self, e: usize) -> &PortInfo {
        &self.edges[e].head_port
    }
    fn tail_port_mut(&mut self, e: usize) -> &mut PortInfo {
        &mut self.edges[e].tail_port
    }
    fn head_port_mut(&mut self, e: usize) -> &mut PortInfo {
        &mut self.edges[e].head_port
    }
    fn edge_attrs(&self, e: usize) -> &AttrMap {
        &self.src.edges[e].attrs
    }
    fn arrow_flags(&self, e: usize) -> (u32, u32) {
        arrow_flags(&self.src.edges[e].attrs, self.is_directed())
    }
    fn spl(&self, e: usize) -> Option<&Vec<Bezier>> {
        self.edges[e].spl.as_ref()
    }
    fn spl_mut(&mut self, e: usize) -> &mut Option<Vec<Bezier>> {
        &mut self.edges[e].spl
    }
    fn bb_mut(&mut self) -> &mut BoundingBox {
        &mut self.bb
    }
    fn label(&self, e: usize, k: LabelKind) -> Option<LabelView> {
        let ed = &self.edges[e];
        let l = match k {
            LabelKind::Main => ed.label.as_ref(),
            LabelKind::Head => ed.head_label.as_ref(),
            LabelKind::Tail => ed.tail_label.as_ref(),
            LabelKind::Xlabel => ed.xlabel.as_ref(),
        }?;
        Some(LabelView {
            dimen: l.dimen,
            pos: l.pos,
            set: l.set,
        })
    }
    fn set_label_pos(&mut self, e: usize, k: LabelKind, pos: Point) {
        let ed = &mut self.edges[e];
        let l = match k {
            LabelKind::Main => ed.label.as_mut(),
            LabelKind::Head => ed.head_label.as_mut(),
            LabelKind::Tail => ed.tail_label.as_mut(),
            LabelKind::Xlabel => ed.xlabel.as_mut(),
        };
        if let Some(l) = l {
            l.pos = pos;
            l.set = true;
        }
    }
    fn has_port_label_attrs(&self) -> bool {
        self.has_labelangle || self.has_labeldistance
    }
}

fn end_points(spl: &[Bezier]) -> (Point, Point) {
    let b = &spl[0];
    let p = if b.sflag != 0 { b.sp } else { b.list[0] };
    let b = &spl[spl.len() - 1];
    let q = if b.eflag != 0 {
        b.ep
    } else {
        b.list[b.list.len() - 1]
    };
    (p, q)
}

fn polyline_midpoint(spl: &[Bezier]) -> Point {
    let mut d = 0.0;
    for b in spl {
        let mut k = 3;
        while k < b.list.len() {
            d += dist(b.list[k - 3], b.list[k]);
            k += 3;
        }
    }
    let mut dd = d / 2.0;
    for b in spl {
        let mut k = 3;
        while k < b.list.len() {
            let pf = b.list[k - 3];
            let qf = b.list[k];
            let seg = dist(pf, qf);
            if seg >= dd {
                return Point::new(
                    qf.x.mul_add(dd, pf.x * (seg - dd)) / seg,
                    qf.y.mul_add(dd, pf.y * (seg - dd)) / seg,
                );
            }
            dd -= seg;
            k += 3;
        }
    }
    // #435: a bezier with no complete segment (the one-point bezier `clip_and_install`
    // installs when the clipping walks meet, e.g. a labeled back edge between nodes
    // whose margins touch) or NaN segment lengths never satisfy `seg >= dd`. C falls
    // out of the loop into `UNREACHABLE()` at the end of `polylineMidpoint`
    // (lib/common/splines.c:1277) and aborts, as it does on an empty spline or an
    // empty point list; the port answers those with the default point rather than a
    // panic. Rounding cannot strand the walk when a segment exists: before the last
    // segment the remaining half is `d_last - total / 2 <= d_last` up to
    // O(n * 2^-53) of the total. The midpoint of a one-point polyline is its only
    // point, so the port returns the spline's first point.
    spl.first()
        .and_then(|b| b.list.first())
        .copied()
        .unwrap_or_default()
}

/// `dotneato_closest(spl, pt)`.
pub fn dotneato_closest(spl: &[Bezier], pt: Point) -> Point {
    let mut besti = usize::MAX;
    let mut bestj = usize::MAX;
    let mut bestdist2 = f64::MAX;
    for (i, b) in spl.iter().enumerate() {
        for (j, p) in b.list.iter().enumerate() {
            let d2 = dist2(*p, pt);
            if bestj == usize::MAX || d2 < bestdist2 {
                besti = i;
                bestj = j;
                bestdist2 = d2;
            }
        }
    }
    let bz = &spl[besti];
    if bestj == bz.list.len() - 1 {
        bestj -= 1;
    }
    let j = 3 * (bestj / 3);
    let c = [bz.list[j], bz.list[j + 1], bz.list[j + 2], bz.list[j + 3]];
    let mut low = 0.0f64;
    let mut high = 1.0f64;
    let mut dlow2 = dist2(c[0], pt);
    let mut dhigh2 = dist2(c[3], pt);
    loop {
        let t = (low + high) / 2.0;
        let pt2 = bezier_pt(&c, t);
        if (dlow2 - dhigh2).abs() < 1.0 || (high - low).abs() < 0.00001 {
            return pt2;
        }
        if dlow2 < dhigh2 {
            high = t;
            dhigh2 = dist2(pt2, pt);
        } else {
            low = t;
            dlow2 = dist2(pt2, pt);
        }
    }
}

/// `edgeMidpoint(g, e)`.
pub fn edge_midpoint(et: EdgeType, spl: &[Bezier]) -> Point {
    let (p, q) = end_points(spl);
    if approxeqpt(p, q, MILLIPOINT) {
        p
    } else if et == EdgeType::Spline || et == EdgeType::Curved {
        dotneato_closest(spl, mid(p, q))
    } else {
        polyline_midpoint(spl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #360: where `pboxfn` writes no box, the boxes are the caller's `kept` ones, and
    /// the node box where it has none.
    #[test]
    fn a_pboxfn_that_writes_no_box_keeps_the_callers_boxes() {
        let b = |x: f64| BoundingBox::new(x, 0.0, x + 1.0, 1.0);
        let kept = Some(vec![b(1.0), b(2.0)]);
        assert_eq!(
            port_boxes(Some((TOP, None)), kept.clone()),
            Some((TOP, vec![b(1.0), b(2.0)]))
        );
        assert_eq!(port_boxes(Some((TOP, None)), None), None);
        assert_eq!(
            port_boxes(Some((BOTTOM, Some(vec![b(3.0)]))), kept.clone()),
            Some((BOTTOM, vec![b(3.0)]))
        );
        assert_eq!(port_boxes(Some((0, None)), kept.clone()), None, "no mask");
        assert_eq!(port_boxes(None, kept), None, "no pboxfn");
    }

    /// #173: C splits a segment with a non-finite control point until it stops on a half
    /// inside `bb` (leaving `bb` as it was) or overflows its stack; the port returns.
    #[test]
    fn update_bb_bz_leaves_bb_for_a_non_finite_segment() {
        let bb0 = BoundingBox::new(0.0, 0.0, 18.0, 18.0);
        for cp in [
            [Point::new(f64::NAN, 20.0); 4],
            [
                Point::new(0.0, 0.0),
                Point::new(f64::NAN, f64::NAN),
                Point::new(100.0, 0.0),
                Point::new(0.0, 0.0),
            ],
            [
                Point::new(0.0, 0.0),
                Point::new(5.0, 30.0),
                Point::new(10.0, -30.0),
                Point::new(f64::INFINITY, 0.0),
            ],
        ] {
            let mut bb = bb0;
            update_bb_bz(&mut bb, &cp);
            assert_eq!(bb, bb0, "{cp:?}");
        }
    }

    /// #227: a segment between points 1e17 apart cannot be refined to 2pt, as adjacent
    /// doubles there are 16 apart, and C never finishes. The budget stops the split, and
    /// the pieces taken whole still enclose the curve. A small segment is refined as in C.
    #[test]
    fn update_bb_bz_budget_ends_a_huge_segment_and_keeps_small_ones_exact() {
        let curve = |s: f64| {
            [
                Point::new(0.0, 0.0),
                Point::new(0.0, s),
                Point::new(s, s),
                Point::new(s, 0.0),
            ]
        };
        reset_bz_split_budget();
        let mut bb = BoundingBox::new(0.0, 0.0, 1.0, 1.0);
        update_bb_bz(&mut bb, &curve(100.0));
        // The curve's top is at 75; C accepts pieces whose control points are within 2pt.
        assert!(bb.ur.y >= 75.0 && bb.ur.y < 77.0, "{bb:?}");
        assert!(!BZ_OUT_OF_SPLITS.with(std::cell::Cell::get));

        let s = 1e17;
        let mut bb = BoundingBox::new(0.0, 0.0, 1.0, 1.0);
        update_bb_bz(&mut bb, &curve(s));
        assert!(bb.ur.y >= 0.75 * s && bb.ur.y <= s, "{bb:?}");
        assert!(bb.ur.x >= s && bb.ur.x.is_finite(), "{bb:?}");
        assert!(BZ_OUT_OF_SPLITS.with(std::cell::Cell::get));

        // Once a segment has run out, a later one makes no split at all: taken whole, its
        // control points are the bb.
        let mut bb = BoundingBox::new(0.0, 0.0, 1.0, 1.0);
        update_bb_bz(&mut bb, &curve(2.0 * s));
        assert_eq!(bb, BoundingBox::new(0.0, 0.0, 2.0 * s, 2.0 * s));
        reset_bz_split_budget();
        assert!(!BZ_OUT_OF_SPLITS.with(std::cell::Cell::get));

        // A segment needing exactly the splits left is refined in full, without a refusal.
        let fresh = || BoundingBox::new(0.0, 0.0, 1.0, 1.0);
        let mut left = u64::MAX;
        assert!(!update_bb_bz_r(&mut fresh(), &curve(100.0), &mut left));
        let needed = u64::MAX - left;
        let mut exact = needed;
        assert!(!update_bb_bz_r(&mut fresh(), &curve(100.0), &mut exact));
        assert_eq!(exact, 0);
        let mut short = needed - 1;
        assert!(update_bb_bz_r(&mut fresh(), &curve(100.0), &mut short));
    }

    /// #269: at 1e301 the pieces stop shrinking (a piece recurs two splits down) while
    /// `ptToLine2` overflows to NaN, so C recurses on it until its stack overflows (the
    /// segment of `digraph{ratio="1e300"; f:n -> b}`). The port stops at `BZ_MAX_DEPTH` and
    /// takes the pieces whole; the bb still contains the curve.
    #[test]
    fn update_bb_bz_ends_a_piece_that_splits_into_itself() {
        let cp = [
            Point::new(0.0, 4.500_000_000_000_001e301),
            Point::new(9.797_174_393_178_826e284, 6.100_000_000_000_001e301),
            Point::new(1.205_130_684_404_372_4e143, 9.000_000_000_000_001e300),
            Point::new(11.118_052_319_535_058, 9.000_000_000_000_001e300),
        ];
        reset_bz_split_budget();
        let mut bb = BoundingBox::new(-27.0, 0.0, 27.0, 5.400_000_000_000_001e301);
        update_bb_bz(&mut bb, &cp);
        assert!(BZ_OUT_OF_SPLITS.with(std::cell::Cell::get));
        // The pieces taken whole enclose the curve (up to its rounding at 1e301).
        let slack = 1e-12 * bb.ur.y;
        for k in 0..=64 {
            let (p, _, _) = bezier_split(&cp, f64::from(k) / 64.0);
            assert!(
                bb.ll.x - slack <= p.x
                    && p.x <= bb.ur.x + slack
                    && bb.ll.y - slack <= p.y
                    && p.y <= bb.ur.y + slack,
                "{k}: {p:?} {bb:?}"
            );
        }
        reset_bz_split_budget();
    }

    /// #435: the one-point bezier `clip_and_install` installs when the clipping walks
    /// meet (`start == end + 3`, as for a back edge between nodes whose margins touch)
    /// has no complete stride-3 segment, so the walk never satisfies `seg >= dd`. C
    /// falls out of it into `UNREACHABLE()` at the end of `polylineMidpoint`
    /// (lib/common/splines.c:1277) and aborts; the port returns the only point. With a
    /// real segment the walk returns the geometric midpoint, as C's does. With no
    /// points at all C aborts at the same place; the port returns the default point.
    #[test]
    fn polyline_midpoint_of_a_segmentless_bezier_is_its_only_point() {
        let only = Point::new(55.6, 17.9);
        let one_point = Bezier {
            list: vec![only],
            sflag: 0,
            eflag: 1,
            sp: Point::default(),
            ep: Point::new(54.0, 14.8),
        };
        assert_eq!(polyline_midpoint(&[one_point]), only);

        let p = |x: f64| Point::new(x, 0.0);
        let segment = Bezier {
            list: vec![p(0.0), p(0.0), p(0.0), p(10.0)],
            ..Bezier::default()
        };
        assert_eq!(polyline_midpoint(&[segment]), p(5.0));

        assert_eq!(polyline_midpoint(&[]), Point::default(), "no beziers");
        assert_eq!(
            polyline_midpoint(&[Bezier::default()]),
            Point::default(),
            "a bezier with no points"
        );
    }
}
