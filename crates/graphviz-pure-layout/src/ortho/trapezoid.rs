//! Port of `lib/ortho/trapezoid.c` and `trap.h`: Seidel's randomized
//! trapezoidation of a set of polygons.

use super::fail::{At, Fail, Need, OResult, idx};
use graphviz_pure_core::Point;
use graphviz_pure_core::math;

pub const C_EPS: f64 = 1.0e-7;

/// `segment_t`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SegmentT {
    pub v0: Point,
    pub v1: Point,
    pub is_inserted: bool,
    pub root0: usize,
    pub root1: usize,
    pub next: i32,
    pub prev: i32,
}

/// `trap_t`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Trap {
    pub lseg: i32,
    pub rseg: i32,
    pub hi: Point,
    pub lo: Point,
    pub u0: usize,
    pub u1: usize,
    pub d0: usize,
    pub d1: usize,
    pub sink: usize,
    pub usave: usize,
    pub uside: i32,
    pub is_valid: bool,
}

/// `is_valid_trap(index)`.
pub fn is_valid_trap(index: usize) -> bool {
    index != 0 && index != usize::MAX
}

pub fn fp_equal(s: f64, t: f64) -> bool {
    (s - t).abs() <= C_EPS
}

pub fn dfp_cmp(f1: f64, f2: f64) -> i32 {
    let d = f1 - f2;
    if d < -C_EPS {
        -1
    } else if d > C_EPS {
        1
    } else {
        0
    }
}

pub fn equal_to(v0: Point, v1: Point) -> bool {
    fp_equal(v0.y, v1.y) && fp_equal(v0.x, v1.x)
}

pub fn greater_than(v0: Point, v1: Point) -> bool {
    if v0.y > v1.y + C_EPS {
        true
    } else if v0.y < v1.y - C_EPS {
        false
    } else {
        v0.x > v1.x
    }
}

const T_X: i32 = 1;
const T_Y: i32 = 2;
const T_SINK: i32 = 3;
const FIRSTPT: i32 = 1;
const LASTPT: i32 = 2;
const S_LEFT: i32 = 1;
const S_RIGHT: i32 = 2;

fn cross(v0: Point, v1: Point, v2: Point) -> f64 {
    math::fma(v1.x - v0.x, v2.y - v0.y, -((v1.y - v0.y) * (v2.x - v0.x)))
}

/// `qnode_t`.
#[derive(Debug, Clone, Copy, Default)]
struct QNode {
    nodetype: i32,
    segnum: i32,
    yval: Point,
    trnum: usize,
    parent: usize,
    left: usize,
    right: usize,
}

fn newnode(qs: &mut Vec<QNode>) -> usize {
    qs.push(QNode::default());
    qs.len() - 1
}

fn newtrap(tr: &mut Vec<Trap>) -> usize {
    tr.push(Trap::default());
    tr.len() - 1
}

fn max_(v0: Point, v1: Point) -> Point {
    if v0.y > v1.y + C_EPS {
        return v0;
    }
    if fp_equal(v0.y, v1.y) {
        if v0.x > v1.x + C_EPS {
            return v0;
        }
        return v1;
    }
    v1
}

fn min_(v0: Point, v1: Point) -> Point {
    if v0.y < v1.y - C_EPS {
        return v0;
    }
    if fp_equal(v0.y, v1.y) {
        if v0.x < v1.x {
            return v0;
        }
        return v1;
    }
    v1
}

fn greater_than_equal_to(v0: Point, v1: Point) -> bool {
    greater_than(v0, v1) || equal_to(v0, v1)
}

fn less_than(v0: Point, v1: Point) -> bool {
    !greater_than_equal_to(v0, v1)
}

