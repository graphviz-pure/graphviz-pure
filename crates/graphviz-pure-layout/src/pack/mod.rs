//! Port of Graphviz `lib/pack`: connected components (`ccomps.c`) and
//! packing of disconnected layouts (`pack.c`, polyomino packing after
//! Freivalds et al., plus array packing).
//!
//! Components are [`Sub`] views over an [`LGraph`]; packing reads node
//! positions (`ND_pos`, inches), sizes and installed edge splines, and
//! translates each component like `shiftGraphs`.

// Faithful ports of numeric C loops index several arrays in lockstep.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use crate::common::geom::{add, sub as psub};
use crate::common::types::{Bezier, LGraph, P_PIN, Sub};
use crate::common::utils::{compute_bb, coord, scan_d};
use crate::cutil::qsort_by;
use graphviz_pure_core::diag::{fmt_f, fprintf_stderr, verbose_byte, verbose_on};

pub mod cccomps;
use graphviz_pure_core::{BoundingBox, Point};
use std::collections::HashSet;

/// `pack_mode`; order matters (`mode <= l_graph` selects polyomino packing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum PackMode {
    #[default]
    Undef,
    Clust,
    Node,
    Graph,
    Array,
    Aspect,
}

pub const PK_COL_MAJOR: u32 = 1 << 0;
pub const PK_USER_VALS: u32 = 1 << 1;
pub const PK_LEFT_ALIGN: u32 = 1 << 2;
pub const PK_RIGHT_ALIGN: u32 = 1 << 3;
pub const PK_TOP_ALIGN: u32 = 1 << 4;
pub const PK_BOT_ALIGN: u32 = 1 << 5;
pub const PK_INPUT_ORDER: u32 = 1 << 6;

/// `pack_info`.
#[derive(Debug, Clone, Default)]
pub struct PackInfo {
    pub aspect: f32,
    pub sz: i32,
    pub margin: u32,
    pub do_splines: bool,
    pub mode: PackMode,
    pub fixed: Option<Vec<bool>>,
    pub vals: Option<Vec<u32>>,
    pub flags: u32,
}

fn chk_flags<'a>(p: &'a str, pinfo: &mut PackInfo) -> &'a str {
    let Some(mut rest) = p.strip_prefix('_') else {
        return p;
    };
    while let Some(c) = rest.chars().next() {
        let f = match c {
            'c' => PK_COL_MAJOR,
            'i' => PK_INPUT_ORDER,
            'u' => PK_USER_VALS,
            't' => PK_TOP_ALIGN,
            'b' => PK_BOT_ALIGN,
            'l' => PK_LEFT_ALIGN,
            'r' => PK_RIGHT_ALIGN,
            _ => break,
        };
        pinfo.flags |= f;
        rest = &rest[1..];
    }
    rest
}

/// `parsePackModeInfo(p, dflt, pinfo)` / `getPackModeInfo`.
pub fn get_pack_mode_info(p: Option<&str>, dflt: PackMode, pinfo: &mut PackInfo) -> PackMode {
    pinfo.flags = 0;
    pinfo.mode = dflt;
    pinfo.sz = 0;
    pinfo.vals = None;
    if let Some(p) = p {
        if let Some(r) = p.strip_prefix("array") {
            pinfo.mode = PackMode::Array;
            let r = chk_flags(r, pinfo);
            // sscanf(p, "%d", &i) > 0 && i > 0
            if let Some(i) = scan_d(r)
                && i > 0
            {
                pinfo.sz = i;
            }
        } else if let Some(r) = p.strip_prefix("aspect") {
            pinfo.mode = PackMode::Aspect;
            // sscanf(p + strlen("aspect"), "%f", &v) > 0 && v > 0
            pinfo.aspect = match graphviz_pure_core::strtod::scan_f(r) {
                Some((v, _)) if v > 0.0 => v,
                _ => 1.0,
            };
        } else if p == "cluster" {
            pinfo.mode = PackMode::Clust;
        } else if p == "graph" {
            pinfo.mode = PackMode::Graph;
        } else if p == "node" {
            pinfo.mode = PackMode::Node;
        }
    }
    if verbose_on() {
        let mut m = format!("pack info:\n  mode   {}\n", mode_str(pinfo.mode));
        if pinfo.mode == PackMode::Aspect {
            m.push_str(&format!("  aspect {}\n", fmt_f(f64::from(pinfo.aspect), 6)));
        }
        m.push_str(&format!(
            "  size   {}\n  flags  {}\n",
            pinfo.sz, pinfo.flags
        ));
        fprintf_stderr(&m);
    }
    pinfo.mode
}

/// `mode2Str(m)`.
fn mode_str(m: PackMode) -> &'static str {
    match m {
        PackMode::Clust => "cluster",
        PackMode::Node => "node",
        PackMode::Graph => "graph",
        PackMode::Array => "array",
        PackMode::Aspect => "aspect",
        PackMode::Undef => "undefined",
    }
}

/// `getPack(g, not_def, dflt)`: the `pack` attribute.
pub fn get_pack(p: Option<&str>, not_def: i64, dflt: i64) -> i64 {
    let Some(p) = p else { return not_def };
    // sscanf(p, "%d", &i) == 1 && i >= 0: 2147483648 reads as INT_MIN
    match scan_d(p) {
        Some(i) if i >= 0 => i64::from(i),
        Some(_) => not_def,
        None if p.starts_with('t') || p.starts_with('T') => dflt,
        None => not_def,
    }
}

/// `getPackInfo(g, dflt, dfltMargin, pinfo)`.
pub fn get_pack_info(
    g: &LGraph<'_>,
    dflt: PackMode,
    dflt_margin: i64,
    pinfo: &mut PackInfo,
) -> PackMode {
    pack_info(
        g.gattr("pack"),
        g.gattr("packmode"),
        dflt,
        dflt_margin,
        pinfo,
    )
}

/// [`get_pack_info`] on the graph's `pack` and `packmode` values.
pub fn pack_info(
    pack: Option<&str>,
    packmode: Option<&str>,
    dflt: PackMode,
    dflt_margin: i64,
    pinfo: &mut PackInfo,
) -> PackMode {
    pinfo.margin = get_pack(pack, dflt_margin, dflt_margin) as u32;
    graphviz_pure_core::verbose_msg!("  margin {}\n", pinfo.margin);
    pinfo.do_splines = false;
    pinfo.fixed = None;
    get_pack_mode_info(packmode, dflt, pinfo)
}

/// DFS of `ccomps.c`: marks and collects the component of `n`.
fn dfs(g: &LGraph<'_>, root: &Sub, n: usize, mark: &mut [bool], out: &mut Vec<usize>) {
    let mut stack = vec![n];
    mark[n] = true;
    while let Some(v) = stack.pop() {
        out.push(v);
        for e in g.all_edges(root, v) {
            let mut other = g.edges[e].tail;
            if other == v {
                other = g.edges[e].head;
            }
            if !mark[other] {
                mark[other] = true;
                stack.push(other);
            }
        }
    }
}

