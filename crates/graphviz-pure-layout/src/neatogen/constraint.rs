//! Orthogonal-constraint overlap removal, ported from
//! `lib/neatogen/constraint.c` (`cAdjust` and its helpers) for
//! `overlap=ortho|ortho_yx|orthoxy|orthoyx|portho|portho_yx|porthoxy|porthoyx`.
//!
//! Each pass builds a constraint DAG over the nodes sorted by one coordinate
//! (a cdt ordered bag, reproduced by [`DtTree`]), solves it with network
//! simplex (`rank(cg, 2, INT_MAX)`, [`crate::common::ns`]) and moves the
//! nodes to the resulting ranks. Coordinates are integers in tenths of a
//! point, as in C.

use super::adjust::{AdjustMode, sep_factor};
use crate::common::ns::{self, NsData, NsEdge, NsNode};
use crate::common::types::{LGraph, Sub};
use crate::cutil::DtTree;
use graphviz_pure_core::math;

/// `SCALE`: for precision, scale up before the algorithms, then down.
const SCALE: i32 = 10;
const SCALE2: i32 = SCALE / 2;

#[derive(Debug, Clone, Copy, Default)]
struct IBox {
    llx: i32,
    lly: i32,
    urx: i32,
    ury: i32,
}

/// `nitem`.
#[derive(Debug, Clone, Copy, Default)]
struct NItem {
    val: i32,
    x: i32,
    y: i32,
    np: usize,
    cnode: usize,
    bb: IBox,
}

type DistFn = fn(&IBox, &IBox) -> i32;
type IntersectFn = fn(&NItem, &NItem) -> bool;

// The `int` arithmetic below wraps where C's does (UB in C, a plain 32-bit add under clang):
// positions near 3e6 inches and beyond already wrap when `initItem` converts them (#240).
fn dist_y(b1: &IBox, b2: &IBox) -> i32 {
    (b1.ury.wrapping_sub(b1.lly)).wrapping_add(b2.ury.wrapping_sub(b2.lly)) / 2
}

fn dist_x(b1: &IBox, b2: &IBox) -> i32 {
    (b1.urx.wrapping_sub(b1.llx)).wrapping_add(b2.urx.wrapping_sub(b2.llx)) / 2
}

/// `intersectX0`: boxes could overlap if shifted in y but don't, or overlap
/// and a y move is the smaller fix. `q` is above `p`.
fn intersect_x0(p: &NItem, q: &NItem) -> bool {
    let v = p.bb.llx <= q.bb.urx && q.bb.llx <= p.bb.urx;
    if !v {
        return false;
    }
    if p.bb.ury < q.bb.lly {
        return true;
    }
    let ydelta = dist_y(&p.bb, &q.bb).wrapping_sub(q.y.wrapping_sub(p.y));
    let xdelta = if q.x >= p.x {
        dist_x(&p.bb, &q.bb).wrapping_sub(q.x.wrapping_sub(p.x))
    } else {
        dist_x(&p.bb, &q.bb).wrapping_sub(p.x.wrapping_sub(q.x))
    };
    ydelta <= xdelta
}

/// `intersectY0`: as `intersectX0` with the axes swapped; `q` is right of `p`.
fn intersect_y0(p: &NItem, q: &NItem) -> bool {
    let v = p.bb.lly <= q.bb.ury && q.bb.lly <= p.bb.ury;
    if !v {
        return false;
    }
    if p.bb.urx < q.bb.llx {
        return true;
    }
    let xdelta = dist_x(&p.bb, &q.bb).wrapping_sub(q.x.wrapping_sub(p.x));
    let ydelta = if q.y >= p.y {
        dist_y(&p.bb, &q.bb).wrapping_sub(q.y.wrapping_sub(p.y))
    } else {
        dist_y(&p.bb, &q.bb).wrapping_sub(p.y.wrapping_sub(q.y))
    };
    xdelta <= ydelta
}

fn intersect_y(p: &NItem, q: &NItem) -> bool {
    p.bb.lly <= q.bb.ury && q.bb.lly <= p.bb.ury
}