fn init_query_structure(
    segnum: i32,
    seg: &mut [SegmentT],
    tr: &mut Vec<Trap>,
    qs: &mut Vec<QNode>,
) -> OResult<usize> {
    let s = *seg.at(idx(segnum)?)?;
    let i1 = newnode(qs);
    qs[i1].nodetype = T_Y;
    qs[i1].yval = max_(s.v0, s.v1);
    let root = i1;
    let i2 = newnode(qs);
    qs[i1].right = i2;
    qs[i2].nodetype = T_SINK;
    qs[i2].parent = i1;
    let i3 = newnode(qs);
    qs[i1].left = i3;
    qs[i3].nodetype = T_Y;
    qs[i3].yval = min_(s.v0, s.v1);
    qs[i3].parent = i1;
    let i4 = newnode(qs);
    qs[i3].left = i4;
    qs[i4].nodetype = T_SINK;
    qs[i4].parent = i3;
    let i5 = newnode(qs);
    qs[i3].right = i5;
    qs[i5].nodetype = T_X;
    qs[i5].segnum = segnum;
    qs[i5].parent = i3;
    let i6 = newnode(qs);
    qs[i5].left = i6;
    qs[i6].nodetype = T_SINK;
    qs[i6].parent = i5;
    let i7 = newnode(qs);
    qs[i5].right = i7;
    qs[i7].nodetype = T_SINK;
    qs[i7].parent = i5;
    let t1 = newtrap(tr);
    let t2 = newtrap(tr);
    let t3 = newtrap(tr);
    let t4 = newtrap(tr);
    tr[t1].hi = qs[i1].yval;
    tr[t2].hi = qs[i1].yval;
    tr[t4].lo = qs[i1].yval;
    tr[t1].lo = qs[i3].yval;
    tr[t2].lo = qs[i3].yval;
    tr[t3].hi = qs[i3].yval;
    tr[t4].hi = Point::new(f64::MAX, f64::MAX);
    tr[t3].lo = Point::new(-f64::MAX, -f64::MAX);
    tr[t1].rseg = segnum;
    tr[t2].lseg = segnum;
    tr[t1].u0 = t4;
    tr[t2].u0 = t4;
    tr[t1].d0 = t3;
    tr[t2].d0 = t3;
    tr[t4].d0 = t1;
    tr[t3].u0 = t1;
    tr[t4].d1 = t2;
    tr[t3].u1 = t2;
    tr[t1].sink = i6;
    tr[t2].sink = i7;
    tr[t3].sink = i4;
    tr[t4].sink = i2;
    for t in [t1, t2, t3, t4] {
        tr[t].is_valid = true;
    }
    qs[i2].trnum = t4;
    qs[i4].trnum = t3;
    qs[i6].trnum = t1;
    qs[i7].trnum = t2;
    seg.at_mut(idx(segnum)?)?.is_inserted = true;
    Ok(root)
}

fn is_left_of(segnum: i32, seg: &[SegmentT], v: Point) -> OResult<bool> {
    let s = seg.at(idx(segnum)?)?;
    let area = if greater_than(s.v1, s.v0) {
        if fp_equal(s.v1.y, v.y) {
            if v.x < s.v1.x { 1.0 } else { -1.0 }
        } else if fp_equal(s.v0.y, v.y) {
            if v.x < s.v0.x { 1.0 } else { -1.0 }
        } else {
            cross(s.v0, s.v1, v)
        }
    } else if fp_equal(s.v1.y, v.y) {
        if v.x < s.v1.x { 1.0 } else { -1.0 }
    } else if fp_equal(s.v0.y, v.y) {
        if v.x < s.v0.x { 1.0 } else { -1.0 }
    } else {
        cross(s.v1, s.v0, v)
    };
    Ok(area > 0.0)
}

fn inserted(segnum: i32, seg: &[SegmentT], whichpt: i32) -> OResult<bool> {
    let s = seg.at(idx(segnum)?)?;
    let other = if whichpt == FIRSTPT { s.prev } else { s.next };
    Ok(seg.at(idx(other)?)?.is_inserted)
}

/// `locate_endpoint(v, vo, r, seg, qs)`: C's tail recursion down the query
/// structure, as a loop. A query structure that does not end in a sink
/// within its size is broken: C would recurse through it until the stack
/// overflows (or index `qs` with garbage).
fn locate_endpoint(
    v: Point,
    vo: Point,
    r: usize,
    seg: &[SegmentT],
    qs: &[QNode],
) -> OResult<usize> {
    let mut r = r;
    for _ in 0..=qs.len() {
        let rptr = qs.at(r)?;
        r = match rptr.nodetype {
            T_SINK => return Ok(rptr.trnum),
            T_Y => {
                if greater_than(v, rptr.yval) {
                    rptr.right
                } else if equal_to(v, rptr.yval) {
                    if greater_than(vo, rptr.yval) {
                        rptr.right
                    } else {
                        rptr.left
                    }
                } else {
                    rptr.left
                }
            }
            T_X => {
                let s = seg.at(idx(rptr.segnum)?)?;
                let left = if equal_to(v, s.v0) || equal_to(v, s.v1) {
                    if fp_equal(v.y, vo.y) {
                        vo.x < v.x
                    } else {
                        is_left_of(rptr.segnum, seg, vo)?
                    }
                } else {
                    is_left_of(rptr.segnum, seg, v)?
                };
                if left { rptr.left } else { rptr.right }
            }
            // C asserts the node type (`assert(0)`).
            _ => return Err(Fail::here()),
        };
    }
    Err(Fail::here())
}