/// `pccomps(g, &ncc, pfx, &pinned)`: connected components, with all pinned
/// nodes (and what they reach) in the first one. Returns the components and
/// whether a pinned component exists.
pub fn pccomps(g: &LGraph<'_>, pfx: &str) -> (Vec<Sub>, bool) {
    let root = g.root_sub();
    let n = g.nodes.len();
    let mut mark = vec![false; n];
    let mut out = Vec::new();
    let mut pinned_nodes = Vec::new();
    let mut pin = false;
    for v in 0..n {
        if mark[v] || g.nodes[v].pinned != P_PIN {
            continue;
        }
        pin = true;
        dfs(g, &root, v, &mut mark, &mut pinned_nodes);
    }
    if pin {
        out.push(Sub::new(n, pinned_nodes));
    }
    for v in 0..n {
        if mark[v] {
            continue;
        }
        let mut c = Vec::new();
        dfs(g, &root, v, &mut mark, &mut c);
        out.push(Sub::new(n, c));
    }
    for (i, s) in out.iter_mut().enumerate() {
        s.name = format!("{pfx}{i}");
    }
    (out, pin)
}

/// `ccomps(g, &ncc, pfx)`: connected components in node order.
pub fn ccomps(g: &LGraph<'_>) -> Vec<Sub> {
    let root = g.root_sub();
    let n = g.nodes.len();
    let mut mark = vec![false; n];
    let mut out = Vec::new();
    for v in 0..n {
        if mark[v] {
            continue;
        }
        let mut c = Vec::new();
        dfs(g, &root, v, &mut mark, &mut c);
        out.push(Sub::new(n, c));
    }
    // setPrefix's default prefix
    for (i, s) in out.iter_mut().enumerate() {
        s.name = format!("_cc_{i}");
    }
    out
}

/// A graph or cluster of a packing layout (`Agraph_t *` in `shiftGraph`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkGraph {
    /// Component `gs[i]`.
    Comp(usize),
    /// A cluster (backend index).
    Clust(usize),
}

/// `ND_clust(n)` as seen by `genPoly` in `l_clust` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkClust {
    /// A cluster (backend index).
    Clust(usize),
    /// A non-cluster graph the layout recorded (dot sets `ND_clust(n)` to the
    /// layout root for nodes outside every cluster).
    Graph(usize),
}

/// What `lib/pack` reads and writes on the components `gs[0..ng]` of a root
/// graph: node positions (`ND_pos`, `ND_coord`), sizes, edge splines, labels
/// and cluster boxes. Implemented for neato-family [`LGraph`] components and
/// for dot (`dotgen::pack`).
pub trait PackGraph {
    fn pk_ncomps(&self) -> usize;
    /// `agnameof(gs[i])`, which the verbose messages print.
    fn pk_name(&self, i: usize) -> String;
    /// `compute_bb(gs[i])`, storing the result as `GD_bb(gs[i])`.
    fn pk_compute_bb(&mut self, i: usize);
    /// `GD_bb(g)`.
    fn pk_graph_bb(&self, g: PkGraph) -> BoundingBox;
    fn pk_bb(&self, i: usize) -> BoundingBox {
        self.pk_graph_bb(PkGraph::Comp(i))
    }
    /// Nodes of `gs[i]` in `agfstnode` order.
    fn pk_nodes(&self, i: usize) -> Vec<usize>;
    /// `GD_clust(g)[1..=GD_n_cluster(g)]`.
    fn pk_clusters(&self, g: PkGraph) -> Vec<usize>;
    /// Nodes of cluster `c` (`agfstnode(subg)`).
    fn pk_cluster_nodes(&self, c: usize) -> Vec<usize>;
    /// `ND_clust(n)` before `genPoly` marks the top-level clusters.
    fn pk_node_clust(&self, n: usize) -> Option<PkClust>;
    /// `coord(n)`: `ND_pos` in points.
    fn pk_coord(&self, n: usize) -> Point;
    /// `(ND_xsize(n), ND_ysize(n))`.
    fn pk_node_size(&self, n: usize) -> Point;
    /// Out-edges of `n` in the root graph (`agfstout(root, n)`).
    fn pk_root_outs(&self, n: usize) -> Vec<usize>;
    fn pk_head(&self, e: usize) -> usize;
    /// `ED_spl(e)`.
    fn pk_spl(&self, e: usize) -> Option<&[Bezier]>;
    /// `agget(gs[i], "sortv")`.
    fn pk_sortv(&self, i: usize) -> Option<&str>;
    /// `shiftGraphs` on a node: `ND_pos += d / 72`, `ND_coord += d`, and its xlabel.
    fn pk_shift_node(&mut self, n: usize, dx: f64, dy: f64);
    /// `shiftEdge(e, dx, dy)`.
    fn pk_shift_edge(&mut self, e: usize, dx: f64, dy: f64);
    /// The non-recursive part of `shiftGraph`: `GD_bb` and a set `GD_label`.
    fn pk_shift_graph_box(&mut self, g: PkGraph, dx: f64, dy: f64);
}

/// [`PackGraph`] over neato-family components ([`Sub`] views of an [`LGraph`]).
struct LPack<'g, 'a> {
    g: &'g mut LGraph<'a>,
    gs: &'g mut [Sub],
}