fn intersect_x(p: &NItem, q: &NItem) -> bool {
    p.bb.llx <= q.bb.urx && q.bb.llx <= p.bb.urx
}

/// A strict directed constraint graph (`agopen("cg", Agstrictdirected)`)
/// with `agedge(cg, t, h, NULL, 1)` returning an existing edge.
struct Cg {
    ns: NsData,
    /// `(tail, head) -> edge`.
    edge_of: std::collections::HashMap<(usize, usize), usize>,
}

impl Cg {
    fn new() -> Self {
        Cg {
            ns: NsData::default(),
            edge_of: Default::default(),
        }
    }

    fn add_node(&mut self) -> usize {
        let n = self.ns.nodes.len();
        self.ns.nodes.push(NsNode::default());
        self.ns.nlist.push(n);
        n
    }

    /// `agedge(cg, t, h, NULL, 1)`: `(edge, created)`.
    fn agedge(&mut self, t: usize, h: usize) -> (usize, bool) {
        if let Some(&e) = self.edge_of.get(&(t, h)) {
            return (e, false);
        }
        let e = self.ns.edges.len();
        self.ns.edges.push(NsEdge {
            tail: t,
            head: h,
            ..Default::default()
        });
        self.edge_of.insert((t, h), e);
        (e, true)
    }

    /// `elist_append(e, ND_out(t)); elist_append(e, ND_in(h))`.
    fn append(&mut self, e: usize) {
        let (t, h) = (self.ns.edges[e].tail, self.ns.edges[e].head);
        self.ns.nodes[t].out.push(e);
        self.ns.nodes[h].in_.push(e);
    }
}

/// `mapGraphs(vg, cg, dist)`: the visibility edges `vg` become constraint
/// edges between the corresponding (merged) constraint nodes.
fn map_graphs(items: &[NItem], vg: &[(usize, usize)], cg: &mut Cg, dist: DistFn) {
    for &(p, q) in vg {
        let (tp, hp) = (&items[p], &items[q]);
        let delta = dist(&tp.bb, &hp.bb);
        let (ce, _) = cg.agedge(tp.cnode, hp.cnode);
        cg.ns.edges[ce].weight = 1;
        if cg.ns.edges[ce].minlen < delta {
            if cg.ns.edges[ce].minlen == 0 {
                cg.append(ce);
            }
            cg.ns.edges[ce].minlen = delta;
        }
    }
}

/// `mkNConstraintG`: like `mkConstraintG` without enforcing the orthogonal
/// order; overlapping pairs (per `intersect`) are kept apart in their current
/// order, with a heavier weight when they are joined by a real edge.
fn mk_n_constraint_g(
    g: &LGraph<'_>,
    sub: &Sub,
    items: &mut [NItem],
    list: &[usize],
    intersect: IntersectFn,
    dist: DistFn,
) -> Cg {
    let mut cg = Cg::new();
    for &p in list {
        items[p].cnode = cg.add_node();
    }
    for (k, &p) in list.iter().enumerate() {
        for &nxp in &list[k + 1..] {
            let mut e = None;
            if intersect(&items[p], &items[nxp]) {
                let delta = dist(&items[p].bb, &items[nxp].bb);
                let (ce, _) = cg.agedge(items[p].cnode, items[nxp].cnode);
                cg.ns.edges[ce].minlen = delta;
                cg.ns.edges[ce].weight = 1;
                e = Some(ce);
            }
            if let Some(ce) = e
                && g.find_edge(sub, items[p].np, items[nxp].np).is_some()
            {
                cg.ns.edges[ce].weight = 100;
            }
        }
    }
    // Edges were created per tail in list order with increasing heads, which
    // is the agfstout (head sequence) order the C loop appends them in.
    for e in 0..cg.ns.edges.len() {
        cg.append(e);
    }
    cg
}

