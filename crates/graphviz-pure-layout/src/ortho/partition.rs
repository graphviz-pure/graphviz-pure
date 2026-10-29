//! Port of `lib/ortho/partition.c`: decompose the free space around the node
//! rectangles into maximal rectangles, as the intersection of a horizontal
//! and a vertical trapezoidation (each split into monotone pieces).

use super::fail::{At, Fail, OResult, idx};
use super::trapezoid::{
    SegmentT, Trap, construct_trapezoids, equal_to, fp_equal, greater_than, is_valid_trap,
};
use graphviz_pure_core::math;
use graphviz_pure_core::{BoundingBox, Point};

const TR_FROM_UP: i32 = 1;
const TR_FROM_DN: i32 = 2;

#[derive(Debug, Clone, Copy, Default)]
struct MonChain {
    vnum: i32,
    next: i32,
    prev: i32,
}

#[derive(Debug, Clone, Copy, Default)]
struct VertexChain {
    pt: Point,
    vnext: [i32; 4],
    vpos: [i32; 4],
    nextfree: i32,
}

/// The file-scope statics of partition.c and the monotonation state.
struct Mono<'a> {
    chain_idx: i32,
    mon_idx: usize,
    mon: Vec<i32>,
    vert: Vec<VertexChain>,
    chain: Vec<MonChain>,
    visited: Vec<bool>,
    decomp: Vec<BoundingBox>,
    seg: &'a [SegmentT],
    tr: &'a [Trap],
    flip: bool,
}

/// `perp(p)`: rotate by 90 degrees counter-clockwise.
fn perp(p: Point) -> Point {
    Point::new(-p.y, p.x)
}

fn convert(bb: BoundingBox, flip: bool, ccw: bool) -> [Point; 4] {
    let mut pts = [bb.ll, Point::default(), bb.ur, Point::default()];
    if ccw {
        pts[1] = Point::new(bb.ur.x, bb.ll.y);
        pts[3] = Point::new(bb.ll.x, bb.ur.y);
    } else {
        pts[1] = Point::new(bb.ll.x, bb.ur.y);
        pts[3] = Point::new(bb.ur.x, bb.ll.y);
    }
    if flip {
        for p in &mut pts {
            *p = perp(*p);
        }
    }
    pts
}

fn store(seg: &mut [SegmentT], first: i32, pts: &[Point; 4]) -> i32 {
    let last = first + 4 - 1;
    for (j, i) in (first..=last).enumerate() {
        let iu = i as usize;
        if i == first {
            seg[iu].next = first + 1;
            seg[iu].prev = last;
        } else if i == last {
            seg[iu].next = first;
            seg[iu].prev = last - 1;
        } else {
            seg[iu].next = i + 1;
            seg[iu].prev = i - 1;
        }
        seg[iu].is_inserted = false;
        seg[iu].v0 = pts[j];
        let p = seg[iu].prev as usize;
        seg[p].v1 = pts[j];
    }
    last + 1
}

fn gen_segments(cells: &[BoundingBox], bb: BoundingBox, seg: &mut [SegmentT], flip: bool) {
    let mut i = 1;
    i = store(seg, i, &convert(bb, flip, true));
    for c in cells {
        i = store(seg, i, &convert(*c, flip, false));
    }
}

/// `generateRandomOrdering(n, permute)`.
fn generate_random_ordering(n: usize, permute: &mut [i32]) {
    for (i, p) in permute.iter_mut().enumerate().take(n) {
        *p = i as i32 + 1;
    }
    for i in 0..n {
        let j = math::fma(crate::cutil::drand48(), (n - i) as f64, i as f64) as usize;
        if j != i {
            permute.swap(i, j);
        }
    }
}

fn inside_polygon(t: &Trap, seg: &[SegmentT]) -> OResult<bool> {
    let rseg = t.rseg;
    if !t.is_valid {
        return Ok(false);
    }
    if t.lseg <= 0 || t.rseg <= 0 {
        return Ok(false);
    }
    if (!is_valid_trap(t.u0) && !is_valid_trap(t.u1))
        || (!is_valid_trap(t.d0) && !is_valid_trap(t.d1))
    {
        let s = seg.at(idx(rseg)?)?;
        return Ok(greater_than(s.v1, s.v0));
    }
    Ok(false)
}