fn merge_trapezoids(
    segnum: i32,
    tfirst: usize,
    tlast: usize,
    side: i32,
    tr: &mut [Trap],
    qs: &mut [QNode],
) -> OResult<()> {
    let mut t = tfirst;
    // Each pass merges a trapezoid away or steps down to the next one, so a
    // longer walk is a cycle in broken links, where C loops forever.
    let mut budget = tr.len().saturating_mul(2).saturating_add(2);
    while is_valid_trap(t) && greater_than_equal_to(tr.at(t)?.lo, tr.at(tlast)?.lo) {
        budget = budget.checked_sub(1).need()?;
        // `cond` assigns tnext in short-circuit order.
        let matches = |tr: &[Trap], x: usize| -> OResult<bool> {
            Ok(if side == S_LEFT {
                tr.at(x)?.rseg == segnum
            } else {
                tr.at(x)?.lseg == segnum
            })
        };
        let mut tnext = tr.at(t)?.d0;
        let mut cond = is_valid_trap(tnext) && matches(tr, tnext)?;
        if !cond {
            tnext = tr.at(t)?.d1;
            cond = is_valid_trap(tnext) && matches(tr, tnext)?;
        }
        if cond && tr.at(t)?.lseg == tr.at(tnext)?.lseg && tr.at(t)?.rseg == tr.at(tnext)?.rseg {
            let ptnext = qs.at(tr.at(tnext)?.sink)?.parent;
            if qs.at(ptnext)?.left == tr.at(tnext)?.sink {
                qs.at_mut(ptnext)?.left = tr.at(t)?.sink;
            } else {
                qs.at_mut(ptnext)?.right = tr.at(t)?.sink;
            }
            tr.at_mut(t)?.d0 = tr.at(tnext)?.d0;
            if is_valid_trap(tr.at(t)?.d0) {
                let d = tr.at(t)?.d0;
                if tr.at(d)?.u0 == tnext {
                    tr.at_mut(d)?.u0 = t;
                } else if tr.at(d)?.u1 == tnext {
                    tr.at_mut(d)?.u1 = t;
                }
            }
            tr.at_mut(t)?.d1 = tr.at(tnext)?.d1;
            if is_valid_trap(tr.at(t)?.d1) {
                let d = tr.at(t)?.d1;
                if tr.at(d)?.u0 == tnext {
                    tr.at_mut(d)?.u0 = t;
                } else if tr.at(d)?.u1 == tnext {
                    tr.at_mut(d)?.u1 = t;
                }
            }
            tr.at_mut(t)?.lo = tr.at(tnext)?.lo;
            tr.at_mut(tnext)?.is_valid = false;
        } else {
            t = tnext;
        }
    }
    Ok(())
}

