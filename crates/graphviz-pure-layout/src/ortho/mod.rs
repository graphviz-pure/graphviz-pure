//! Port of Graphviz `lib/ortho` (`splines=ortho`): orthogonal edge routing.
//!
//! - [`maze`] — `maze.c`: node cells, free cells from [`partition`], and the
//!   search graph over cell sides
//! - [`partition`] / [`trapezoid`] — `partition.c`, `trapezoid.c`: free-space
//!   decomposition by Seidel trapezoidation
//! - [`sgraph`] — `sgraph.c`, `fPQ.c`: search graph and Dijkstra
//! - [`rawgraph`] — `rawgraph.c`: track ordering graphs
//! - this module — `ortho.c`: routing, channel and track assignment
//! - [`fail`] — the failure boundary: how every failure inside these modules
//!   reaches [`ortho_edges_of`] and is handled there
//!
//! The router works on any layout implementing [`OrthoGraph`].

// Faithful ports of C loops index several arrays in lockstep.
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::neg_multiply,
    clippy::needless_late_init
)]

pub mod fail;
pub mod maze;
pub mod partition;
pub mod rawgraph;
pub mod sgraph;
pub mod trapezoid;

use crate::cdt::Cdt;
use crate::common::splines::{SplineGraph, SplineInfo, clip_and_install_info};
use fail::{At, Fail, Need, OResult, ensure};
use graphviz_pure_core::Point;
use maze::{Cell, M_BOTTOM, M_LEFT, M_RIGHT, M_TOP, MZ_HSCAN, MZ_VSCAN, Maze, mk_maze, update_wts};
use rawgraph::RawGraph;
use sgraph::{Pq, SGraph};
use std::cmp::Ordering;

/// The layout state `orthoEdges` needs beyond [`SplineGraph`].
pub trait OrthoGraph: SplineGraph {
    /// Nodes in `agfstnode` order.
    fn ortho_nodes(&self) -> Vec<usize>;
    /// Out-edges of `n` in `agfstout` order.
    fn ortho_out_edges(&self, n: usize) -> Vec<usize>;
    /// The `Concentrate` global.
    fn ortho_concentrate(&self) -> bool;
    /// `Nop == 2 && ED_spl(e)`: the edge keeps the spline it was given.
    fn ortho_keeps_spline(&self, _e: usize) -> bool {
        false
    }
    /// `AGSEQ(n)`.
    fn node_seq(&self, n: usize) -> usize;
    /// `agnameof(n)` (debug dumps).
    fn xl_name(&self, n: usize) -> String;
}

/// `bend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Bend {
    #[default]
    Node,
    Up,
    Left,
    Down,
    Right,
}

/// `segment`.
#[derive(Debug, Clone, Copy, Default)]
struct Segment {
    is_vert: bool,
    comm_coord: f64,
    p1: f64,
    p2: f64,
    l1: Bend,
    l2: Bend,
    ind_no: Option<usize>,
    track_no: Option<i32>,
    prev: Option<usize>,
    next: Option<usize>,
}

/// A segment of a route: (route index, segment index).
type SegRef = (usize, usize);

/// `channel`.
#[derive(Debug, Clone, Default)]
struct Channel {
    p1: f64,
    p2: f64,
    seg_list: Vec<SegRef>,
    g: RawGraph,
    cp: usize,
}

/// `chanItem`.
#[derive(Debug, Clone)]
struct ChanItem {
    v: f64,
    chans: Cdt<Channel>,
}