fn get_angle(vp0: Point, vpnext: Point, vp1: Point) -> f64 {
    let v0 = Point::new(vpnext.x - vp0.x, vpnext.y - vp0.y);
    let v1 = Point::new(vp1.x - vp0.x, vp1.y - vp0.y);
    let dot = math::fma(v0.x, v1.x, v0.y * v1.y);
    let cross_sine = math::fma(v0.x, v1.y, -(v1.x * v0.y));
    let l0 = math::hypot(v0.x, v0.y);
    let l1 = math::hypot(v1.x, v1.y);
    if cross_sine >= 0.0 {
        dot / l0 / l1
    } else {
        -1.0 * dot / l0 / l1 - 2.0
    }
}

impl Mono<'_> {
    fn get_vertex_positions(&self, v0: i32, v1: i32) -> OResult<(usize, usize)> {
        let vp0 = self.vert.at(idx(v0)?)?;
        let vp1 = self.vert.at(idx(v1)?)?;
        let mut angle = -4.0;
        let mut tp = 0;
        for i in 0..4 {
            if vp0.vnext[i] <= 0 {
                continue;
            }
            let temp = get_angle(vp0.pt, self.vert.at(idx(vp0.vnext[i])?)?.pt, vp1.pt);
            if temp > angle {
                angle = temp;
                tp = i;
            }
        }
        let mut angle = -4.0;
        let mut tq = 0;
        for i in 0..4 {
            if vp1.vnext[i] <= 0 {
                continue;
            }
            let temp = get_angle(vp1.pt, self.vert.at(idx(vp1.vnext[i])?)?.pt, vp0.pt);
            if temp > angle {
                angle = temp;
                tq = i;
            }
        }
        Ok((tp, tq))
    }

    fn new_chain_element(&mut self) -> OResult<usize> {
        self.chain_idx += 1;
        let i = idx(self.chain_idx)?;
        if i >= self.chain.len() {
            self.chain.resize(i + 1, MonChain::default());
        }
        Ok(i)
    }

    fn make_new_monotone_poly(&mut self, mcur: usize, v0: i32, v1: i32) -> OResult<usize> {
        self.mon_idx += 1;
        let mnew = self.mon_idx;
        let (ip, iq) = self.get_vertex_positions(v0, v1)?;
        let (v0u, v1u) = (idx(v0)?, idx(v1)?);
        let p = idx(self.vert.at(v0u)?.vpos[ip])?;
        let q = idx(self.vert.at(v1u)?.vpos[iq])?;
        let i = self.new_chain_element()?;
        let j = self.new_chain_element()?;
        let ch = &mut self.chain;
        ch[i].vnum = v0;
        ch[j].vnum = v1;
        ch[i].next = ch.at(p)?.next;
        let pn = idx(ch[p].next)?;
        ch.at_mut(pn)?.prev = i as i32;
        ch[i].prev = j as i32;
        ch[j].next = i as i32;
        ch[j].prev = ch.at(q)?.prev;
        let qp = idx(ch[q].prev)?;
        ch.at_mut(qp)?.next = j as i32;
        ch[p].next = q as i32;
        ch[q].prev = p as i32;
        let nf0 = idx(self.vert[v0u].nextfree)?;
        let nf1 = idx(self.vert[v1u].nextfree)?;
        let inext = self.chain.at(idx(self.chain[i].next)?)?.vnum;
        let vp0 = &mut self.vert[v0u];
        vp0.vnext[ip] = v1;
        // A vertex has 4 chain slots. On degenerate cells (touching, overlapping
        // or non-finite boxes, #171) C writes `vpos[nextfree]` and
        // `vnext[nextfree]` past the end of its `int[4]` arrays, into the rest of
        // the vertex chain (and, once `nextfree` is overwritten, sometimes past
        // `vert`: undefined behaviour). C's rectangles do not depend on this data:
        // `traverse_polygon` builds them from the trapezoids and segments. So skip
        // the out-of-range writes and go on. Where C later fails, `mk_maze_graph`'s
        // `chkSgraph` check catches it.
        if let Some(slot) = vp0.vpos.get_mut(nf0) {
            *slot = i as i32;
            vp0.vnext[nf0] = inext;
        }
        vp0.nextfree += 1;
        let vp1 = &mut self.vert[v1u];
        if let Some(slot) = vp1.vpos.get_mut(nf1) {
            *slot = j as i32;
            vp1.vnext[nf1] = v0;
        }
        vp1.nextfree += 1;
        if mnew >= self.mon.len() {
            self.mon.resize(mnew + 1, 0);
        }
        *self.mon.at_mut(mcur)? = p as i32;
        self.mon[mnew] = i as i32;
        Ok(mnew)
    }

    /// `traverse_polygon(..., mcur, trnum, from, flip, dir, ...)`. C recurses
    /// once per trapezoid, which can nest as deep as there are trapezoids;
    /// the port keeps the pending calls on a stack. Each call's recursive
    /// calls come last, after all its own work, so running them from the
    /// stack in order visits the trapezoids in C's order.
    fn traverse(&mut self, mcur: usize, trnum: usize, from: usize, dir: i32) -> OResult<()> {
        let mut stack = vec![(mcur, trnum, from, dir)];
        let mut calls = Vec::new();
        while let Some((mcur, trnum, from, dir)) = stack.pop() {
            self.traverse_polygon(mcur, trnum, from, dir, &mut calls)?;
            stack.extend(calls.drain(..).rev());
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    /// One call of `traverse_polygon`: its recursive calls go to `calls`.
    fn traverse_polygon(
        &mut self,
        mcur: usize,
        trnum: usize,
        from: usize,
        dir: i32,
        calls: &mut Vec<(usize, usize, usize, i32)>,
    ) -> OResult<()> {
        if !is_valid_trap(trnum) || *self.visited.at(trnum)? {
            return Ok(());
        }
        let t = *self.tr.at(trnum)?;
        let seg = self.seg;
        let tr = self.tr;
        self.visited[trnum] = true;
        if t.hi.y > t.lo.y + super::trapezoid::C_EPS
            && fp_equal(seg.at(idx(t.lseg)?)?.v0.x, seg.at(idx(t.lseg)?)?.v1.x)
            && fp_equal(seg.at(idx(t.rseg)?)?.v0.x, seg.at(idx(t.rseg)?)?.v1.x)
        {
            let newbox = if self.flip {
                BoundingBox::new(
                    t.lo.y,
                    -seg.at(idx(t.rseg)?)?.v0.x,
                    t.hi.y,
                    -seg.at(idx(t.lseg)?)?.v0.x,
                )
            } else {
                BoundingBox::new(
                    seg.at(idx(t.lseg)?)?.v0.x,
                    t.lo.y,
                    seg.at(idx(t.rseg)?)?.v0.x,
                    t.hi.y,
                )
            };
            self.decomp.push(newbox);
        }
        let (up, dn) = (TR_FROM_UP, TR_FROM_DN);
        macro_rules! tp {
            ($m:expr, $t:expr, $d:expr) => {
                calls.push(($m, $t, trnum, $d))
            };
        }
        let vu0 = is_valid_trap(t.u0);
        let vu1 = is_valid_trap(t.u1);
        let vd0 = is_valid_trap(t.d0);
        let vd1 = is_valid_trap(t.d1);
        if !vu0 && !vu1 {
            if vd0 && vd1 {
                let v0 = tr.at(t.d1)?.lseg;
                let v1 = t.lseg;
                if from == t.d1 {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.d1, up);
                    tp!(mnew, t.d0, up);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.d0, up);
                    tp!(mnew, t.d1, up);
                }
            } else {
                tp!(mcur, t.u0, dn);
                tp!(mcur, t.u1, dn);
                tp!(mcur, t.d0, up);
                tp!(mcur, t.d1, up);
            }
        } else if !vd0 && !vd1 {
            if vu0 && vu1 {
                let v0 = t.rseg;
                let v1 = tr.at(t.u0)?.rseg;
                if from == t.u1 {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.u1, dn);
                    tp!(mnew, t.u0, dn);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.u0, dn);
                    tp!(mnew, t.u1, dn);
                }
            } else {
                tp!(mcur, t.u0, dn);
                tp!(mcur, t.u1, dn);
                tp!(mcur, t.d0, up);
                tp!(mcur, t.d1, up);
            }
        } else if vu0 && vu1 {
            if vd0 && vd1 {
                let v0 = tr.at(t.d1)?.lseg;
                let v1 = tr.at(t.u0)?.rseg;
                if (dir == dn && t.d1 == from) || (dir == up && t.u1 == from) {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.u1, dn);
                    tp!(mcur, t.d1, up);
                    tp!(mnew, t.u0, dn);
                    tp!(mnew, t.d0, up);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.u0, dn);
                    tp!(mcur, t.d0, up);
                    tp!(mnew, t.u1, dn);
                    tp!(mnew, t.d1, up);
                }
            } else if equal_to(t.lo, seg.at(idx(t.lseg)?)?.v1) {
                let v0 = tr.at(t.u0)?.rseg;
                let v1 = seg.at(idx(t.lseg)?)?.next;
                if dir == up && t.u0 == from {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.u0, dn);
                    tp!(mnew, t.d0, up);
                    tp!(mnew, t.u1, dn);
                    tp!(mnew, t.d1, up);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.u1, dn);
                    tp!(mcur, t.d0, up);
                    tp!(mcur, t.d1, up);
                    tp!(mnew, t.u0, dn);
                }
            } else {
                let v0 = t.rseg;
                let v1 = tr.at(t.u0)?.rseg;
                if dir == up && t.u1 == from {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.u1, dn);
                    tp!(mnew, t.d1, up);
                    tp!(mnew, t.d0, up);
                    tp!(mnew, t.u0, dn);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.u0, dn);
                    tp!(mcur, t.d0, up);
                    tp!(mcur, t.d1, up);
                    tp!(mnew, t.u1, dn);
                }
            }
        } else if vu0 || vu1 {
            if vd0 && vd1 {
                if equal_to(t.hi, seg.at(idx(t.lseg)?)?.v0) {
                    let v0 = tr.at(t.d1)?.lseg;
                    let v1 = t.lseg;
                    if !(dir == dn && t.d0 == from) {
                        let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                        tp!(mcur, t.u1, dn);
                        tp!(mcur, t.d1, up);
                        tp!(mcur, t.u0, dn);
                        tp!(mnew, t.d0, up);
                    } else {
                        let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                        tp!(mcur, t.d0, up);
                        tp!(mnew, t.u0, dn);
                        tp!(mnew, t.u1, dn);
                        tp!(mnew, t.d1, up);
                    }
                } else {
                    let v0 = tr.at(t.d1)?.lseg;
                    let v1 = seg.at(idx(t.rseg)?)?.next;
                    if dir == dn && t.d1 == from {
                        let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                        tp!(mcur, t.d1, up);
                        tp!(mnew, t.u1, dn);
                        tp!(mnew, t.u0, dn);
                        tp!(mnew, t.d0, up);
                    } else {
                        let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                        tp!(mcur, t.u0, dn);
                        tp!(mcur, t.d0, up);
                        tp!(mcur, t.u1, dn);
                        tp!(mnew, t.d1, up);
                    }
                }
            } else if equal_to(t.hi, seg.at(idx(t.lseg)?)?.v0)
                && equal_to(t.lo, seg.at(idx(t.rseg)?)?.v0)
            {
                let v0 = t.rseg;
                let v1 = t.lseg;
                if dir == up {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.u0, dn);
                    tp!(mcur, t.u1, dn);
                    tp!(mnew, t.d1, up);
                    tp!(mnew, t.d0, up);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.d1, up);
                    tp!(mcur, t.d0, up);
                    tp!(mnew, t.u0, dn);
                    tp!(mnew, t.u1, dn);
                }
            } else if equal_to(t.hi, seg.at(idx(t.rseg)?)?.v1)
                && equal_to(t.lo, seg.at(idx(t.lseg)?)?.v1)
            {
                let v0 = seg.at(idx(t.rseg)?)?.next;
                let v1 = seg.at(idx(t.lseg)?)?.next;
                if dir == up {
                    let mnew = self.make_new_monotone_poly(mcur, v1, v0)?;
                    tp!(mcur, t.u0, dn);
                    tp!(mcur, t.u1, dn);
                    tp!(mnew, t.d1, up);
                    tp!(mnew, t.d0, up);
                } else {
                    let mnew = self.make_new_monotone_poly(mcur, v0, v1)?;
                    tp!(mcur, t.d1, up);
                    tp!(mcur, t.d0, up);
                    tp!(mnew, t.u0, dn);
                    tp!(mnew, t.u1, dn);
                }
            } else {
                tp!(mcur, t.u0, dn);
                tp!(mcur, t.d0, up);
                tp!(mcur, t.u1, dn);
                tp!(mcur, t.d1, up);
            }
        }
        Ok(())
    }
}