impl PackGraph for LPack<'_, '_> {
    fn pk_ncomps(&self) -> usize {
        self.gs.len()
    }
    fn pk_name(&self, i: usize) -> String {
        self.gs.get(i).map(|s| s.name.clone()).unwrap_or_default()
    }
    fn pk_compute_bb(&mut self, i: usize) {
        compute_bb(self.g, &mut self.gs[i], false);
    }
    fn pk_graph_bb(&self, g: PkGraph) -> BoundingBox {
        match g {
            PkGraph::Comp(i) => self.gs[i].bb,
            PkGraph::Clust(c) => self.g.clusters[c].bb,
        }
    }
    fn pk_nodes(&self, i: usize) -> Vec<usize> {
        self.gs[i].nodes.clone()
    }
    fn pk_clusters(&self, g: PkGraph) -> Vec<usize> {
        match g {
            PkGraph::Comp(i) => self.gs[i].clusters.clone(),
            PkGraph::Clust(c) => self.g.clusters[c].children.clone(),
        }
    }
    fn pk_cluster_nodes(&self, c: usize) -> Vec<usize> {
        crate::neatogen::neatoinit::subgraph_nodes(self.g.src, self.g.clusters[c].subgraph)
            .into_iter()
            .collect()
    }
    fn pk_node_clust(&self, _n: usize) -> Option<PkClust> {
        None
    }
    fn pk_coord(&self, n: usize) -> Point {
        coord(self.g, n)
    }
    fn pk_node_size(&self, n: usize) -> Point {
        let nd = &self.g.nodes[n];
        Point::new(nd.lw + nd.rw, nd.ht)
    }
    fn pk_root_outs(&self, n: usize) -> Vec<usize> {
        let root = self.g.root_sub();
        self.g.outs(&root, n).collect()
    }
    fn pk_head(&self, e: usize) -> usize {
        self.g.edges[e].head
    }
    fn pk_spl(&self, e: usize) -> Option<&[Bezier]> {
        self.g.edges[e].spl.as_deref()
    }
    fn pk_sortv(&self, _i: usize) -> Option<&str> {
        self.g.gattr("sortv")
    }
    fn pk_shift_node(&mut self, n: usize, dx: f64, dy: f64) {
        let nd = &mut self.g.nodes[n];
        nd.pos[0] += dx / 72.0;
        nd.pos[1] += dy / 72.0;
        nd.coord.x += dx;
        nd.coord.y += dy;
        if let Some(l) = nd.xlabel.as_mut() {
            l.pos.x += dx;
            l.pos.y += dy;
        }
    }
    fn pk_shift_edge(&mut self, e: usize, dx: f64, dy: f64) {
        let ed = &mut self.g.edges[e];
        for l in [
            &mut ed.label,
            &mut ed.xlabel,
            &mut ed.head_label,
            &mut ed.tail_label,
        ]
        .into_iter()
        .flatten()
        {
            l.pos.x += dx;
            l.pos.y += dy;
        }
        if let Some(spl) = ed.spl.as_mut() {
            shift_beziers(spl, dx, dy);
        }
    }
    fn pk_shift_graph_box(&mut self, g: PkGraph, dx: f64, dy: f64) {
        let (bb, label) = match g {
            PkGraph::Comp(i) => (&mut self.gs[i].bb, None),
            PkGraph::Clust(c) => {
                let cl = &mut self.g.clusters[c];
                (&mut cl.bb, cl.label.as_mut())
            }
        };
        bb.ll.x += dx;
        bb.ur.x += dx;
        bb.ll.y += dy;
        bb.ur.y += dy;
        if let Some(l) = label.filter(|l| l.set) {
            l.pos.x += dx;
            l.pos.y += dy;
        }
    }
}

/// The spline part of `shiftEdge`.
pub fn shift_beziers(spl: &mut [Bezier], dx: f64, dy: f64) {
    for b in spl.iter_mut() {
        for p in &mut b.list {
            p.x += dx;
            p.y += dy;
        }
        if b.sflag != 0 {
            b.sp.x += dx;
            b.sp.y += dy;
        }
        if b.eflag != 0 {
            b.ep.x += dx;
            b.ep.y += dy;
        }
    }
}

const C: f64 = 100.0;

fn c_round(f: f64) -> i64 {
    i64::from(crate::common::utils::c_round(f))
}

/// `GRID(x, s)`.
fn grid(x: f64, s: i32) -> i32 {
    (x / f64::from(s)).ceil() as i32
}

/// `CVAL(v, s)` on doubles.
fn cval(v: f64, s: i32) -> f64 {
    let s = f64::from(s);
    if v >= 0.0 {
        v / s
    } else {
        ((v + 1.0) / s) - 1.0
    }
}

fn cell(p: Point, s: i32) -> Point {
    Point::new(cval(p.x, s), cval(p.y, s))
}

fn round_pt(p: Point) -> Point {
    Point::new(p.x.round(), p.y.round())
}

/// `computeStep(ng, bbs, margin)`.
fn compute_step(bbs: &[BoundingBox], margin: u32) -> i32 {
    let ng = bbs.len();
    let a = C * ng as f64 - 1.0;
    let mut b = 0.0;
    let mut c = 0.0;
    for bb in bbs {
        let w = bb.ur.x - bb.ll.x + 2.0 * f64::from(margin);
        let h = bb.ur.y - bb.ll.y + 2.0 * f64::from(margin);
        b -= w + h;
        c = (-w).mul_add(h, c);
    }
    let d = b.mul_add(b, -(4.0 * a * c));
    // C asserts `d >= 0`. A box whose width or height is NaN (`inf - inf`: a component
    // whose nodes are all at `-inf`, #239) makes `d` NaN, C aborts, and the NaN step would
    // fill cells without end. Return 0 instead, which takes the callers' `stepSize <= 0`
    // exit: the components stay unpacked. (A merely infinite box gives `d = inf` and a
    // saturated step, which the budget below handles.)
    if d.is_nan() || d < 0.0 {
        warn_too_large();
        return 0;
    }
    let r = d.sqrt();
    let l1 = (-b + r) / (2.0 * a);
    let root = l1 as i32;
    let root = if root == 0 { 1 } else { root };
    if verbose_byte() > 2 {
        let l2 = (-b - r) / (2.0 * a);
        let f = |v: f64| fmt_f(v, 6);
        fprintf_stderr(&format!(
            "Packing: compute grid size\na {} b {} c {} d {} r {}\nroot {root} ({}) {} ({})\n r1 {} r2 {}\n",
            f(a),
            f(b),
            f(c),
            f(d),
            f(r),
            f(l1),
            l2 as i32,
            f(l2),
            // clang: fmuladd(a * l, l, b * l) + c
            f((a * l1).mul_add(l1, b * l1) + c),
            f((a * l2).mul_add(l2, b * l2) + c),
        ));
    }
    root
}

struct GInfo {
    perim: i32,
    cells: Vec<(i64, i64)>,
    index: usize,
}

type PointSet = HashSet<(i64, i64)>;

/// Not in C: the most cells genBox/genPoly may fill when computeStep saturated (issue #174).
/// Measured on the reference build with one edge of `n` cells (#250's input, scaled): C takes
/// 0.9 s and 1.1 GB at 3.4e7 cells, 3.9 s and 4.3 GB at 2^27, 10.5 s and 9.2 GB at 3.4e8,
/// 47 s and 18 GB at 1e9; the port 1.8 s and 1.5 GB at 3.4e7, 13 s and 6 GB at 2^27, with
/// the same output. Past 2^27, where C needs more than 4 GB, the port warns instead (#250).
#[cfg(not(target_pointer_width = "32"))]
const MAX_CELLS: f64 = (1u64 << 27) as f64;
/// A 32-bit target (wasm32) has at most 4 GiB, which 2^27 cells would exceed: there the
/// budget is 2^25 cells (the port: about 1.5 GB), which still covers #250's 3.35e7.
#[cfg(target_pointer_width = "32")]
const MAX_CELLS: f64 = (1u64 << 25) as f64;

/// Not in C. With a huge component (height="1e30", quantum="1e40") `l1` in computeStep
/// (`lib/pack/pack.c:86`) is far above INT_MAX, `(int)l1` saturates on the reference
/// platform, and the step is then far too small: genBox/genPoly fill ~1e22 cells per side
/// and C never finishes. An unsaturated step gives about `C * ng` cells by construction, so
/// only a saturated step gets a budget; when it runs out, packing fails like C's
/// `stepSize <= 0` exit and the components stay unpacked.
struct Budget(Option<f64>);