/// `update_trapezoid(s, seg, tr, t, tn)`. Where `t.u0` is `SIZE_MAX` (no
/// upper neighbour, #199), C reads `LIST_GET(tr, SIZE_MAX)` and fails the
/// `index < list.size` assertion in `gv_list_get_` (`lib/util/list.c:211`);
/// the port's checked read fails there too.
fn update_trapezoid(
    s: &SegmentT,
    seg: &[SegmentT],
    tr: &mut [Trap],
    t: usize,
    tn: usize,
) -> OResult<()> {
    if is_valid_trap(tr.at(t)?.u0) && is_valid_trap(tr.at(t)?.u1) {
        if is_valid_trap(tr.at(t)?.usave) {
            if tr.at(t)?.uside == S_LEFT {
                tr.at_mut(tn)?.u0 = tr.at(t)?.u1;
                tr.at_mut(t)?.u1 = usize::MAX;
                tr.at_mut(tn)?.u1 = tr.at(t)?.usave;
                let a = tr.at(t)?.u0;
                tr.at_mut(a)?.d0 = t;
                let b = tr.at(tn)?.u0;
                tr.at_mut(b)?.d0 = tn;
                let c = tr.at(tn)?.u1;
                tr.at_mut(c)?.d0 = tn;
            } else {
                tr.at_mut(tn)?.u1 = usize::MAX;
                tr.at_mut(tn)?.u0 = tr.at(t)?.u1;
                tr.at_mut(t)?.u1 = tr.at(t)?.u0;
                tr.at_mut(t)?.u0 = tr.at(t)?.usave;
                let a = tr.at(t)?.u0;
                tr.at_mut(a)?.d0 = t;
                let b = tr.at(t)?.u1;
                tr.at_mut(b)?.d0 = t;
                let c = tr.at(tn)?.u0;
                tr.at_mut(c)?.d0 = tn;
            }
            tr.at_mut(t)?.usave = 0;
            tr.at_mut(tn)?.usave = 0;
        } else {
            tr.at_mut(tn)?.u0 = tr.at(t)?.u1;
            tr.at_mut(t)?.u1 = usize::MAX;
            tr.at_mut(tn)?.u1 = usize::MAX;
            let b = tr.at(tn)?.u0;
            tr.at_mut(b)?.d0 = tn;
        }
    } else {
        let tmp_u = tr.at(t)?.u0;
        let td0 = tr.at(tmp_u)?.d0;
        if is_valid_trap(td0) && is_valid_trap(tr.at(tmp_u)?.d1) {
            if tr.at(td0)?.rseg > 0 && !is_left_of(tr.at(td0)?.rseg, seg, s.v1)? {
                tr.at_mut(t)?.u0 = usize::MAX;
                tr.at_mut(t)?.u1 = usize::MAX;
                tr.at_mut(tn)?.u1 = usize::MAX;
                let b = tr.at(tn)?.u0;
                tr.at_mut(b)?.d1 = tn;
            } else {
                tr.at_mut(tn)?.u0 = usize::MAX;
                tr.at_mut(tn)?.u1 = usize::MAX;
                tr.at_mut(t)?.u1 = usize::MAX;
                let a = tr.at(t)?.u0;
                tr.at_mut(a)?.d0 = t;
            }
        } else {
            let a = tr.at(t)?.u0;
            tr.at_mut(a)?.d0 = t;
            tr.at_mut(a)?.d1 = tn;
        }
    }
    Ok(())
}

/// Splits `tu` at `v`: the lower half becomes a new trapezoid; returns it.
fn split_at(
    tr: &mut Vec<Trap>,
    qs: &mut Vec<QNode>,
    segnum: i32,
    tu: usize,
    v: Point,
) -> OResult<usize> {
    let tl = newtrap(tr);
    *tr.at_mut(tl)? = *tr.at(tu)?;
    tr.at_mut(tu)?.lo = v;
    tr.at_mut(tl)?.hi = v;
    tr.at_mut(tu)?.d0 = tl;
    tr.at_mut(tu)?.d1 = 0;
    tr.at_mut(tl)?.u0 = tu;
    tr.at_mut(tl)?.u1 = 0;
    let d0 = tr.at(tl)?.d0;
    if is_valid_trap(d0) && tr.at(d0)?.u0 == tu {
        tr.at_mut(d0)?.u0 = tl;
    }
    if is_valid_trap(d0) && tr.at(d0)?.u1 == tu {
        tr.at_mut(d0)?.u1 = tl;
    }
    let d1 = tr.at(tl)?.d1;
    if is_valid_trap(d1) && tr.at(d1)?.u0 == tu {
        tr.at_mut(d1)?.u0 = tl;
    }
    if is_valid_trap(d1) && tr.at(d1)?.u1 == tu {
        tr.at_mut(d1)?.u1 = tl;
    }
    let i1 = newnode(qs);
    let i2 = newnode(qs);
    let sk = tr.at(tu)?.sink;
    qs.at_mut(sk)?.nodetype = T_Y;
    qs.at_mut(sk)?.yval = v;
    qs.at_mut(sk)?.segnum = segnum;
    qs.at_mut(sk)?.left = i2;
    qs.at_mut(sk)?.right = i1;
    qs.at_mut(i1)?.nodetype = T_SINK;
    qs.at_mut(i1)?.trnum = tu;
    qs.at_mut(i1)?.parent = sk;
    qs.at_mut(i2)?.nodetype = T_SINK;
    qs.at_mut(i2)?.trnum = tl;
    qs.at_mut(i2)?.parent = sk;
    tr.at_mut(tu)?.sink = i1;
    tr.at_mut(tl)?.sink = i2;
    Ok(tl)
}