/// `mkConstraintG`: a chain over the distinct coordinate values enforcing
/// their left-to-right order, plus constraints between items of different
/// values that would overlap (per `intersect`).
fn mk_constraint_g(
    items: &mut [NItem],
    list: &[usize],
    intersect: IntersectFn,
    dist: DistFn,
) -> Cg {
    let mut cg = Cg::new();
    // construct basic chain to enforce left to right order
    let mut oldval = -i32::MAX;
    let mut prev: Option<usize> = None;
    let mut n = 0;
    for &p in list {
        if oldval != items[p].val {
            oldval = items[p].val;
            n = cg.add_node();
            if let Some(pv) = prev {
                let (e, _) = cg.agedge(pv, n);
                cg.ns.edges[e].minlen = SCALE;
                cg.ns.edges[e].weight = 1;
                cg.append(e);
            }
            prev = Some(n);
        }
        items[p].cnode = n;
    }

    // visibility graph: edges from each item to later items of another value
    let mut vg: Vec<(usize, usize)> = Vec::new();
    let mut oldval = -i32::MAX;
    let mut nxt: Option<usize> = None;
    for (k, &p) in list.iter().enumerate() {
        if oldval != items[p].val {
            oldval = items[p].val;
            nxt = list[k + 1..]
                .iter()
                .position(|&q| items[q].val != oldval)
                .map(|i| k + 1 + i);
            if nxt.is_none() {
                break;
            }
        }
        for &nxp in &list[nxt.unwrap()..] {
            if intersect(&items[p], &items[nxp]) {
                vg.push((p, nxp));
            }
        }
    }
    map_graphs(items, &vg, &mut cg, dist);
    cg
}

/// `constrainX` / `constrainY`: build the constraint graph for one axis,
/// solve it with network simplex and move the items.
fn constrain(
    g: &LGraph<'_>,
    sub: &Sub,
    items: &mut [NItem],
    ifn: IntersectFn,
    ortho: bool,
    is_x: bool,
) {
    let mut dt = DtTree::new(true);
    for i in 0..items.len() {
        items[i].val = if is_x { items[i].x } else { items[i].y };
        dt.insert(i, |a, b| items[a].val.cmp(&items[b].val));
    }
    let list = dt.flatten();
    let dist: DistFn = if is_x { dist_x } else { dist_y };
    let mut cg = if ortho {
        mk_constraint_g(items, &list, ifn, dist)
    } else {
        mk_n_constraint_g(g, sub, items, &list, ifn, dist)
    };
    // C ignores `rank`'s result here too (constraint.c:398, :433). `rank2` fails only where C
    // exits or crashes on overflow: `x_cutval`'s sum, or a cut value that wrapped negative
    // with no entering edge (`TB_balance` doesn't run with balance 2). Every edge here
    // weighs 1 or 100, so a cut value stays below 100 times the edge count and neither can
    // happen for a constraint graph that fits in memory.
    let _ = ns::rank(&mut cg.ns, 2, i32::MAX);
    for p in items.iter_mut() {
        let newpos = cg.ns.nodes[p.cnode].rank;
        if is_x {
            let delta = newpos.wrapping_sub(p.x);
            p.x = newpos;
            p.bb.llx = p.bb.llx.wrapping_add(delta);
            p.bb.urx = p.bb.urx.wrapping_add(delta);
        } else {
            let delta = newpos.wrapping_sub(p.y);
            p.y = newpos;
            p.bb.lly = p.bb.lly.wrapping_add(delta);
            p.bb.ury = p.bb.ury.wrapping_add(delta);
        }
    }
}

fn overlaps(items: &[NItem]) -> bool {
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let (a, b) = (&items[i].bb, &items[j].bb);
            if a.urx >= b.llx && b.urx >= a.llx && a.ury >= b.lly && b.ury >= a.lly {
                return true;
            }
        }
    }
    false
}

/// `POINTS(f)`, with `f*72 ± .5` fused as clang does.
fn points_fma(f: f64) -> i32 {
    graphviz_pure_core::cnum::c_round_mul(f, 72.0)
}