impl Budget {
    fn for_step(step: i32) -> Self {
        // `l1` is NaN only when `d` is NaN or negative, which compute_step refuses (#239).
        Self((step == i32::MAX).then_some(MAX_CELLS))
    }

    /// Spends `n` cells; false once the budget is exceeded (or `n` is NaN).
    fn take(&mut self, n: f64) -> bool {
        match &mut self.0 {
            None => true,
            Some(left) => {
                *left -= n;
                *left >= 0.0
            }
        }
    }
}

/// Warns that packing gave up on a [`Budget`], once per run.
fn warn_too_large() {
    if graphviz_pure_core::diag::emit_once("\0pack\0too large") {
        graphviz_pure_core::diag::agwarningf("components too large to pack; packing skipped\n");
    }
}

fn sgn(x: i64) -> i64 {
    if x > 0 { 1 } else { -1 }
}

/// `fillLine(p, q, ps)`: Bresenham over cells; false if `budget` runs out first.
fn fill_line(p: Point, q: Point, ps: &mut PointSet, budget: &mut Budget) -> bool {
    if !budget.take((q.x - p.x).abs().max((q.y - p.y).abs()) + 1.0) {
        return false;
    }
    let (x1, y1, x2, y2) = (c_round(p.x), c_round(p.y), c_round(q.x), c_round(q.y));
    let dx = x2 - x1;
    let ax = dx.abs() << 1;
    let sx = sgn(dx);
    let dy = y2 - y1;
    let ay = dy.abs() << 1;
    let sy = sgn(dy);
    let (mut x, mut y) = (x1, y1);
    if ax > ay {
        let mut d = ay - (ax >> 1);
        loop {
            ps.insert((x, y));
            if x == x2 {
                return true;
            }
            if d >= 0 {
                y += sy;
                d -= ax;
            }
            x += sx;
            d += ay;
        }
    } else {
        let mut d = ax - (ay >> 1);
        loop {
            ps.insert((x, y));
            if y == y2 {
                return true;
            }
            if d >= 0 {
                x += sx;
                d -= ay;
            }
            y += sy;
            d += ax;
        }
    }
}

/// `fillEdge(e, p, ps, dx, dy, ssize, doS)`.
fn fill_edge<P: PackGraph + ?Sized>(
    g: &P,
    e: usize,
    p: Point,
    ps: &mut PointSet,
    dx: f64,
    dy: f64,
    ssize: i32,
    do_s: bool,
    budget: &mut Budget,
) -> bool {
    let mv = |q: Point| Point::new(q.x + dx, q.y + dy);
    let spl = g.pk_spl(e);
    if !do_s || spl.is_none() {
        let hpt = cell(mv(g.pk_coord(g.pk_head(e))), ssize);
        return fill_line(p, hpt, ps, budget);
    }
    for bz in spl.unwrap() {
        // Not in C. Clipping can leave a bezier with no points (two overlapping pinned nodes
        // joined by a multi-edge: fuzz case 003, issue #12). C reads bz.list[0] past the end
        // and crashes; skip it instead. Test: cli.rs packed_edges_with_empty_splines_do_not_panic.
        if bz.list.is_empty() {
            continue;
        }
        // `fillEdge` reads `list[1]` whenever `sflag` is clear, and it does not look at
        // `size` first (`lib/pack/pack.c`). `clip_and_install` allocates `pn` points with
        // `gv_calloc` and then sets `size` to the clipped count (`lib/common/splines.c`
        // `new_spline`): a one-point polyline (`start == end + 3`) still has `list[1]`
        // inside that allocation, and the slot is the zero `calloc` left. `make_polyline`
        // of a multi-edge hands over `pn >= 4`, so the read is defined. The port keeps
        // only the clipped points, so the missing slot is that zero. The port models
        // that zeroed slot rather than reading the allocation. Repeating `list[0]`
        // drops the diagonal through the origin cell and the packer places the component
        // short (#456; measured on the sfdp polyline repro, step 27, edge n217 -> n0,
        // `malloc_size` 160 = 10 points).
        let (mut pt, mut hpt, mut k) = if bz.sflag != 0 {
            (bz.sp, bz.list[0], 1)
        } else if let Some(next) = bz.list.get(1) {
            (bz.list[0], *next, 2)
        } else {
            (bz.list[0], Point::new(0.0, 0.0), 2)
        };
        pt = cell(mv(pt), ssize);
        hpt = cell(mv(hpt), ssize);
        if !fill_line(pt, hpt, ps, budget) {
            return false;
        }
        while k < bz.list.len() {
            pt = hpt;
            hpt = cell(mv(bz.list[k]), ssize);
            if !fill_line(pt, hpt, ps, budget) {
                return false;
            }
            k += 1;
        }
        if bz.eflag != 0 {
            pt = hpt;
            hpt = cell(mv(bz.ep), ssize);
            if !fill_line(pt, hpt, ps, budget) {
                return false;
            }
        }
    }
    true
}

/// Adds the cells of `[ll, ur]`; false if `budget` runs out first.
fn add_rect(ps: &mut PointSet, ll: Point, ur: Point, budget: &mut Budget) -> bool {
    if ll.x <= ur.x && ll.y <= ur.y && !budget.take((ur.x - ll.x + 1.0) * (ur.y - ll.y + 1.0)) {
        return false;
    }
    // C counts with ints. Count the cells rather than step a double: above 2^53 (a node
    // pinned at pos="1e30,1e30!") `x += 1.0` no longer advances, and C hangs there too.
    let (nx, ny) = ((ur.x - ll.x + 1.0) as i64, (ur.y - ll.y + 1.0) as i64);
    for i in 0..nx {
        let x = ll.x + i as f64;
        for j in 0..ny {
            ps.insert((x as i64, (ll.y + j as f64) as i64));
        }
    }
    true
}

/// `genBox(bb0, info, ssize, margin, center)`.
fn gen_box(
    bb0: &BoundingBox,
    ssize: i32,
    margin: u32,
    center: Point,
    index: usize,
    budget: &mut Budget,
) -> Option<GInfo> {
    let m = f64::from(margin);
    let bb = BoundingBox::new(
        bb0.ll.x.round(),
        bb0.ll.y.round(),
        bb0.ur.x.round(),
        bb0.ur.y.round(),
    );
    let mut ps = PointSet::new();
    let ll = round_pt(cell(Point::new(center.x - m, center.y - m), ssize));
    let ur = round_pt(cell(
        Point::new(
            center.x + bb.ur.x - bb.ll.x + m,
            center.y + bb.ur.y - bb.ll.y + m,
        ),
        ssize,
    ));
    if !add_rect(&mut ps, ll, ur, budget) {
        return None;
    }
    let w = grid(bb0.ur.x - bb0.ll.x + 2.0 * m, ssize);
    let h = grid(bb0.ur.y - bb0.ll.y + 2.0 * m, ssize);
    Some(GInfo {
        // A degenerate box (an empty component's ±DBL_MAX) saturates GRID's
        // double-to-int conversion; C's int sum then wraps.
        perim: w.wrapping_add(h),
        cells: ps.into_iter().collect(),
        index,
    })
}