/// `add_segment`. [`Fail::Segment`] where C prints "trapezoid segment
/// construction failed" and returns -1.
fn add_segment(
    segnum: i32,
    seg: &mut [SegmentT],
    tr: &mut Vec<Trap>,
    qs: &mut Vec<QNode>,
) -> OResult<()> {
    let mut s = *seg.at(idx(segnum)?)?;
    let mut tfirstr = 0usize;
    let mut tlastr = 0usize;
    let mut tribot = false;
    let is_swapped = if greater_than(s.v1, s.v0) {
        std::mem::swap(&mut s.v0, &mut s.v1);
        std::mem::swap(&mut s.root0, &mut s.root1);
        true
    } else {
        false
    };
    let tfirst = if !inserted(segnum, seg, if is_swapped { LASTPT } else { FIRSTPT })? {
        let tu = locate_endpoint(s.v0, s.v1, s.root0, seg, qs)?;
        split_at(tr, qs, segnum, tu, s.v0)?
    } else {
        locate_endpoint(s.v0, s.v1, s.root0, seg, qs)?
    };
    let tlast = if !inserted(segnum, seg, if is_swapped { FIRSTPT } else { LASTPT })? {
        let tu = locate_endpoint(s.v1, s.v0, s.root1, seg, qs)?;
        split_at(tr, qs, segnum, tu, s.v1)?;
        tu
    } else {
        tribot = true;
        locate_endpoint(s.v1, s.v0, s.root1, seg, qs)?
    };
    let mut t = tfirst;
    // The walk visits each trapezoid above `tlast` once, going down; a longer
    // one follows a cycle in broken links, where C loops (adding a trapezoid
    // each time) until memory runs out.
    let mut budget = tr.len();
    while is_valid_trap(t) && greater_than_equal_to(tr.at(t)?.lo, tr.at(tlast)?.lo) {
        budget = budget.checked_sub(1).need()?;
        let sk = tr.at(t)?.sink;
        let i1 = newnode(qs);
        let i2 = newnode(qs);
        qs.at_mut(sk)?.nodetype = T_X;
        qs.at_mut(sk)?.segnum = segnum;
        qs.at_mut(sk)?.left = i1;
        qs.at_mut(sk)?.right = i2;
        qs.at_mut(i1)?.nodetype = T_SINK;
        qs.at_mut(i1)?.trnum = t;
        qs.at_mut(i1)?.parent = sk;
        qs.at_mut(i2)?.nodetype = T_SINK;
        let tn = newtrap(tr);
        qs.at_mut(i2)?.trnum = tn;
        tr.at_mut(tn)?.is_valid = true;
        qs.at_mut(i2)?.parent = sk;
        if t == tfirst {
            tfirstr = tn;
        }
        if equal_to(tr.at(t)?.lo, tr.at(tlast)?.lo) {
            tlastr = tn;
        }
        *tr.at_mut(tn)? = *tr.at(t)?;
        tr.at_mut(t)?.sink = i1;
        tr.at_mut(tn)?.sink = i2;
        let t_sav = t;
        let tn_sav = tn;
        let (d0v, d1v) = (is_valid_trap(tr.at(t)?.d0), is_valid_trap(tr.at(t)?.d1));
        if !d0v && !d1v {
            return Err(Fail::Segment);
        } else if d0v && !d1v || !d0v && d1v {
            let use_d0 = d0v;
            update_trapezoid(&s, seg, tr, t, tn)?;
            let low = |tr: &[Trap], x: usize| -> OResult<usize> {
                let x = tr.at(x)?;
                Ok(if use_d0 { x.d0 } else { x.d1 })
            };
            if fp_equal(tr.at(t)?.lo.y, tr.at(tlast)?.lo.y)
                && fp_equal(tr.at(t)?.lo.x, tr.at(tlast)?.lo.x)
                && tribot
            {
                let tmptriseg = if is_swapped {
                    seg.at(idx(segnum)?)?.prev
                } else {
                    seg.at(idx(segnum)?)?.next
                };
                if tmptriseg > 0 && is_left_of(tmptriseg, seg, s.v0)? {
                    let d = low(tr, t)?;
                    tr.at_mut(d)?.u0 = t;
                    tr.at_mut(tn)?.d0 = usize::MAX;
                    tr.at_mut(tn)?.d1 = usize::MAX;
                } else {
                    let d = low(tr, tn)?;
                    tr.at_mut(d)?.u1 = tn;
                    tr.at_mut(t)?.d0 = usize::MAX;
                    tr.at_mut(t)?.d1 = usize::MAX;
                }
            } else {
                let d = low(tr, t)?;
                if is_valid_trap(tr.at(d)?.u0) && is_valid_trap(tr.at(d)?.u1) {
                    if tr.at(d)?.u0 == t {
                        tr.at_mut(d)?.usave = tr.at(d)?.u1;
                        tr.at_mut(d)?.uside = S_LEFT;
                    } else {
                        tr.at_mut(d)?.usave = tr.at(d)?.u0;
                        tr.at_mut(d)?.uside = S_RIGHT;
                    }
                }
                tr.at_mut(d)?.u0 = t;
                tr.at_mut(d)?.u1 = tn;
            }
            t = low(tr, t)?;
        } else {
            let mut i_d0 = false;
            if fp_equal(tr.at(t)?.lo.y, s.v0.y) {
                if tr.at(t)?.lo.x > s.v0.x {
                    i_d0 = true;
                }
            } else {
                let y0 = tr.at(t)?.lo.y;
                let yt = (y0 - s.v0.y) / (s.v1.y - s.v0.y);
                let tmppt = Point::new(math::fma(yt, s.v1.x - s.v0.x, s.v0.x), y0);
                if less_than(tmppt, tr.at(t)?.lo) {
                    i_d0 = true;
                }
            }
            update_trapezoid(&s, seg, tr, t, tn)?;
            let tnext;
            let (d0, d1) = (tr.at(t)?.d0, tr.at(t)?.d1);
            if fp_equal(tr.at(t)?.lo.y, tr.at(tlast)?.lo.y)
                && fp_equal(tr.at(t)?.lo.x, tr.at(tlast)?.lo.x)
                && tribot
            {
                tr.at_mut(d0)?.u0 = t;
                tr.at_mut(d0)?.u1 = usize::MAX;
                tr.at_mut(d1)?.u0 = tn;
                tr.at_mut(d1)?.u1 = usize::MAX;
                tr.at_mut(tn)?.d0 = d1;
                tr.at_mut(t)?.d1 = usize::MAX;
                tr.at_mut(tn)?.d1 = usize::MAX;
                tnext = tr.at(t)?.d1;
            } else if i_d0 {
                tr.at_mut(d0)?.u0 = t;
                tr.at_mut(d0)?.u1 = tn;
                tr.at_mut(d1)?.u0 = tn;
                tr.at_mut(d1)?.u1 = usize::MAX;
                tr.at_mut(t)?.d1 = usize::MAX;
                tnext = tr.at(t)?.d0;
            } else {
                tr.at_mut(d0)?.u0 = t;
                tr.at_mut(d0)?.u1 = usize::MAX;
                tr.at_mut(d1)?.u0 = t;
                tr.at_mut(d1)?.u1 = tn;
                tr.at_mut(tn)?.d0 = d1;
                tr.at_mut(tn)?.d1 = usize::MAX;
                tnext = tr.at(t)?.d1;
            }
            t = tnext;
        }
        tr.at_mut(t_sav)?.rseg = segnum;
        tr.at_mut(tn_sav)?.lseg = segnum;
    }
    merge_trapezoids(segnum, tfirst, tlast, S_LEFT, tr, qs)?;
    merge_trapezoids(segnum, tfirstr, tlastr, S_RIGHT, tr, qs)?;
    seg.at_mut(idx(segnum)?)?.is_inserted = true;
    Ok(())
}