/// `monotonate_trapezoids(nsegs, seg, tr, flip, decomp)`.
fn monotonate_trapezoids(
    nsegs: i32,
    seg: &[SegmentT],
    tr: &[Trap],
    flip: bool,
) -> OResult<Vec<BoundingBox>> {
    let n = idx(nsegs)?;
    let mut m = Mono {
        chain_idx: nsegs,
        mon_idx: 0,
        mon: vec![0; n.max(1)],
        vert: vec![VertexChain::default(); n + 1],
        chain: vec![MonChain::default(); tr.len().max(n + 1)],
        visited: vec![false; tr.len()],
        decomp: Vec::new(),
        seg,
        tr,
        flip,
    };
    let mut tr_start = tr.len();
    for (j, t) in tr.iter().enumerate() {
        if inside_polygon(t, seg)? {
            tr_start = j;
            break;
        }
    }
    for i in 1..=n {
        let s = seg.at(i)?;
        m.chain[i].prev = s.prev;
        m.chain[i].next = s.next;
        m.chain[i].vnum = i as i32;
        m.vert[i].pt = s.v0;
        m.vert[i].vnext[0] = s.next;
        m.vert[i].vpos[0] = i as i32;
        m.vert[i].nextfree = 1;
    }
    m.mon[0] = 1;
    // With no trapezoid inside the polygon, C reads `LIST_GET(tr, tr_start)`
    // one past the end and fails `gv_list_get_`'s assertion (#228's NaN pins).
    let ts = *tr.at(tr_start)?;
    if is_valid_trap(ts.u0) {
        m.traverse(0, tr_start, ts.u0, TR_FROM_UP)?;
    } else if is_valid_trap(ts.d0) {
        m.traverse(0, tr_start, ts.d0, TR_FROM_DN)?;
    }
    Ok(m.decomp)
}