/// genBox's `Verbose > 2` dump of the cells of `info`, made from `bb0`, named `name()`.
fn verbose_box_cells(
    name: impl FnOnce() -> String,
    bb0: &BoundingBox,
    ssize: i32,
    margin: u32,
    info: &GInfo,
) {
    if verbose_byte() > 2 {
        let m = 2.0 * f64::from(margin);
        let w = grid(bb0.ur.x - bb0.ll.x + m, ssize);
        let h = grid(bb0.ur.y - bb0.ll.y + m, ssize);
        verbose_cells(&name(), &info.cells, w, h);
    }
}

/// genBox's and genPoly's `Verbose > 2` dump: the cells in the PointSet's (x, y) order.
fn verbose_cells(name: &str, cells: &[(i64, i64)], w: i32, h: i32) {
    if verbose_byte() > 2 {
        let mut sorted = cells.to_vec();
        sorted.sort_unstable();
        let mut m = format!("{name} no. cells {} W {w} H {h}\n", cells.len());
        for (x, y) in sorted {
            m.push_str(&format!("  {x} {y} cell\n"));
        }
        fprintf_stderr(&m);
    }
}

/// `genPoly(root, g, info, ssize, pinfo, center)` for component `gi`.
fn gen_poly<P: PackGraph + ?Sized>(
    g: &P,
    gi: usize,
    ssize: i32,
    pinfo: &PackInfo,
    center: Point,
    index: usize,
    budget: &mut Budget,
) -> Option<GInfo> {
    let mut ps = PointSet::new();
    let margin = f64::from(pinfo.margin);
    let gbb = g.pk_bb(gi);
    let dx = center.x - gbb.ll.x.round();
    let dy = center.y - gbb.ll.y.round();
    let nodes = g.pk_nodes(gi);
    if pinfo.mode == PackMode::Clust {
        // ND_clust(n) as the layout left it, overridden for nodes of top-level
        // clusters with a non-degenerate box.
        let mut clust_of: std::collections::HashMap<usize, PkClust> = nodes
            .iter()
            .filter_map(|&n| g.pk_node_clust(n).map(|c| (n, c)))
            .collect();
        for c in g.pk_clusters(PkGraph::Comp(gi)) {
            let b = g.pk_graph_bb(PkGraph::Clust(c));
            let mut bb = BoundingBox::new(
                b.ll.x.round(),
                b.ll.y.round(),
                b.ur.x.round(),
                b.ur.y.round(),
            );
            if bb.ur.x > bb.ll.x && bb.ur.y > bb.ll.y {
                bb.ll = Point::new(bb.ll.x + dx - margin, bb.ll.y + dy - margin);
                bb.ur = Point::new(bb.ur.x + dx + margin, bb.ur.y + dy + margin);
                let ll = round_pt(cell(bb.ll, ssize));
                let ur = round_pt(cell(bb.ur, ssize));
                if !add_rect(&mut ps, ll, ur, budget) {
                    return None;
                }
                for n in g.pk_cluster_nodes(c) {
                    clust_of.insert(n, PkClust::Clust(c));
                }
            }
        }
        for &n in &nodes {
            let ptf = g.pk_coord(n);
            let mut pt = Point::new(ptf.x.round() + dx, ptf.y.round() + dy);
            let in_clust = clust_of.get(&n).copied();
            if in_clust.is_none() {
                let s = g.pk_node_size(n);
                let s2 = Point::new((margin + s.x / 2.0).round(), (margin + s.y / 2.0).round());
                let ll = round_pt(cell(psub(pt, s2), ssize));
                let ur = round_pt(cell(add(pt, s2), ssize));
                if !add_rect(&mut ps, ll, ur, budget) {
                    return None;
                }
            }
            pt = round_pt(cell(pt, ssize));
            for e in g.pk_root_outs(n) {
                if in_clust.is_some() && in_clust == clust_of.get(&g.pk_head(e)).copied() {
                    continue;
                }
                if !fill_edge(g, e, pt, &mut ps, dx, dy, ssize, pinfo.do_splines, budget) {
                    return None;
                }
            }
        }
    } else {
        for &n in &nodes {
            let ptf = g.pk_coord(n);
            let mut pt = Point::new(ptf.x.round() + dx, ptf.y.round() + dy);
            let s = g.pk_node_size(n);
            let s2 = Point::new((margin + s.x / 2.0).round(), (margin + s.y / 2.0).round());
            let ll = round_pt(cell(psub(pt, s2), ssize));
            let ur = round_pt(cell(add(pt, s2), ssize));
            if !add_rect(&mut ps, ll, ur, budget) {
                return None;
            }
            pt = round_pt(cell(pt, ssize));
            for e in g.pk_root_outs(n) {
                if !fill_edge(g, e, pt, &mut ps, dx, dy, ssize, pinfo.do_splines, budget) {
                    return None;
                }
            }
        }
    }
    let w = grid(gbb.ur.x - gbb.ll.x + 2.0 * margin, ssize);
    let h = grid(gbb.ur.y - gbb.ll.y + 2.0 * margin, ssize);
    let cells: Vec<(i64, i64)> = ps.into_iter().collect();
    verbose_cells(&g.pk_name(gi), &cells, w, h);
    Some(GInfo {
        perim: w.wrapping_add(h),
        cells,
        index,
    })
}

/// `fits(x, y, info, ps, place, step, bbs)`.
fn fits(
    x: i64,
    y: i64,
    info: &GInfo,
    ps: &mut PointSet,
    step: i32,
    bbs: &[BoundingBox],
) -> Option<Point> {
    if info
        .cells
        .iter()
        // Not in C: C's cells are doubles and it hangs before this point. Cells of a node
        // pinned near 1e30 saturate to i64::MAX, so wrap (as the release build did) rather
        // than panic.
        .any(|&(cx, cy)| ps.contains(&(cx.wrapping_add(x), cy.wrapping_add(y))))
    {
        return None;
    }
    let ll = Point::new(bbs[info.index].ll.x.round(), bbs[info.index].ll.y.round());
    let place = Point::new(
        // `step * x` is int * int in C and wraps once the step is large.
        f64::from(step.wrapping_mul(x as i32)) - ll.x,
        f64::from(step.wrapping_mul(y as i32)) - ll.y,
    );
    for &(cx, cy) in &info.cells {
        ps.insert((cx.wrapping_add(x), cy.wrapping_add(y)));
    }
    if verbose_byte() >= 2 {
        fprintf_stderr(&format!(
            "cc ({} cells) at ({x},{y}) ({},{})\n",
            info.cells.len(),
            fmt_f(place.x, 0),
            fmt_f(place.y, 0)
        ));
    }
    Some(place)
}