fn find_new_roots(segnum: usize, seg: &mut [SegmentT], tr: &[Trap], qs: &[QNode]) -> OResult<()> {
    let s = *seg.at(segnum)?;
    if s.is_inserted {
        return Ok(());
    }
    let r0 = locate_endpoint(s.v0, s.v1, s.root0, seg, qs)?;
    seg.at_mut(segnum)?.root0 = tr.at(r0)?.sink;
    let r1 = locate_endpoint(s.v1, s.v0, s.root1, seg, qs)?;
    seg.at_mut(segnum)?.root1 = tr.at(r1)?.sink;
    Ok(())
}

fn math_logstar_n(n: i32) -> i32 {
    let mut i = 0;
    let mut v = f64::from(n);
    while v >= 1.0 {
        v = math::log2(v);
        i += 1;
    }
    i - 1
}

fn math_n(n: i32, h: i32) -> i32 {
    let mut v = f64::from(n);
    for _ in 0..h {
        v = math::log2(v);
    }
    (f64::from(n) / v).ceil() as i32
}

/// `construct_trapezoids(nseg, seg, permute)`.
pub fn construct_trapezoids(
    nseg: i32,
    seg: &mut [SegmentT],
    permute: &[i32],
) -> OResult<Vec<Trap>> {
    let mut qs: Vec<QNode> = Vec::new();
    let mut tr: Vec<Trap> = vec![Trap::default()];
    let mut segi = 0usize;
    let root = init_query_structure(*permute.at(segi)?, seg, &mut tr, &mut qs)?;
    segi += 1;
    for s in seg.iter_mut().take(nseg as usize + 1).skip(1) {
        s.root0 = root;
        s.root1 = root;
    }
    let logstar = math_logstar_n(nseg);
    for h in 1..=logstar {
        for _ in math_n(nseg, h - 1) + 1..=math_n(nseg, h) {
            add_segment(*permute.at(segi)?, seg, &mut tr, &mut qs)?;
            segi += 1;
        }
        for i in 1..=nseg as usize {
            find_new_roots(i, seg, &tr, &qs)?;
        }
    }
    for _ in math_n(nseg, logstar) + 1..=nseg {
        add_segment(*permute.at(segi)?, seg, &mut tr, &mut qs)?;
        segi += 1;
    }
    Ok(tr)
}