/// `POINTS(f)` for `initItem`'s `x` and `y`. The reference build converts the two with a
/// vector instruction to 64 bits (`fcvtzs.2d`) and keeps the low 32 (`xtn.2s`), so past
/// INT_MAX (about 3e6 inches at `SCALE`) they wrap modulo 2^32 rather than saturate as the
/// scalar conversion of `w2`/`h2` does (#240).
fn points_wrapping(f: f64) -> i32 {
    let v = if f * 72.0 >= 0.0 {
        math::fma(f, 72.0, 0.5)
    } else {
        math::fma(f, 72.0, -0.5)
    };
    (v as i64) as i32
}

/// `d2i`: saturating truncation.
fn d2i(v: f64) -> i32 {
    if v > i32::MAX as f64 {
        i32::MAX
    } else if v < i32::MIN as f64 {
        i32::MIN
    } else {
        v as i32
    }
}

/// `initItem(n, p, margin)`.
fn init_item(g: &LGraph<'_>, n: usize, margin: &super::adjust::Expand) -> NItem {
    let nd = &g.nodes[n];
    let x = points_wrapping(SCALE as f64 * nd.pos[0]);
    let y = points_wrapping(SCALE as f64 * nd.pos[1]);
    let (w2, h2) = if margin.do_add {
        (
            d2i(SCALE as f64 * (points_fma(nd.width / 2.0) as f64 + margin.x)),
            d2i(SCALE as f64 * (points_fma(nd.height / 2.0) as f64 + margin.y)),
        )
    } else {
        (
            points_fma(margin.x * SCALE2 as f64 * nd.width),
            points_fma(margin.y * SCALE2 as f64 * nd.height),
        )
    };
    NItem {
        val: 0,
        x,
        y,
        np: n,
        cnode: 0,
        // C's `x - w2` overflows for a far pin (#240): UB, but clang emits a plain 32-bit add,
        // which wraps.
        bb: IBox {
            llx: x.wrapping_sub(w2),
            lly: y.wrapping_sub(h2),
            urx: x.wrapping_add(w2),
            ury: y.wrapping_add(h2),
        },
    }
}

/// `cAdjust(g, mode)`: returns whether there were overlaps to remove.
pub fn c_adjust(g: &mut LGraph<'_>, sub: &Sub, mode: AdjustMode) -> bool {
    let margin = sep_factor(g);
    let mut items: Vec<NItem> = sub
        .nodes
        .iter()
        .map(|&n| init_item(g, n, &margin))
        .collect();
    if !overlaps(&items) {
        return false;
    }
    let gr: &LGraph<'_> = g;
    let it = &mut items;
    match mode {
        AdjustMode::OrthoXY => {
            constrain(gr, sub, it, intersect_y, true, true);
            constrain(gr, sub, it, intersect_x, true, false);
        }
        AdjustMode::OrthoYXrev => {
            constrain(gr, sub, it, intersect_x, true, false);
            constrain(gr, sub, it, intersect_y, true, true);
        }
        AdjustMode::Ortho => {
            constrain(gr, sub, it, intersect_y0, true, true);
            constrain(gr, sub, it, intersect_x, true, false);
        }
        AdjustMode::OrthoYX => {
            constrain(gr, sub, it, intersect_x0, true, false);
            constrain(gr, sub, it, intersect_y, true, true);
        }
        AdjustMode::POrthoXY => {
            constrain(gr, sub, it, intersect_y, false, true);
            constrain(gr, sub, it, intersect_x, false, false);
        }
        AdjustMode::POrthoYXrev => {
            constrain(gr, sub, it, intersect_x, false, false);
            constrain(gr, sub, it, intersect_y, false, true);
        }
        AdjustMode::POrthoYX => {
            constrain(gr, sub, it, intersect_x0, false, false);
            constrain(gr, sub, it, intersect_y, false, true);
        }
        _ => {
            constrain(gr, sub, it, intersect_y0, false, true);
            constrain(gr, sub, it, intersect_x, false, false);
        }
    }
    for p in &items {
        g.nodes[p.np].pos[0] = (p.x as f64 / 72.0) / SCALE as f64;
        g.nodes[p.np].pos[1] = (p.y as f64 / 72.0) / SCALE as f64;
    }
    true
}