/// `placeFixed(info, ps, place, center)`.
fn place_fixed(info: &GInfo, ps: &mut PointSet, center: Point) -> Point {
    for &c in &info.cells {
        ps.insert(c);
    }
    let place = Point::new(-center.x, -center.y);
    if verbose_byte() >= 2 {
        fprintf_stderr(&format!(
            "cc ({} cells) at ({},{})\n",
            info.cells.len(),
            fmt_f(place.x, 0),
            fmt_f(place.y, 0)
        ));
    }
    place
}

/// `placeGraph(i, info, ps, place, step, margin, bbs)`.
fn place_graph(
    i: usize,
    info: &GInfo,
    ps: &mut PointSet,
    step: i32,
    margin: u32,
    bbs: &[BoundingBox],
) -> Point {
    let bb = bbs[info.index];
    if i == 0 {
        let w = grid(bb.ur.x - bb.ll.x + 2.0 * f64::from(margin), step);
        let h = grid(bb.ur.y - bb.ll.y + 2.0 * f64::from(margin), step);
        if let Some(p) = fits(i64::from(-w / 2), i64::from(-h / 2), info, ps, step, bbs) {
            return p;
        }
    }
    if let Some(p) = fits(0, 0, info, ps, step, bbs) {
        return p;
    }
    let w = (bb.ur.x - bb.ll.x).ceil();
    let h = (bb.ur.y - bb.ll.y).ceil();
    let mut bnd: i64 = 1;
    loop {
        if w >= h {
            let (mut x, mut y) = (0i64, -bnd);
            while x < bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                x += 1;
            }
            while y < bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                y += 1;
            }
            while x > -bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                x -= 1;
            }
            while y > -bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                y -= 1;
            }
            while x < 0 {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                x += 1;
            }
        } else {
            let (mut x, mut y) = (-bnd, 0i64);
            while y > -bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                y -= 1;
            }
            while x < bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                x += 1;
            }
            while y < bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                y += 1;
            }
            while x > -bnd {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                x -= 1;
            }
            while y > 0 {
                if let Some(p) = fits(x, y, info, ps, step, bbs) {
                    return p;
                }
                y -= 1;
            }
        }
        bnd += 1;
    }
}

fn cmpf(x: &usize, y: &usize, infos: &[GInfo]) -> i32 {
    let (a, b) = (infos[*x].perim, infos[*y].perim);
    if b < a {
        -1
    } else if b > a {
        1
    } else {
        0
    }
}

/// `polyGraphs(ng, gs, root, pinfo)`.
fn poly_graphs<P: PackGraph + ?Sized>(g: &mut P, pinfo: &PackInfo) -> Option<Vec<Point>> {
    let ng = g.pk_ncomps();
    if ng == 0 {
        return None;
    }
    let mut fixed_bb = BoundingBox::default();
    let mut fixed_cnt = 0;
    for i in 0..ng {
        g.pk_compute_bb(i);
        if let Some(f) = &pinfo.fixed
            && f[i]
        {
            let b = g.pk_bb(i);
            let bb = BoundingBox::new(
                b.ll.x.round(),
                b.ll.y.round(),
                b.ur.x.round(),
                b.ur.y.round(),
            );
            if fixed_cnt > 0 {
                fixed_bb.ll.x = bb.ll.x.min(fixed_bb.ll.x);
                fixed_bb.ll.y = bb.ll.y.min(fixed_bb.ll.y);
                fixed_bb.ur.x = bb.ur.x.max(fixed_bb.ur.x);
                fixed_bb.ur.y = bb.ur.y.max(fixed_bb.ur.y);
            } else {
                fixed_bb = bb;
            }
            fixed_cnt += 1;
        }
        if verbose_byte() > 2 {
            let b = g.pk_bb(i);
            let f = |v: f64| graphviz_pure_core::diag::fmt_g(v, 5);
            fprintf_stderr(&format!(
                "bb[{}] {} {} {} {}\n",
                g.pk_name(i),
                f(b.ll.x),
                f(b.ll.y),
                f(b.ur.x),
                f(b.ur.y)
            ));
        }
    }
    let bbs: Vec<BoundingBox> = (0..ng).map(|i| g.pk_bb(i)).collect();
    let step = compute_step(&bbs, pinfo.margin);
    graphviz_pure_core::verbose_msg!("step size = {step}\n");
    if step <= 0 {
        return None;
    }
    let mut center = Point::default();
    if pinfo.fixed.is_some() {
        center = Point::new(
            ((fixed_bb.ll.x + fixed_bb.ur.x) / 2.0).round(),
            ((fixed_bb.ll.y + fixed_bb.ur.y) / 2.0).round(),
        );
    }
    let mut budget = Budget::for_step(step);
    let Some(infos) = (0..ng)
        .map(|i| {
            if pinfo.mode == PackMode::Graph {
                let b = &bbs[i];
                let info = gen_box(b, step, pinfo.margin, center, i, &mut budget);
                if let Some(gi) = &info {
                    verbose_box_cells(|| g.pk_name(i), b, step, pinfo.margin, gi);
                }
                info
            } else {
                gen_poly(g, i, step, pinfo, center, i, &mut budget)
            }
        })
        .collect::<Option<Vec<GInfo>>>()
    else {
        warn_too_large();
        return None;
    };
    let mut sinfo: Vec<usize> = (0..ng).collect();
    qsort_by(&mut sinfo, |a, b| cmpf(a, b, &infos));
    let mut ps = PointSet::new();
    let mut places = vec![Point::default(); ng];
    if let Some(fixed) = &pinfo.fixed {
        for i in 0..ng {
            if fixed[i] {
                let inf = &infos[sinfo[i]];
                places[inf.index] = place_fixed(inf, &mut ps, center);
            }
        }
        for i in 0..ng {
            if !fixed[i] {
                let inf = &infos[sinfo[i]];
                places[inf.index] = place_graph(i, inf, &mut ps, step, pinfo.margin, &bbs);
            }
        }
    } else {
        for (i, &si) in sinfo.iter().enumerate() {
            let inf = &infos[si];
            places[inf.index] = place_graph(i, inf, &mut ps, step, pinfo.margin, &bbs);
        }
    }
    verbose_places(&places);
    Some(places)
}

/// polyRects' and polyGraphs' `Verbose > 1` list of the translations.
fn verbose_places(places: &[Point]) {
    if verbose_byte() > 1 {
        let mut m = String::new();
        for (i, p) in places.iter().enumerate() {
            m.push_str(&format!("pos[{i}] {} {}\n", fmt_f(p.x, 0), fmt_f(p.y, 0)));
        }
        fprintf_stderr(&m);
    }
}