/// `rectIntersect(d, r0, r1)`.
fn rect_intersect(r0: &BoundingBox, r1: &BoundingBox) -> Option<BoundingBox> {
    let d = BoundingBox::new(
        r0.ll.x.max(r1.ll.x),
        r0.ll.y.max(r1.ll.y),
        r0.ur.x.min(r1.ur.x),
        r0.ur.y.min(r1.ur.y),
    );
    (!(d.ll.x >= d.ur.x || d.ll.y >= d.ur.y)).then_some(d)
}

/// `partition(cells, ncells, &nrects, bb)`: the free rectangles.
pub fn partition(cells: &[BoundingBox], bb: BoundingBox) -> OResult<Vec<BoundingBox>> {
    let ncells = cells.len();
    let nsegs = 4 * (ncells + 1);
    let nsegs_i = i32::try_from(nsegs).map_err(|_| Fail::here())?;
    let mut segs = vec![SegmentT::default(); nsegs + 1];
    let mut permute = vec![0i32; nsegs];
    gen_segments(cells, bb, &mut segs, false);
    crate::cutil::srand48(173);
    generate_random_ordering(nsegs, &mut permute);
    // A failed `add_segment` is C's "horizontal/vertical trapezoid
    // construction failed" exit: partition returns NULL.
    let failed = |vertical| {
        move |f| match f {
            Fail::Segment => Fail::Trapezoidation { vertical },
            f => f,
        }
    };
    let hor_traps = construct_trapezoids(nsegs_i, &mut segs, &permute).map_err(failed(false))?;
    let hor_decomp = monotonate_trapezoids(nsegs_i, &segs, &hor_traps, false)?;
    gen_segments(cells, bb, &mut segs, true);
    generate_random_ordering(nsegs, &mut permute);
    let ver_traps = construct_trapezoids(nsegs_i, &mut segs, &permute).map_err(failed(true))?;
    let vert_decomp = monotonate_trapezoids(nsegs_i, &segs, &ver_traps, true)?;
    let mut rs = Vec::new();
    for v in &vert_decomp {
        for h in &hor_decomp {
            if let Some(b) = rect_intersect(v, h) {
                rs.push(b);
            }
        }
    }
    // C returns `LIST_DETACH(&rs, ...)`, which is NULL for an empty list, and
    // mkMaze fails on NULL: tiny touching nodes can leave no free cell (#203).
    if rs.is_empty() {
        return Err(Fail::NoFreeCell);
    }
    Ok(rs)
}