#[cfg(test)]
mod tests {
    use super::super::maze::mk_maze;
    use graphviz_pure_core::Point;

    /// Issue #199: the node boxes sfdp gives the graph of
    /// `tests/golden/sfdp/attr_label_scheme_4_overlap.dot` (C's are just as
    /// far out). A segment's trapezoid has no upper neighbour in
    /// `update_trapezoid`; C fails the `gv_list_get_` bounds assertion there or
    /// fails `add_segment`, and the port indexed out of bounds. Now
    /// `construct_trapezoids` fails, so `partition` and the maze do.
    #[test]
    fn trapezoid_without_upper_neighbour_fails_the_maze() {
        crate::cutil::in_fresh_run(|| {
            let nodes = [
                (5.645_595_931_397_244e143, 2.714_485_296_736_977_5e141, 54.0),
                (
                    3.467_860_031_774_634_7e143,
                    6.468_036_434_169_714_6e140,
                    122.028_051_165_226_49,
                ),
                (7.059_533_087_296_913e143, 5.178_586_274_370_083e141, 54.0),
                (
                    1.617_525_775_244_846_3e143,
                    4.028_608_178_598_936_4e141,
                    122.028_051_165_226_49,
                ),
                (0.0, 6.409_635_423_132_405e141, 54.0),
                (
                    1.842_062_239_158_465_5e143,
                    4.832_925_318_390_855e141,
                    120.9456,
                ),
                (2.007_766_420_241_161e143, 4.169_174_310_896_976_4e141, 54.0),
                (6.903_054_462_442_888e143, 1.839_068_144_634_683_3e141, 54.0),
                (
                    7.415_308_134_430_418e143,
                    7.120_899_497_147_811e141,
                    122.028_051_165_226_49,
                ),
                (6.538_957_779_520_443e143, 5.270_099_366_146_343_6e141, 54.0),
                (3.699_826_958_060_975e143, 7.522_274_695_607_58e141, 54.0),
                (5.758_790_465_790_995e143, 0.0, 122.028_051_165_226_49),
            ]
            .map(|(x, y, w)| (Point::new(x, y), w, 36.0));
            assert!(mk_maze(&nodes).is_err());
        });
    }
}