/// `INC(m, c, r, nc, nr)`.
fn inc(m: bool, c: &mut usize, r: &mut usize, nc: usize, nr: usize) {
    if m {
        *c += 1;
        if *c == nc {
            *c = 0;
            *r += 1;
        }
    } else {
        *r += 1;
        if *r == nr {
            *r = 0;
            *c += 1;
        }
    }
}

/// `arrayRects(ng, gs, pinfo)`.
pub fn array_rects(gs: &[BoundingBox], pinfo: &PackInfo) -> Vec<Point> {
    let ng = gs.len();
    let sz = pinfo.sz;
    let (row_major, nr, nc);
    if pinfo.flags & PK_COL_MAJOR != 0 {
        row_major = false;
        nr = if sz > 0 {
            sz as usize
        } else {
            (ng as f64).sqrt().ceil() as usize
        };
        nc = ng.div_ceil(nr);
    } else {
        row_major = true;
        nc = if sz > 0 {
            sz as usize
        } else {
            (ng as f64).sqrt().ceil() as usize
        };
        nr = ng.div_ceil(nc);
    }
    graphviz_pure_core::verbose_msg!(
        "array packing: {} {nr} rows {nc} columns\n",
        if row_major {
            "row major"
        } else {
            "column major"
        }
    );
    let mut widths = vec![0.0f64; nc + 1];
    let mut heights = vec![0.0f64; nr + 1];
    let m = f64::from(pinfo.margin);
    let info: Vec<(f64, f64, usize)> = gs
        .iter()
        .enumerate()
        .map(|(i, b)| (b.ur.x - b.ll.x + m, b.ur.y - b.ll.y + m, i))
        .collect();
    let mut sinfo: Vec<usize> = (0..ng).collect();
    if let Some(vals) = &pinfo.vals {
        // gv_sort is qsort with a context argument.
        qsort_by(&mut sinfo, |a, b| {
            let (x, y) = (vals[info[*a].2], vals[info[*b].2]);
            if x > y {
                1
            } else if x < y {
                -1
            } else {
                0
            }
        });
    } else if pinfo.flags & PK_INPUT_ORDER == 0 {
        qsort_by(&mut sinfo, |a, b| {
            let da = info[*a].1 + info[*a].0;
            let db = info[*b].1 + info[*b].0;
            if da < db {
                1
            } else if da > db {
                -1
            } else {
                0
            }
        });
    }
    let (mut r, mut c) = (0usize, 0usize);
    for &si in &sinfo {
        widths[c] = widths[c].max(info[si].0);
        heights[r] = heights[r].max(info[si].1);
        inc(row_major, &mut c, &mut r, nc, nr);
    }
    let mut wd = 0.0;
    for w in widths.iter_mut() {
        let v = *w;
        *w = wd;
        wd += v;
    }
    let mut ht = 0.0;
    for i in (1..=nr).rev() {
        let v = heights[i - 1];
        heights[i] = ht;
        ht += v;
    }
    heights[0] = ht;
    let mut places = vec![Point::default(); ng];
    let (mut r, mut c) = (0usize, 0usize);
    for &si in &sinfo {
        let idx = info[si].2;
        let bb = gs[idx];
        let x = if pinfo.flags & PK_LEFT_ALIGN != 0 {
            widths[c].round()
        } else if pinfo.flags & PK_RIGHT_ALIGN != 0 {
            (widths[c + 1] - (bb.ur.x - bb.ll.x)).round()
        } else {
            ((widths[c] + widths[c + 1] - bb.ur.x - bb.ll.x) / 2.0).round()
        };
        let y = if pinfo.flags & PK_TOP_ALIGN != 0 {
            (heights[r] - (bb.ur.y - bb.ll.y)).round()
        } else if pinfo.flags & PK_BOT_ALIGN != 0 {
            heights[r + 1].round()
        } else {
            ((heights[r] + heights[r + 1] - bb.ur.y - bb.ll.y) / 2.0).round()
        };
        places[idx] = Point::new(x, y);
        inc(row_major, &mut c, &mut r, nc, nr);
    }
    places
}

/// `putGraphs(ng, gs, root, pinfo)`: translations for each component.
pub fn put_graphs(g: &mut LGraph<'_>, gs: &mut [Sub], pinfo: &mut PackInfo) -> Option<Vec<Point>> {
    put_graphs_in(&mut LPack { g, gs }, pinfo)
}

/// `putGraphs` over any [`PackGraph`].
pub fn put_graphs_in<P: PackGraph + ?Sized>(g: &mut P, pinfo: &mut PackInfo) -> Option<Vec<Point>> {
    let ng = g.pk_ncomps();
    if ng == 0 {
        return None;
    }
    if pinfo.mode <= PackMode::Graph {
        return poly_graphs(g, pinfo);
    }
    for i in 0..ng {
        g.pk_compute_bb(i);
    }
    let bbs: Vec<BoundingBox> = (0..ng).map(|i| g.pk_bb(i)).collect();
    if pinfo.mode == PackMode::Array {
        if pinfo.flags & PK_USER_VALS != 0 {
            let vals = (0..ng)
                .map(|i| {
                    g.pk_sortv(i)
                        .and_then(scan_d)
                        .filter(|&v| v >= 0)
                        .unwrap_or(0) as u32
                })
                .collect();
            pinfo.vals = Some(vals);
        }
        let pts = array_rects(&bbs, pinfo);
        pinfo.vals = None;
        return Some(pts);
    }
    None
}

/// `shiftGraphs(ng, gs, pp, root, doSplines)`.
pub fn shift_graphs(g: &mut LGraph<'_>, gs: &mut [Sub], pp: &[Point], do_splines: bool) {
    shift_graphs_in(&mut LPack { g, gs }, pp, do_splines);
}

/// `shiftGraphs` over any [`PackGraph`].
pub fn shift_graphs_in<P: PackGraph + ?Sized>(g: &mut P, pp: &[Point], do_splines: bool) {
    for i in 0..g.pk_ncomps() {
        let (dx, dy) = (pp[i].x, pp[i].y);
        for n in g.pk_nodes(i) {
            g.pk_shift_node(n, dx, dy);
            if do_splines {
                for e in g.pk_root_outs(n) {
                    g.pk_shift_edge(e, dx, dy);
                }
            }
        }
        shift_graph(g, PkGraph::Comp(i), dx, dy);
    }
}

/// `shiftGraph(g, dx, dy)`.
fn shift_graph<P: PackGraph + ?Sized>(g: &mut P, sg: PkGraph, dx: f64, dy: f64) {
    g.pk_shift_graph_box(sg, dx, dy);
    for c in g.pk_clusters(sg) {
        shift_graph(g, PkGraph::Clust(c), dx, dy);
    }
}