fn fcmp(a: f64, b: f64) -> Ordering {
    if a < b {
        Ordering::Less
    } else if a > b {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// `chancmpid(k1, k2)`: intervals compare equal when one contains the other.
fn chancmpid(k1: (f64, f64), k2: (f64, f64)) -> Ordering {
    if k1.0 > k2.0 {
        if k1.1 <= k2.1 {
            return Ordering::Equal;
        }
        return Ordering::Greater;
    }
    if k1.0 < k2.0 {
        if k1.1 >= k2.1 {
            return Ordering::Equal;
        }
        return Ordering::Less;
    }
    Ordering::Equal
}

/// A channel dictionary (`Dt_t*` of `chanItem`s, each with a `Dt_t*` of
/// channels), with cdt's splay-tree semantics.
#[derive(Default)]
struct Chans {
    items: Cdt<ChanItem>,
}

/// (chanItem node, channel node) in a [`Chans`].
type ChanKey = (usize, usize);

impl Chans {
    /// `addChan(chdict, cp, j)`.
    fn add(&mut self, chp: Channel, j: f64) {
        let ii = match self.items.find(&|it: &ChanItem| fcmp(j, it.v)) {
            Some(i) => i,
            None => {
                self.items
                    .insert(
                        ChanItem {
                            v: j,
                            chans: Cdt::new(),
                        },
                        &|it: &ChanItem| fcmp(j, it.v),
                    )
                    .0
            }
        };
        let key = (chp.p1, chp.p2);
        self.items
            .get_mut(ii)
            .chans
            .insert(chp, &|c: &Channel| chancmpid(key, (c.p1, c.p2)));
    }

    /// `chanSearch(chans, seg)`, with `None` where C's asserts fail.
    fn find(&mut self, seg: &Segment) -> Option<ChanKey> {
        let v = seg.comm_coord;
        let i = self.items.find(&|it: &ChanItem| fcmp(v, it.v))?;
        let key = (seg.p1, seg.p2);
        let c = self
            .items
            .get_mut(i)
            .chans
            .find(&|c: &Channel| chancmpid(key, (c.p1, c.p2)))?;
        Some((i, c))
    }

    /// `chanSearch(chans, seg)`, failing where C's asserts do (#170).
    fn search(&mut self, seg: &Segment) -> OResult<ChanKey> {
        self.find(seg).ok_or(Fail::NoChannel)
    }

    fn chan(&self, k: ChanKey) -> &Channel {
        self.items.get(k.0).chans.get(k.1)
    }

    fn chan_mut(&mut self, k: ChanKey) -> &mut Channel {
        self.items.get_mut(k.0).chans.get_mut(k.1)
    }
}

struct Ortho {
    mp: Maze,
    routes: Vec<Vec<Segment>>,
    hchans: Chans,
    vchans: Chans,
}

/// `cellOf(p, q)`: the cell shared by two snodes.
fn cell_of(sg: &SGraph, p: usize, q: usize) -> OResult<usize> {
    let (pn, qn) = (sg.node(p)?, sg.node(q)?);
    let cp = pn.cells[0];
    if cp == qn.cells[0] || cp == qn.cells[1] {
        return cp.need();
    }
    pn.cells[1].need()
}

fn mid(a: f64, b: f64) -> f64 {
    (a + b) / 2.0
}

fn mid_pt(cp: &Cell) -> Point {
    Point::new(mid(cp.bb.ll.x, cp.bb.ur.x), mid(cp.bb.ll.y, cp.bb.ur.y))
}

/// `sidePt(ptr, cp)`.
fn side_pt(sg: &SGraph, ptr: usize, c: usize, cp: &Cell) -> OResult<Point> {
    let n = sg.node(ptr)?;
    Ok(if Some(c) == n.cells[1] {
        if n.is_vert {
            Point::new(cp.bb.ll.x, mid(cp.bb.ll.y, cp.bb.ur.y))
        } else {
            Point::new(mid(cp.bb.ll.x, cp.bb.ur.x), cp.bb.ll.y)
        }
    } else if n.is_vert {
        Point::new(cp.bb.ur.x, mid(cp.bb.ll.y, cp.bb.ur.y))
    } else {
        Point::new(mid(cp.bb.ll.x, cp.bb.ur.x), cp.bb.ur.y)
    })
}

/// `setSeg(dir, fix, b1, b2, l1, l2)`.
fn set_seg(dir: bool, fix: f64, b1: f64, b2: f64, l1: Bend, l2: Bend) -> Segment {
    let mut sp = Segment {
        is_vert: dir,
        comm_coord: fix,
        ..Segment::default()
    };
    if b1 < b2 {
        sp.p1 = b1;
        sp.p2 = b2;
        sp.l1 = l1;
        sp.l2 = l2;
    } else {
        sp.p2 = b1;
        sp.p1 = b2;
        sp.l2 = l1;
        sp.l1 = l2;
    }
    sp
}

/// `convertSPtoRoute(g, fst, lst)`.
///
/// [`Fail::NoPath`] when `shortPath` has never reached `fst`: its `N_DAD`
/// is still NULL (`reset` does not clear it), and C dereferences it and
/// segfaults (#186). When an earlier edge did reach `fst`, C follows the
/// stale parent without crashing, and so does the port.
fn convert_sp_to_route(mp: &mut Maze, fst: usize, lst: usize) -> OResult<Vec<Segment>> {
    let dad = |sg: &SGraph, n: usize| sg.node(n).map(|np| np.n_dad);
    let mut rte = Vec::new();
    let mut ptr = dad(&mp.sg, fst)?.ok_or(Fail::NoPath)?;
    let mut prev = ptr;
    let mut next = dad(&mp.sg, ptr)?.need()?;
    let c0 = mp.sg.node(ptr)?.cells[0].need()?;
    let mut cp = if mp.cell(c0)?.is_node() {
        mp.sg.node(ptr)?.cells[1].need()?
    } else {
        c0
    };
    let mut bp1 = side_pt(&mp.sg, ptr, cp, mp.cell(cp)?)?;
    let mut prevbp = Point::new(0.0, 0.0);
    let first_dad = ptr;
    // A parent chain visits each node once; a longer one is a cycle through
    // stale parents, where C loops until memory runs out.
    let mut budget = mp.sg.nodes.len();
    while let Some(nn) = dad(&mp.sg, next)? {
        budget = budget.checked_sub(1).need()?;
        let ncp = cell_of(&mp.sg, prev, next)?;
        let pe = mp.sg.node(ptr)?.n_edge.need()?;
        let cell = mp.cell(ncp)?.clone();
        update_wts(&mut mp.sg, &cell, pe)?;
        let pv = mp.sg.node(ptr)?.is_vert;
        let nv = mp.sg.node(next)?.is_vert;
        if pv != nv || nn == lst {
            let bp2 = if pv != nv {
                mid_pt(&cell)
            } else {
                side_pt(&mp.sg, next, ncp, &cell)?
            };
            let (l1, l2, fix, b1, b2);
            if pv {
                l1 = if ptr == first_dad {
                    Bend::Node
                } else if prevbp.y > bp1.y {
                    Bend::Up
                } else {
                    Bend::Down
                };
                l2 = if pv != nv {
                    if mp.sg.node(next)?.cells[0] == Some(ncp) {
                        Bend::Up
                    } else {
                        Bend::Down
                    }
                } else {
                    Bend::Node
                };
                fix = mp.cell(cp)?.bb.ll.y;
                b1 = mp.cell(cp)?.bb.ll.x;
                b2 = cell.bb.ll.x;
            } else {
                l1 = if ptr == first_dad {
                    Bend::Node
                } else if prevbp.x > bp1.x {
                    Bend::Right
                } else {
                    Bend::Left
                };
                l2 = if pv != nv {
                    if mp.sg.node(next)?.cells[0] == Some(ncp) {
                        Bend::Right
                    } else {
                        Bend::Left
                    }
                } else {
                    Bend::Node
                };
                fix = mp.cell(cp)?.bb.ll.x;
                b1 = mp.cell(cp)?.bb.ll.y;
                b2 = cell.bb.ll.y;
            }
            rte.push(set_seg(!pv, fix, b1, b2, l1, l2));
            cp = ncp;
            prevbp = bp1;
            bp1 = bp2;
            if pv != nv && nn == lst {
                let (l1, fix, b1, b2);
                if nv {
                    l1 = if prevbp.y > bp1.y {
                        Bend::Up
                    } else {
                        Bend::Down
                    };
                    fix = mp.cell(cp)?.bb.ll.y;
                    b1 = mp.cell(cp)?.bb.ll.x;
                    b2 = cell.bb.ll.x;
                } else {
                    l1 = if prevbp.x > bp1.x {
                        Bend::Right
                    } else {
                        Bend::Left
                    };
                    fix = mp.cell(cp)?.bb.ll.x;
                    b1 = mp.cell(cp)?.bb.ll.y;
                    b2 = cell.bb.ll.y;
                }
                rte.push(set_seg(!nv, fix, b1, b2, l1, Bend::Node));
            }
            ptr = next;
        }
        prev = next;
        next = nn;
    }
    let n = rte.len();
    for i in 0..n {
        if i > 0 {
            rte[i].prev = Some(i - 1);
        }
        if i + 1 < n {
            rte[i].next = Some(i + 1);
        }
    }
    Ok(rte)
}

/// `extractHChans(mp)` / `extractVChans(mp)`.
///
/// The walks move one way across the maze, so neither passes more cells than
/// there are; a longer one is a cycle in a broken maze, where C loops.
fn extract_chans(mp: &mut Maze, horizontal: bool) -> OResult<Chans> {
    let mut chans = Chans::default();
    let (back, fwd, flag) = if horizontal {
        (M_LEFT, M_RIGHT, MZ_HSCAN)
    } else {
        (M_BOTTOM, M_TOP, MZ_VSCAN)
    };
    for i in mp.ngcells..mp.cells.len() {
        let mut cp = i;
        if mp.cells[cp].flags & flag != 0 {
            continue;
        }
        let mut budget = mp.cells.len();
        while let Some(np) = *mp.cell(cp)?.sides.at(back)? {
            budget = budget.checked_sub(1).need()?;
            match mp.sg.node(np)?.cells[0] {
                Some(nc) if !mp.cell(nc)?.is_node() => cp = nc,
                _ => break,
            }
        }
        let start = cp;
        let p1 = if horizontal {
            mp.cells[cp].bb.ll.x
        } else {
            mp.cells[cp].bb.ll.y
        };
        mp.cells[cp].flags |= flag;
        let mut budget = mp.cells.len();
        while let Some(np) = *mp.cell(cp)?.sides.at(fwd)? {
            budget = budget.checked_sub(1).need()?;
            match mp.sg.node(np)?.cells[1] {
                Some(nc) if !mp.cell(nc)?.is_node() => {
                    cp = nc;
                    mp.cells[cp].flags |= flag;
                }
                _ => break,
            }
        }
        let p2 = if horizontal {
            mp.cells[cp].bb.ur.x
        } else {
            mp.cells[cp].bb.ur.y
        };
        let key = if horizontal {
            mp.cells[start].bb.ll.y
        } else {
            mp.cells[start].bb.ll.x
        };
        chans.add(
            Channel {
                p1,
                p2,
                cp: start,
                ..Channel::default()
            },
            key,
        );
    }
    Ok(chans)
}

fn eq_end_seg(s1l2: Bend, s2l2: Bend, t1: Bend, t2: Bend) -> i32 {
    if (s1l2 == t2 && s2l2 != t2) || (s1l2 == Bend::Node && s2l2 == t1) {
        0
    } else {
        -1
    }
}

fn overlap_seg(s1: &Segment, s2: &Segment, t1: Bend, t2: Bend) -> i32 {
    if s1.p2 < s2.p2 {
        if s1.l2 == t1 && s2.l1 == t2 {
            return -1;
        }
        if s1.l2 == t2 && s2.l1 == t1 {
            return 1;
        }
        return 0;
    }
    if s1.p2 > s2.p2 {
        if s2.l1 == t2 && s2.l2 == t2 {
            return -1;
        }
        if s2.l1 == t1 && s2.l2 == t1 {
            return 1;
        }
        return 0;
    }
    if s2.l1 == t2 {
        return eq_end_seg(s1.l2, s2.l2, t1, t2);
    }
    -eq_end_seg(s2.l2, s1.l2, t1, t2)
}

fn ell_seg(s1l1: Bend, s1l2: Bend, t: Bend) -> i32 {
    if s1l1 == t {
        if s1l2 == t {
            return -1;
        }
        return 0;
    }
    1
}

fn seg_cmp_(s1: &Segment, s2: &Segment, t1: Bend, t2: Bend) -> i32 {
    if s1.p2 < s2.p1 || s1.p1 > s2.p2 {
        return 0;
    }
    if s1.p1 < s2.p1 && s2.p1 < s1.p2 {
        return overlap_seg(s1, s2, t1, t2);
    }
    if s2.p1 < s1.p1 && s1.p1 < s2.p2 {
        return -overlap_seg(s2, s1, t1, t2);
    }
    if s1.p1 == s2.p1 {
        if s1.p2 < s2.p2 {
            if s1.l2 == t1 {
                return eq_end_seg(s2.l1, s1.l1, t1, t2);
            }
            return -eq_end_seg(s2.l1, s1.l1, t1, t2);
        }
        if s1.p2 > s2.p2 {
            if s2.l2 == t2 {
                return eq_end_seg(s1.l1, s2.l1, t1, t2);
            }
            return -eq_end_seg(s1.l1, s2.l1, t1, t2);
        }
        if s1.l1 == s2.l1 && s1.l2 == s2.l2 {
            return 0;
        }
        if s2.l1 == s2.l2 {
            if s2.l1 == t1 {
                return 1;
            }
            if s2.l1 == t2 {
                return -1;
            }
            if s1.l1 != t1 && s1.l2 != t1 {
                return 1;
            }
            if s1.l1 != t2 && s1.l2 != t2 {
                return -1;
            }
            return 0;
        }
        if s2.l1 == t1 && s2.l2 == t2 {
            if s1.l1 != t1 && s1.l2 == t2 {
                return 1;
            }
            if s1.l1 == t1 && s1.l2 != t2 {
                return -1;
            }
            return 0;
        }
        if s2.l2 == t1 && s2.l1 == t2 {
            if s1.l2 != t1 && s1.l1 == t2 {
                return 1;
            }
            if s1.l2 == t1 && s1.l1 != t2 {
                return -1;
            }
            return 0;
        }
        if s2.l1 == Bend::Node && s2.l2 == t1 {
            return ell_seg(s1.l1, s1.l2, t1);
        }
        if s2.l1 == Bend::Node && s2.l2 == t2 {
            return -ell_seg(s1.l1, s1.l2, t2);
        }
        if s2.l1 == t1 && s2.l2 == Bend::Node {
            return ell_seg(s1.l2, s1.l1, t1);
        }
        return -ell_seg(s1.l2, s1.l1, t2);
    }
    if s1.p2 == s2.p1 {
        if s1.l2 == s2.l1 {
            return 0;
        }
        if s1.l2 == t2 {
            return 1;
        }
        return -1;
    }
    if s1.l1 == s2.l2 {
        return 0;
    }
    if s1.l1 == t2 {
        return 1;
    }
    -1
}

/// `seg_cmp(S1, S2)`; C returns -2 for incomparable segments, which fails
/// `assignTracks`.
fn seg_cmp(s1: &Segment, s2: &Segment) -> OResult<i32> {
    if s1.is_vert != s2.is_vert || s1.comm_coord != s2.comm_coord {
        return Err(Fail::Incomparable);
    }
    Ok(if s1.is_vert {
        seg_cmp_(s1, s2, Bend::Right, Bend::Left)
    } else {
        seg_cmp_(s1, s2, Bend::Down, Bend::Up)
    })
}

/// `is_parallel(s1, s2)`, with C's `assert(s1.comm_coord == s2.comm_coord)`.
fn is_parallel(s1: &Segment, s2: &Segment) -> OResult<bool> {
    ensure(s1.comm_coord == s2.comm_coord)?;
    Ok(s1.p1 == s2.p1 && s1.p2 == s2.p2 && s1.l1 == s2.l1 && s1.l2 == s2.l2)
}

impl Ortho {
    fn seg(&self, r: SegRef) -> &Segment {
        &self.routes[r.0][r.1]
    }

    fn next_seg(&self, r: SegRef, dir: i32) -> Option<SegRef> {
        let s = self.seg(r);
        let n = if dir == 0 { s.prev } else { s.next };
        n.map(|i| (r.0, i))
    }

    fn chans_mut(&mut self, vert: bool) -> &mut Chans {
        if vert {
            &mut self.vchans
        } else {
            &mut self.hchans
        }
    }

    fn chan_key(&mut self, r: SegRef) -> OResult<(bool, ChanKey)> {
        let s = *self.seg(r);
        Ok((s.is_vert, self.chans_mut(s.is_vert).search(&s)?))
    }

    /// The nested `dtflatten`/`dtlink` loop over a channel dictionary. The
    /// `dtlink` pointers are read after each body, which may have searched
    /// (and so restored and splayed) the same dictionaries, as in C.
    fn for_each_chan(
        &mut self,
        vert: bool,
        f: &mut dyn FnMut(&mut Ortho, ChanKey) -> OResult<()>,
    ) -> OResult<()> {
        let mut l1 = self.chans_mut(vert).items.flatten();
        while let Some(i) = l1 {
            let mut l2 = self.chans_mut(vert).items.get_mut(i).chans.flatten();
            while let Some(j) = l2 {
                f(self, (i, j))?;
                l2 = self.chans_for_kind(vert).items.get(i).chans.link(j);
            }
            l1 = self.chans_for_kind(vert).items.link(i);
        }
        Ok(())
    }

    fn chan_g(&mut self, k: (bool, (usize, usize))) -> &mut RawGraph {
        if k.0 {
            &mut self.vchans.chan_mut(k.1).g
        } else {
            &mut self.hchans.chan_mut(k.1).g
        }
    }

    fn chan_g_ref(&self, k: (bool, (usize, usize))) -> &RawGraph {
        if k.0 {
            &self.vchans.chan(k.1).g
        } else {
            &self.hchans.chan(k.1).g
        }
    }

    /// `assignSegs(nrtes, route_list, mp)`. Fails where C's `chanSearch`
    /// fails `assert (chani)` or `assert (cp)` (ortho.c:388/390) and aborts
    /// (#170): a segment of a route lies in no channel of the maze.
    fn assign_segs(&mut self) -> OResult<()> {
        for i in 0..self.routes.len() {
            for j in 0..self.routes[i].len() {
                let s = self.routes[i][j];
                let chans = if s.is_vert {
                    &mut self.vchans
                } else {
                    &mut self.hchans
                };
                let k = chans.search(&s)?;
                let chan = chans.chan_mut(k);
                let ind = chan.seg_list.len();
                chan.seg_list.push((i, j));
                if std::env::var_os("GVDUMP").is_some() {
                    let (p1, p2) = (chan.p1, chan.p2);
                    let s = &self.routes[i][j];
                    graphviz_pure_core::diag::dump(&format!(
                        "OAS {i} {j} {} {:.6} [{p1:.6} {p2:.6}] {ind}",
                        i32::from(s.is_vert),
                        s.comm_coord
                    ));
                }
                self.routes[i][j].ind_no = Some(ind);
            }
        }
        Ok(())
    }

    /// `add_edges_in_G(cp)`.
    fn add_edges_in_g(&mut self, vert: bool, k: (usize, usize)) -> OResult<()> {
        let chans = if vert { &self.vchans } else { &self.hchans };
        let segs = chans.chan(k).seg_list.clone();
        let size = segs.len();
        for x in 0..size.saturating_sub(1) {
            for y in x + 1..size {
                let cmp = seg_cmp(self.seg(segs[x]), self.seg(segs[y]))?;
                let g = self.chan_g((vert, k));
                if cmp > 0 {
                    g.insert_edge(x, y)?;
                } else if cmp == -1 {
                    g.insert_edge(y, x)?;
                }
            }
        }
        Ok(())
    }

    /// `propagate_prec(seg, prec, hops, dir)`.
    fn propagate_prec(&self, seg: SegRef, prec: i32, hops: i32, dir: i32) -> OResult<i32> {
        let mut ans = prec;
        let mut current = seg;
        for _ in 1..=hops {
            let next = self.next_seg(current, dir).need()?;
            let c = *self.seg(current);
            let n = self.seg(next);
            if !c.is_vert {
                if n.comm_coord == c.p1 {
                    if c.l1 == Bend::Up {
                        ans *= -1;
                    }
                } else if c.l2 == Bend::Down {
                    ans *= -1;
                }
            } else if n.comm_coord == c.p1 {
                if c.l1 == Bend::Right {
                    ans *= -1;
                }
            } else if c.l2 == Bend::Left {
                ans *= -1;
            }
            current = next;
        }
        Ok(ans)
    }

    /// `decide_point(ret, si, sj, dir1, dir2)`: (hops, prec).
    fn decide_point(
        &self,
        mut si: SegRef,
        mut sj: SegRef,
        dir1: i32,
        dir2: i32,
    ) -> OResult<(i32, i32)> {
        let mut ans = 0;
        let mut np1;
        let mut np2 = None;
        loop {
            np1 = self.next_seg(si, dir1);
            let Some(a) = np1 else { break };
            np2 = self.next_seg(sj, dir2);
            let Some(b) = np2 else { break };
            if !is_parallel(self.seg(a), self.seg(b))? {
                break;
            }
            ans += 1;
            si = a;
            sj = b;
        }
        let prec = match (np1, np2) {
            (None, _) => 0,
            // C: `assert(0); /* FIXME */`.
            (Some(_), None) => return Err(Fail::here()),
            (Some(a), Some(b)) => {
                let temp = seg_cmp(self.seg(a), self.seg(b))?;
                self.propagate_prec(a, temp, ans + 1, 1 - dir1)?
            }
        };
        Ok((ans, prec))
    }

    /// `set_parallel_edges(seg1, seg2, dir1, dir2, hops, mp)`.
    fn set_parallel_edges(
        &mut self,
        mut seg1: SegRef,
        mut seg2: SegRef,
        dir1: i32,
        dir2: i32,
        hops: i32,
    ) -> OResult<()> {
        let mut chan = self.chan_key(seg1)?;
        let (i1, i2) = (self.seg(seg1).ind_no.need()?, self.seg(seg2).ind_no.need()?);
        self.chan_g(chan).insert_edge(i1, i2)?;
        for _ in 1..=hops {
            let prev1 = self.next_seg(seg1, dir1).need()?;
            let prev2 = self.next_seg(seg2, dir2).need()?;
            let s1 = *self.seg(seg1);
            let (a, b) = (s1.ind_no.need()?, self.seg(seg2).ind_no.need()?);
            let (p1i, p2i) = (
                self.seg(prev1).ind_no.need()?,
                self.seg(prev2).ind_no.need()?,
            );
            let nchan = (!s1.is_vert, {
                let p = *self.seg(prev1);
                self.chans_mut(!s1.is_vert).search(&p)?
            });
            let exists = self.chan_g_ref(chan).edge_exists(a, b)?;
            let at_p1 = self.seg(prev1).comm_coord == s1.p1;
            let first_bend = if !s1.is_vert { Bend::Up } else { Bend::Left };
            // forward: insert prev1 -> prev2; else prev2 -> prev1
            let forward = if at_p1 {
                let hit = s1.l1 == first_bend;
                if !s1.is_vert {
                    // B_UP: exists ? (p2,p1) : (p1,p2); else exists ? (p1,p2) : (p2,p1)
                    if hit { !exists } else { exists }
                } else {
                    // B_LEFT: exists ? (p1,p2) : (p2,p1); else exists ? (p2,p1) : (p1,p2)
                    if hit { exists } else { !exists }
                }
            } else {
                let hit = s1.l2 == first_bend;
                if !s1.is_vert {
                    if hit { exists } else { !exists }
                } else if hit {
                    !exists
                } else {
                    exists
                }
            };
            let g = self.chan_g(nchan);
            if forward {
                g.insert_edge(p1i, p2i)?;
            } else {
                g.insert_edge(p2i, p1i)?;
            }
            if std::env::var_os("GVDUMP").is_some() {
                let g = self.chan_g_ref(nchan);
                graphviz_pure_core::diag::dump(&format!(
                    "OSP {:.6} {p1i} {p2i} {} {}",
                    self.seg(prev1).comm_coord,
                    i32::from(g.edge_exists(p1i, p2i)?),
                    i32::from(g.edge_exists(p2i, p1i)?)
                ));
            }
            chan = nchan;
            seg1 = prev1;
            seg2 = prev2;
        }
        Ok(())
    }

    fn chans_for_kind(&self, vert: bool) -> &Chans {
        if vert { &self.vchans } else { &self.hchans }
    }

    /// `removeEdge(seg1, seg2, dir, mp)`.
    fn remove_edge(&mut self, seg1: SegRef, seg2: SegRef, dir: i32) -> OResult<()> {
        let mut ptr1 = seg1;
        let mut ptr2 = seg2;
        while is_parallel(self.seg(ptr1), self.seg(ptr2))? {
            ptr1 = self.next_seg(ptr1, 1).need()?;
            ptr2 = self.next_seg(ptr2, dir).need()?;
        }
        let chan = self.chan_key(ptr1)?;
        let (a, b) = (self.seg(ptr1).ind_no.need()?, self.seg(ptr2).ind_no.need()?);
        self.chan_g(chan).remove_redge(a, b)
    }

    /// `addPEdges(cp, mp)`.
    fn add_p_edges_chan(&mut self, vert: bool, k: (usize, usize)) -> OResult<()> {
        let segs = self.chans_for_kind(vert).chan(k).seg_list.clone();
        let n = segs.len();
        for i in 0..n.saturating_sub(1) {
            for j in i + 1..n {
                let g = self.chan_g_ref((vert, k));
                if g.edge_exists(i, j)? || g.edge_exists(j, i)? {
                    continue;
                }
                let (si, sj) = (segs[i], segs[j]);
                if !is_parallel(self.seg(si), self.seg(sj))? {
                    continue;
                }
                let (pi, pj) = (self.seg(si).prev, self.seg(sj).prev);
                let dir = match (pi, pj) {
                    (None, None) => 0,
                    (None, Some(_)) | (Some(_), None) => 1,
                    (Some(a), Some(b)) => {
                        let ri = self.routes[si.0].at(a)?.comm_coord;
                        if ri == self.routes[sj.0].at(b)?.comm_coord {
                            0
                        } else {
                            1
                        }
                    }
                };
                let (h1, prec1) = self.decide_point(si, sj, 0, dir)?;
                let (h2, prec2) = self.decide_point(si, sj, 1, 1 - dir)?;
                if std::env::var_os("GVDUMP").is_some() {
                    graphviz_pure_core::diag::dump(&format!(
                        "OPE {:.6} {i} {j} {dir} {h1} {prec1} {h2} {prec2}",
                        self.seg(si).comm_coord
                    ));
                }
                if prec1 == -1 {
                    self.set_parallel_edges(sj, si, dir, 0, h1)?;
                    self.set_parallel_edges(sj, si, 1 - dir, 1, h2)?;
                    if prec2 == 1 {
                        self.remove_edge(si, sj, 1 - dir)?;
                    }
                } else if prec1 == 0 {
                    if prec2 == -1 {
                        self.set_parallel_edges(sj, si, dir, 0, h1)?;
                        self.set_parallel_edges(sj, si, 1 - dir, 1, h2)?;
                    } else if prec2 == 0 || prec2 == 1 {
                        self.set_parallel_edges(si, sj, 0, dir, h1)?;
                        self.set_parallel_edges(si, sj, 1, 1 - dir, h2)?;
                    }
                } else if prec1 == 1 {
                    self.set_parallel_edges(si, sj, 0, dir, h1)?;
                    self.set_parallel_edges(si, sj, 1, 1 - dir, h2)?;
                    if prec2 == -1 {
                        self.remove_edge(si, sj, 1 - dir)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Debug dump of the channel graphs (`GVDUMP`).
    fn dump_chans(&mut self, tag: &str) {
        if std::env::var_os("GVDUMP").is_none() {
            return;
        }
        for vert in [false, true] {
            self.for_each_chan(vert, &mut |o, k| {
                let chans = o.chans_for_kind(vert);
                let ch = chans.chan(k);
                if ch.seg_list.is_empty() {
                    return Ok(());
                }
                let mut l = format!(
                    "OCG {tag} {:.6} [{:.6} {:.6}]:",
                    chans.items.get(k.0).v,
                    ch.p1,
                    ch.p2
                );
                for (i, v) in ch.g.vertices.iter().enumerate() {
                    l.push_str(&format!(" {i}->"));
                    for a in &v.adj_list {
                        l.push_str(&format!("{a},"));
                    }
                }
                graphviz_pure_core::diag::dump(&l);
                Ok(())
            })
            .ok();
        }
    }

    /// `assignTracks(mp)`.
    fn assign_tracks(&mut self) -> OResult<()> {
        // create_graphs
        for vert in [false, true] {
            self.for_each_chan(vert, &mut |o, k| {
                let ch = o.chans_mut(vert).chan_mut(k);
                ch.g = RawGraph::new(ch.seg_list.len());
                Ok(())
            })?;
        }
        // add_np_edges
        for vert in [false, true] {
            self.for_each_chan(vert, &mut |o, k| {
                if o.chans_for_kind(vert).chan(k).seg_list.is_empty() {
                    return Ok(());
                }
                o.add_edges_in_g(vert, k)
            })?;
        }
        self.dump_chans("np");
        // add_p_edges
        for vert in [false, true] {
            self.for_each_chan(vert, &mut |o, k| o.add_p_edges_chan(vert, k))?;
        }
        self.dump_chans("x");
        // assignTrackNo
        for vert in [false, true] {
            self.for_each_chan(vert, &mut |o, k| {
                let ch = o.chans_mut(vert).chan_mut(k);
                if ch.seg_list.is_empty() {
                    return Ok(());
                }
                ch.g.top_sort()?;
                let orders: Vec<(SegRef, i32)> = ch
                    .seg_list
                    .iter()
                    .enumerate()
                    .map(|(i, &r)| (r, ch.g.vertices[i].topsort_order + 1))
                    .collect();
                for (r, t) in orders {
                    o.routes[r.0][r.1].track_no = Some(t);
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// `vtrack(seg, m)`.
    fn vtrack(&mut self, r: SegRef) -> OResult<f64> {
        let s = *self.seg(r);
        let k = self.vchans.search(&s)?;
        let chp = self.vchans.chan(k);
        let f = f64::from(s.track_no.need()?) / (chp.seg_list.len() as f64 + 1.0);
        let bb = self.mp.cell(chp.cp)?.bb;
        Ok(f.mul_add(bb.ur.x - bb.ll.x, bb.ll.x))
    }

    /// `htrack(seg, m)`.
    fn htrack(&mut self, r: SegRef) -> OResult<f64> {
        let s = *self.seg(r);
        let k = self.hchans.search(&s)?;
        let chp = self.hchans.chan(k);
        let f = 1.0 - f64::from(s.track_no.need()?) / (chp.seg_list.len() as f64 + 1.0);
        let bb = self.mp.cell(chp.cp)?.bb;
        let (lo, hi) = (bb.ll.y, bb.ur.y);
        Ok(f.mul_add(hi - lo, lo).round())
    }
}

/// `addLoop(sg, cp, dp, sp)`.
fn add_loop(sg: &mut SGraph, cp: &Cell, c: usize, dp: usize, sp: usize) -> OResult<()> {
    for onp in cp.sides.iter().flatten().copied() {
        let on = sg.node(onp)?;
        if on.is_vert {
            continue;
        }
        let on_top = on.cells[0] == Some(c);
        if on_top {
            sg.create_sedge(sp, onp, 0.0)?;
        } else {
            sg.create_sedge(dp, onp, 0.0)?;
        }
    }
    sg.nnodes += 2;
    Ok(())
}

/// `addNodeEdges(sg, cp, np)`.
fn add_node_edges(sg: &mut SGraph, cp: &Cell, c: usize, np: usize) -> OResult<()> {
    for onp in cp.sides.iter().flatten().copied() {
        sg.create_sedge(np, onp, 0.0)?;
    }
    sg.nnodes += 1;
    sg.node_mut(np)?.cells = [Some(c), Some(c)];
    Ok(())
}

/// `orthoEdges(g, useLbls)`: route every edge of `g` orthogonally. Returns
/// false where C returns non-zero (the maze could not be built).
pub fn ortho_edges<S: OrthoGraph + ?Sized>(g: &mut S, use_lbls: bool) -> bool {
    let nodes = g.ortho_nodes();
    ortho_edges_of(g, nodes, use_lbls)
}

/// `orthoEdges(sg, useLbls)` on the subgraph made of `nodes` (in `agfstnode`
/// order) and their out-edges, e.g. one packed component.
///
/// This is the failure boundary of the router (see [`fail`]): nothing below
/// it panics, and each [`Fail`] is handled here, once. A failure while the
/// maze is built returns false, as C's `orthoEdges` returns -1 when `mkMaze`
/// returns NULL. A failure while edges are routed leaves every edge unrouted
/// and returns true, as C's `goto orthofinish` does when `shortPath` or
/// `assignTracks` fails; the callers then draw straight edges (neato family)
/// or lose the edges (dot), as they do after C's failures.
pub fn ortho_edges_of<S: OrthoGraph + ?Sized>(
    g: &mut S,
    nodes: Vec<usize>,
    use_lbls: bool,
) -> bool {
    // lib/ortho/ortho.c:orthoEdges: label routing is not implemented, so useLbls
    // only warns and is then cleared.
    if use_lbls {
        graphviz_pure_core::diag::agwarningf(
            "Orthogonal edges do not currently handle edge labels. Try using xlabels.\n",
        );
    }
    let boxes: Vec<(Point, f64, f64)> = nodes
        .iter()
        .map(|&n| (g.coord(n), g.lw(n) + g.rw(n), g.ht(n)))
        .collect();
    let mp = match mk_maze(&boxes) {
        Ok(mp) => mp,
        Err(fail) => {
            dump_fail("mkMaze", fail);
            if let Some(msg) = fail.c_message() {
                graphviz_pure_core::diag::fprintf_stderr(&msg);
            }
            return false;
        }
    };
    match route_edges(g, &nodes, mp) {
        Ok(routes) => {
            // attachOrthoEdges
            let sinfo = SplineInfo {
                ignore_swap: true,
                is_ortho: true,
            };
            for (e, ispline) in routes {
                let h = g.head(e);
                if graphviz_pure_core::diag::verbose_byte() > 1 {
                    graphviz_pure_core::diag::fprintf_stderr(&format!(
                        "ortho {} {}\n",
                        g.xl_name(g.tail(e)),
                        g.xl_name(h)
                    ));
                }
                clip_and_install_info(g, e, h, &ispline, sinfo);
            }
        }
        Err(fail) => {
            dump_fail("route", fail);
            let msg = match fail {
                Fail::HeapOverflow => {
                    // PQ_insert (fPQ.c)
                    graphviz_pure_core::diag::agerrorf("Heap overflow\n");
                    return true;
                }
                Fail::Incomparable => {
                    // seg_cmp (ortho.c)
                    graphviz_pure_core::diag::agerrorf("incomparable segments !! -- Aborting\n");
                    return true;
                }
                Fail::NoPath => "orthogonal routing found no path for an edge",
                Fail::NoChannel => "orthogonal routing found no channel for an edge segment",
                Fail::Segment
                | Fail::Trapezoidation { .. }
                | Fail::NoFreeCell
                | Fail::NoCell { .. }
                | Fail::Broken(_) => "orthogonal routing failed",
            };
            // C crashes here (#186, #170 and the invariants it asserts or
            // trusts): warn, and leave the edges unrouted.
            graphviz_pure_core::diag::agwarningf(&format!(
                "{msg} - edges not routed orthogonally\n"
            ));
        }
    }
    true
}

/// The failure and the port's source line, for `GVDUMP` debugging.
fn dump_fail(stage: &str, fail: Fail) {
    if std::env::var_os("GVDUMP").is_some() {
        graphviz_pure_core::diag::dump(&format!("OFAIL {stage} {fail:?}"));
    }
}

/// The body of `orthoEdges` after `mkMaze`: the edges of `nodes` in C's
/// order, each with the polyline `attachOrthoEdges` gives it. Nothing is
/// installed until every edge is routed, so a failure leaves all of them
/// unrouted, as C's `goto orthofinish` does.
fn route_edges<S: OrthoGraph + ?Sized>(
    g: &S,
    nodes: &[usize],
    mp: Maze,
) -> OResult<Vec<(usize, Vec<Point>)>> {
    let node_cell: std::collections::HashMap<usize, usize> =
        nodes.iter().enumerate().map(|(i, &n)| (n, i)).collect();
    let concentrate = g.ortho_concentrate();
    let mut ps: std::collections::HashSet<(usize, usize)> = Default::default();
    let mut es: Vec<(f64, usize)> = Vec::new();
    for &n in nodes {
        for e in g.ortho_out_edges(n) {
            if g.ortho_keeps_spline(e) {
                continue;
            }
            if concentrate {
                let ti = g.node_seq(g.tail(e));
                let hi = g.node_seq(g.head(e));
                let key = if ti <= hi { (ti, hi) } else { (hi, ti) };
                if !ps.insert(key) {
                    continue;
                }
            }
            let p = g.coord(g.tail(e));
            let q = g.coord(g.head(e));
            let d = (p.x - q.x).mul_add(p.x - q.x, (p.y - q.y) * (p.y - q.y));
            es.push((d, e));
        }
    }
    crate::cutil::qsort_by(&mut es, |a, b| {
        if a.0 > b.0 {
            1
        } else if a.0 < b.0 {
            -1
        } else {
            0
        }
    });
    let dump = std::env::var_os("GVDUMP").is_some();
    if dump {
        for &(d, e) in &es {
            graphviz_pure_core::diag::dump(&format!(
                "OE {} {} {:.6}",
                g.xl_name(g.tail(e)),
                g.xl_name(g.head(e)),
                d
            ));
        }
    }
    let mut o = Ortho {
        mp,
        routes: Vec::with_capacity(es.len()),
        hchans: Chans::default(),
        vchans: Chans::default(),
    };
    let gstart = o.mp.sg.nnodes;
    let mut pq = Pq::new(o.mp.sg.nnodes + 2);
    let (sn, dn) = (gstart, gstart + 1);
    for &(_, e) in &es {
        // `CELL(n)`: an end outside the maze has no cell (C reads garbage).
        let start = *node_cell.get(&g.tail(e)).need()?;
        let dest = *node_cell.get(&g.head(e)).need()?;
        if start == dest {
            let cell = o.mp.cell(start)?.clone();
            add_loop(&mut o.mp.sg, &cell, start, dn, sn)?;
        } else {
            let cd = o.mp.cell(dest)?.clone();
            add_node_edges(&mut o.mp.sg, &cd, dest, dn)?;
            let cs = o.mp.cell(start)?.clone();
            add_node_edges(&mut o.mp.sg, &cs, start, sn)?;
        }
        o.mp.sg.short_path(&mut pq, dn, sn)?;
        if dump {
            let mut l = format!("OSPN {}:", o.routes.len());
            let mut x = Some(sn);
            // Bounded: a stale parent chain can cycle.
            for _ in 0..=o.mp.sg.nodes.len() {
                let Some(v) = x.and_then(|v| o.mp.sg.nodes.get(v)) else {
                    break;
                };
                l.push_str(&format!(" {}/{}", v.index, v.n_val));
                x = v.n_dad;
            }
            graphviz_pure_core::diag::dump(&l);
        }
        let r = convert_sp_to_route(&mut o.mp, sn, dn)?;
        if dump {
            let mut l = format!("OR {}:", o.routes.len());
            for sg in &r {
                l.push_str(&format!(
                    " [{} {:?} {:?} {:?} {} {}]",
                    i32::from(sg.is_vert),
                    sg.comm_coord,
                    sg.p1,
                    sg.p2,
                    sg.l1 as i32,
                    sg.l2 as i32
                ));
            }
            graphviz_pure_core::diag::dump(&l);
        }
        o.routes.push(r);
        o.mp.sg.reset()?;
    }
    o.hchans = extract_chans(&mut o.mp, true)?;
    o.vchans = extract_chans(&mut o.mp, false)?;
    o.assign_segs()?;
    o.assign_tracks()?;
    if dump {
        for (i, r) in o.routes.iter().enumerate() {
            let mut l = format!("OT {i}:");
            for sg in r {
                l.push_str(&format!(
                    " {}/{}",
                    sg.ind_no.unwrap_or(0),
                    sg.track_no.unwrap_or(0)
                ));
            }
            graphviz_pure_core::diag::dump(&l);
        }
    }
    // attachOrthoEdges, up to `clip_and_install`
    let mut splines = Vec::with_capacity(es.len());
    for (irte, &(_, e)) in es.iter().enumerate() {
        let p1 = crate::common::geom::add(g.coord(g.tail(e)), g.tail_port(e).p);
        let q1 = crate::common::geom::add(g.coord(g.head(e)), g.head_port(e).p);
        let n = o.routes[irte].len();
        if n == 0 {
            continue;
        }
        let mut ispline = Vec::with_capacity(1 + 3 * n);
        let s0 = (irte, 0);
        let mut p = if o.seg(s0).is_vert {
            Point::new(o.vtrack(s0)?, p1.y)
        } else {
            Point::new(p1.x, o.htrack(s0)?)
        };
        ispline.push(p);
        ispline.push(p);
        let mut last = s0;
        for i in 1..n {
            let s = (irte, i);
            if o.seg(s).is_vert {
                p.x = o.vtrack(s)?;
            } else {
                p.y = o.htrack(s)?;
            }
            ispline.push(p);
            ispline.push(p);
            ispline.push(p);
            last = s;
        }
        p = if o.seg(last).is_vert {
            Point::new(o.vtrack(last)?, q1.y)
        } else {
            Point::new(q1.x, o.htrack(last)?)
        };
        ispline.push(p);
        ispline.push(p);
        splines.push((e, ispline));
    }
    Ok(splines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(centres: &[(f64, f64)]) -> Vec<(Point, f64, f64)> {
        centres
            .iter()
            .map(|&(x, y)| (Point::new(x, y), 54.0, 36.0))
            .collect()
    }

    /// `true` if `fail` is a broken invariant found in `file`.
    fn broken_in(fail: Fail, file: &str) -> bool {
        matches!(fail, Fail::Broken(l) if l.file().ends_with(file))
    }

    /// Issue #229: the boxes neato gives `a [pos="0,0!"]` and
    /// `x [pos="1e12,0!"]`. The free cells between them are 7.2e13 points
    /// wide, so `shortPath`'s distances do not fit an `int`; C fails
    /// `PQcheck`, the port overflowed `N_VAL(n) *= -1`. Now `shortPath` fails,
    /// and `orthoEdges` leaves the edge unrouted.
    #[test]
    fn huge_cells_fail_the_short_path() {
        crate::cutil::in_fresh_run(|| {
            let mut mp = mk_maze(&boxes(&[(27.0, 18.0), (7.2e13, 18.0)])).unwrap();
            let gstart = mp.sg.nnodes;
            let mut pq = Pq::new(gstart + 2);
            let (sn, dn) = (gstart, gstart + 1);
            let (c0, c1) = (mp.cells[0].clone(), mp.cells[1].clone());
            add_node_edges(&mut mp.sg, &c1, 1, dn).unwrap();
            add_node_edges(&mut mp.sg, &c0, 0, sn).unwrap();
            let fail = mp.sg.short_path(&mut pq, dn, sn).unwrap_err();
            assert!(broken_in(fail, "sgraph.rs"), "{fail:?}");
        });
    }

    /// The same edge 7.2e8 points long routes: its distances fit an `int`.
    #[test]
    fn long_cells_within_int_range_route() {
        crate::cutil::in_fresh_run(|| {
            let mut mp = mk_maze(&boxes(&[(27.0, 18.0), (7.2e8, 18.0)])).unwrap();
            let gstart = mp.sg.nnodes;
            let mut pq = Pq::new(gstart + 2);
            let (sn, dn) = (gstart, gstart + 1);
            let (c0, c1) = (mp.cells[0].clone(), mp.cells[1].clone());
            add_node_edges(&mut mp.sg, &c1, 1, dn).unwrap();
            add_node_edges(&mut mp.sg, &c0, 0, sn).unwrap();
            mp.sg.short_path(&mut pq, dn, sn).unwrap();
            assert!(!convert_sp_to_route(&mut mp, sn, dn).unwrap().is_empty());
        });
    }

    /// Issue #228: neato's NaN pin makes both node positions NaN. No
    /// trapezoid lies inside the polygon, and C's `monotonate_trapezoids`
    /// reads `LIST_GET(tr, tr_start)` one past the end and fails its
    /// assertion (C's backtrace: `partition` <- `mkMaze`). The port's maze
    /// fails there.
    #[test]
    fn nan_node_boxes_fail_the_maze_in_partition() {
        crate::cutil::in_fresh_run(|| {
            let fail = mk_maze(&boxes(&[(f64::NAN, 18.0), (f64::NAN, 18.0)])).unwrap_err();
            assert!(broken_in(fail, "partition.rs"), "{fail:?}");
        });
    }

    /// One non-finite box among finite ones takes one of C's own `mkMaze`
    /// failure exits.
    #[test]
    fn one_non_finite_box_fails_the_maze_like_c() {
        crate::cutil::in_fresh_run(|| {
            for c in [
                (f64::NAN, 18.0),
                (f64::INFINITY, 18.0),
                (18.0, f64::NEG_INFINITY),
            ] {
                let fail = mk_maze(&boxes(&[c, (81.0, 18.0)])).unwrap_err();
                assert!(
                    matches!(fail, Fail::Trapezoidation { .. } | Fail::NoFreeCell),
                    "{c:?}: {fail:?}"
                );
            }
        });
    }

    /// Issue #186's state, built directly: `shortPath` never reaches `sn`, so
    /// `N_DAD(sn)` is NULL, where C's `convertSPtoRoute` segfaults.
    #[test]
    fn an_unreached_start_is_no_path() {
        crate::cutil::in_fresh_run(|| {
            let mut mp = mk_maze(&boxes(&[(27.0, 18.0), (200.0, 18.0)])).unwrap();
            let sn = mp.sg.nnodes;
            assert_eq!(
                convert_sp_to_route(&mut mp, sn, sn + 1).unwrap_err(),
                Fail::NoPath
            );
        });
    }
}