/// `packGraphs(ng, gs, root, info)`; false if packing failed.
pub fn pack_graphs(g: &mut LGraph<'_>, gs: &mut [Sub], info: &PackInfo) -> bool {
    pack_graphs_in(&mut LPack { g, gs }, info)
}

/// `packGraphs` over any [`PackGraph`].
pub fn pack_graphs_in<P: PackGraph + ?Sized>(g: &mut P, info: &PackInfo) -> bool {
    let mut info = info.clone();
    let Some(pp) = put_graphs_in(g, &mut info) else {
        return false;
    };
    shift_graphs_in(g, &pp, info.do_splines);
    true
}

/// `putRects(ng, bbs, pinfo)`.
pub fn put_rects(bbs: &[BoundingBox], pinfo: &PackInfo) -> Option<Vec<Point>> {
    if bbs.is_empty() {
        return None;
    }
    match pinfo.mode {
        PackMode::Graph => poly_rects(bbs, pinfo),
        PackMode::Array => Some(array_rects(bbs, pinfo)),
        _ => None,
    }
}

/// `polyRects(ng, gs, pinfo)`.
fn poly_rects(bbs: &[BoundingBox], pinfo: &PackInfo) -> Option<Vec<Point>> {
    let step = compute_step(bbs, pinfo.margin);
    graphviz_pure_core::verbose_msg!("step size = {step}\n");
    if step <= 0 {
        return None;
    }
    let mut budget = Budget::for_step(step);
    let Some(infos) = bbs
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let info = gen_box(b, step, pinfo.margin, Point::default(), i, &mut budget);
            if let Some(gi) = &info {
                verbose_box_cells(String::new, b, step, pinfo.margin, gi);
            }
            info
        })
        .collect::<Option<Vec<GInfo>>>()
    else {
        warn_too_large();
        return None;
    };
    let mut sinfo: Vec<usize> = (0..bbs.len()).collect();
    qsort_by(&mut sinfo, |a, b| cmpf(a, b, &infos));
    let mut ps = PointSet::new();
    let mut places = vec![Point::default(); bbs.len()];
    for (i, &si) in sinfo.iter().enumerate() {
        let inf = &infos[si];
        places[inf.index] = place_graph(i, inf, &mut ps, step, pinfo.margin, bbs);
    }
    verbose_places(&places);
    Some(places)
}

/// `packRects(ng, bbs, pinfo)`: translate the boxes in place; false on failure.
pub fn pack_rects(bbs: &mut [BoundingBox], pinfo: &PackInfo) -> bool {
    if bbs.len() <= 1 {
        return true;
    }
    let Some(pp) = put_rects(bbs, pinfo) else {
        return false;
    };
    for (b, p) in bbs.iter_mut().zip(pp) {
        b.ll = add(b.ll, p);
        b.ur = add(b.ur, p);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #174: only a saturated step gets a budget, and a huge box exhausts it.
    #[test]
    fn saturated_step_box_runs_out_of_budget() {
        let small = BoundingBox::new(0.0, 0.0, 54.0, 36.0);
        assert_eq!(compute_step(&[small, small], 8), 6);
        assert!(Budget::for_step(6).0.is_none());
        let huge = BoundingBox::new(0.0, 0.0, 54.0, 7.2e31);
        let step = compute_step(&[huge, small], 8);
        assert_eq!(step, i32::MAX);
        let mut budget = Budget::for_step(step);
        assert!(gen_box(&huge, step, 8, Point::default(), 0, &mut budget).is_none());
        let mut budget = Budget::for_step(step);
        assert!(gen_box(&small, step, 8, Point::default(), 1, &mut budget).is_some());
    }

    /// Issue #250: C packs the edge of `n1 [pos="-1e12,-1e15!"]`, about 3.4e7 cells at a
    /// saturated step, in about 1 s, so the budget lets it through; it stops at MAX_CELLS.
    #[test]
    fn budget_lets_through_what_c_packs_in_a_second() {
        let cells = 1e15 * 72.0 / f64::from(i32::MAX);
        let mut budget = Budget::for_step(i32::MAX);
        assert!(budget.take(cells.ceil() + 1.0));
        let mut budget = Budget::for_step(i32::MAX);
        assert!(!budget.take(MAX_CELLS + 1.0));
    }

    /// Issue #456: the one-point polyline on sfdp `n217 -> n0` (pack-time coordinates,
    /// step 27). C's `fillEdge` draws through the calloc'd `list[1] == (0,0)` and covers
    /// these cells; repeating `list[0]` does not.
    #[test]
    fn one_point_polyline_fills_through_the_calloc_zero() {
        struct OnePoint {
            spl: Vec<Bezier>,
        }
        impl PackGraph for OnePoint {
            fn pk_ncomps(&self) -> usize {
                0
            }
            fn pk_name(&self, _: usize) -> String {
                String::new()
            }
            fn pk_compute_bb(&mut self, _: usize) {}
            fn pk_graph_bb(&self, _: PkGraph) -> BoundingBox {
                BoundingBox::default()
            }
            fn pk_nodes(&self, _: usize) -> Vec<usize> {
                Vec::new()
            }
            fn pk_clusters(&self, _: PkGraph) -> Vec<usize> {
                Vec::new()
            }
            fn pk_cluster_nodes(&self, _: usize) -> Vec<usize> {
                Vec::new()
            }
            fn pk_node_clust(&self, _: usize) -> Option<PkClust> {
                None
            }
            fn pk_coord(&self, _: usize) -> Point {
                Point::default()
            }
            fn pk_node_size(&self, _: usize) -> Point {
                Point::default()
            }
            fn pk_root_outs(&self, _: usize) -> Vec<usize> {
                Vec::new()
            }
            fn pk_head(&self, _: usize) -> usize {
                0
            }
            fn pk_spl(&self, _: usize) -> Option<&[Bezier]> {
                Some(&self.spl)
            }
            fn pk_sortv(&self, _: usize) -> Option<&str> {
                None
            }
            fn pk_shift_node(&mut self, _: usize, _: f64, _: f64) {}
            fn pk_shift_edge(&mut self, _: usize, _: f64, _: f64) {}
            fn pk_shift_graph_box(&mut self, _: PkGraph, _: f64, _: f64) {}
        }

        let g = OnePoint {
            spl: vec![Bezier {
                list: vec![Point::new(666.2251394528615, 256.1744794455809)],
                eflag: 1,
                ep: Point::new(655.173677660827, 258.8186629413117),
                ..Bezier::default()
            }],
        };
        let mut ps = PointSet::new();
        let mut budget = Budget::for_step(27);
        assert!(fill_edge(
            &g,
            0,
            Point::default(),
            &mut ps,
            0.0,
            0.0,
            27,
            true,
            &mut budget
        ));
        for cell in [(12, 4), (0, 0), (4, 2), (21, 9), (25, 9), (24, 10)] {
            assert!(ps.contains(&cell), "missing {cell:?}");
        }
    }
}
