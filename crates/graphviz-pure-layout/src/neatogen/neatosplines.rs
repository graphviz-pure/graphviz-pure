//! Edge generation for the neato family, ported from
//! `lib/neatogen/neatosplines.c` (`spline_edges`, `spline_edges0`,
//! `splineEdges`, `spline_edges_`, `makeSelfArcs`, `neato_set_aspect`,
//! `neato_translate`) and `lib/common/routespl.c` (`makeStraightEdge(s)`).

use super::adjust::{Expand, esep_factor};
use crate::common::splines::{add_label_bb, make_port_labels, make_self_edge};
use crate::common::types::{EdgeType, LGraph, RatioKind, Sub};
use crate::common::utils::compute_bb;
use graphviz_pure_core::Point;

thread_local! {
    /// `Nop` (`globals.h`): the `-n` level of `neato`, 0 for a full layout.
    static NOP: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// `Nop` for the layout of one of a run's graphs: the run's value, `*nop`, while `f` runs,
/// and the value the layout leaves written back to `*nop` afterwards, unwinding included, so a
/// `Nop = 0` an engine set before a panic still holds for the run's later graphs.
pub(crate) fn with_nop<R>(nop: &mut u8, f: impl FnOnce() -> R) -> R {
    struct Run<'n> {
        nop: &'n mut u8,
        outer: u8,
    }
    impl Drop for Run<'_> {
        fn drop(&mut self) {
            *self.nop = NOP.with(|n| n.replace(self.outer));
        }
    }
    let outer = NOP.with(|n| n.replace(*nop));
    let _run = Run { nop, outer };
    f()
}

/// Sets `Nop = 0` when dropped, unwinding included: the engines that clear `Nop` after
/// their work (`nop1_layout`, `nop2_layout`, `fdpSplines`).
pub(crate) struct ClearNop(());

impl ClearNop {
    pub(crate) fn new() -> Self {
        ClearNop(())
    }
}

impl Drop for ClearNop {
    fn drop(&mut self) {
        set_nop(0);
    }
}

/// `Nop`.
pub(crate) fn nop() -> u8 {
    NOP.with(std::cell::Cell::get)
}

/// `Nop = nop`, for the rest of the run.
pub(crate) fn set_nop(nop: u8) {
    NOP.with(|n| n.set(nop));
}

/// `makeSelfArcs(e, stepx)`.
pub fn make_self_arcs(g: &mut LGraph<'_>, e: usize, stepx: f64) {
    let cnt = g.edges[e].count.max(0) as usize;
    if cnt == 1 || g.concentrate {
        make_self_edge(g, &[e], stepx, stepx);
        if let Some(l) = g.edges[e].label.clone() {
            add_label_bb(&mut g.bb, &l, false);
        }
        make_port_labels(g, e);
    } else if cnt > 1 {
        let mut edges = Vec::with_capacity(cnt);
        let mut cur = Some(e);
        for _ in 0..cnt {
            let Some(c) = cur else { break };
            edges.push(c);
            cur = g.edges[c].to_virt;
        }
        make_self_edge(g, &edges, stepx, stepx);
        for &e in &edges {
            if let Some(l) = g.edges[e].label.clone() {
                add_label_bb(&mut g.bb, &l, false);
            }
            make_port_labels(g, e);
        }
    }
}

/// `makeStraightEdge(g, e, et, sinfo)`: collect `e`'s equivalence class.
pub fn make_straight_edge(g: &mut LGraph<'_>, sub: &Sub, e: usize, et: EdgeType) {
    let mut list = vec![e];
    let mut e0 = e;
    while let Some(n) = g.edges[e0].to_virt {
        if n == e0 {
            break;
        }
        list.push(n);
        e0 = n;
    }
    make_straight_edges(g, sub, &list, et);
}

/// `makeStraightEdges(g, edge_list, e_cnt, et, sinfo)` (shared implementation
/// in [`crate::common::routespl::make_straight_edges`]).
pub fn make_straight_edges(g: &mut LGraph<'_>, sub: &Sub, edge_list: &[usize], et: EdgeType) {
    let (nodesep, conc) = (g.nodesep, g.concentrate);
    crate::common::routespl::make_straight_edges(g, edge_list, et, nodesep, conc, &|g, e| {
        get_cycle_centroid(g, sub, e)
    });
}

/// `equivEdge` key: canonicalised `(n1, p1, n2, p2)`.
fn equiv_key(g: &LGraph<'_>, e: usize) -> (usize, [u64; 2], usize, [u64; 2]) {
    let ed = &g.edges[e];
    let bits = |p: Point| [p.x.to_bits(), p.y.to_bits()];
    let (t, h) = (ed.tail, ed.head);
    let tp = ed.tail_port.p;
    let hp = ed.head_port.p;
    if t < h {
        (t, bits(tp), h, bits(hp))
    } else if t > h {
        (h, bits(hp), t, bits(tp))
    } else {
        let (p1, p2) = if tp.x < hp.x {
            (tp, hp)
        } else if tp.x > hp.x {
            (hp, tp)
        } else if tp.y < hp.y {
            (tp, hp)
        } else if tp.y > hp.y {
            (hp, tp)
        } else {
            (tp, tp)
        };
        (t, bits(p1), t, bits(p2))
    }
}

/// `splineEdges(g, spline_edges_, edgetype)`.
pub fn spline_edges_main(g: &mut LGraph<'_>, sub: &Sub, edgetype: EdgeType) {
    spline_edges_by(g, sub, edgetype, &mut |g, sub, margin, et| {
        spline_edges_route(g, sub, margin, et);
        true
    });
}

/// `splineEdges(g, edgefn, edgetype)`: resolve ports, group equivalent
/// edges (`ED_count`/`ED_to_virt`), then run `edgefn`; false if it failed.
pub fn spline_edges_by(
    g: &mut LGraph<'_>,
    sub: &Sub,
    edgetype: EdgeType,
    edgefn: &mut dyn FnMut(&mut LGraph<'_>, &Sub, &Expand, EdgeType) -> bool,
) -> bool {
    let margin = esep_factor(g);
    for &n in &sub.nodes {
        let outs: Vec<usize> = g.outs(sub, n).collect();
        for e in outs {
            crate::common::ports::resolve_ports(g, e);
        }
    }
    let mut map: std::collections::HashMap<(usize, [u64; 2], usize, [u64; 2]), usize> =
        std::collections::HashMap::new();
    for &n in &sub.nodes {
        let outs: Vec<usize> = g.outs(sub, n).collect();
        for e in outs {
            if nop() > 1
                && let Some(ed) = g.edges.get_mut(e).filter(|ed| ed.spl.is_some())
            {
                // With Nop > 1 an edge that has its spline is its own class.
                ed.count = ed.count.saturating_add(1);
                continue;
            }
            let key = equiv_key(g, e);
            match map.get(&key) {
                Some(&leader) => {
                    g.edges[leader].count += 1;
                    g.edges[e].to_virt = g.edges[leader].to_virt;
                    g.edges[leader].to_virt = Some(e);
                }
                None => {
                    g.edges[e].count = 1;
                    map.insert(key, e);
                }
            }
        }
    }
    edgefn(g, sub, &margin, edgetype)
}

/// `spline_edges_(g, pmargin, edgetype)`.
pub fn spline_edges_route(g: &mut LGraph<'_>, sub: &Sub, pmargin: &Expand, edgetype: EdgeType) {
    if edgetype == EdgeType::Ortho {
        ortho_spline_edges(g, sub, pmargin, edgetype);
        return;
    }
    let router = if edgetype >= EdgeType::Pline {
        super::routing::Router::new(g, sub, pmargin, edgetype)
    } else {
        None
    };
    graphviz_pure_core::verbose_msg!(
        "Creating edges using {}\n",
        match &router {
            Some(_) if edgetype == EdgeType::Spline => "splines",
            Some(_) => "polylines",
            None => "line segments",
        }
    );
    if let Some(r) = &router {
        r.path_pass(g, sub);
    }
    let use_edges = nop() > 1;
    for &n in &sub.nodes {
        let outs: Vec<usize> = g.outs(sub, n).collect();
        for e in outs {
            let head = g.edges[e].head;
            if use_edges && g.edges.get(e).is_some_and(|ed| ed.spl.is_some()) {
                use_edge(g, n, e);
                continue;
            }
            if g.edges[e].count == 0 {
                continue;
            }
            if n == head {
                let step = g.nodesep as f64;
                make_self_arcs(g, e, step);
            } else if let Some(r) = &router {
                r.route_edge(g, sub, e, edgetype);
            } else {
                make_straight_edge(g, sub, e, edgetype);
            }
        }
    }
}

/// `makeObstacle(n, pmargin, isOrtho=true)`: polygon nodes (`SH_POLY`, which
/// includes star and cylinder, and `SH_POINT`) become their outline box
/// (without margin); other shapes as for splines. The ortho branch leaves
/// `margin` at `{0}`, so a scaling (not `+`) margin multiplies every vertex by
/// zero and the box shrinks to the node's centre (`neatosplines.c:431-433`).
fn ortho_obstacle(lg: &LGraph<'_>, n: usize, pm: &Expand) -> Option<Vec<Point>> {
    use crate::common::shapes::InsideFn;
    let nd = &lg.nodes[n];
    let sh = &nd.shape;
    if !matches!(sh.inside, InsideFn::Poly | InsideFn::Star | InsideFn::Point) {
        return super::routing::make_obstacle(lg, n, pm);
    }
    let vs = if sh.fixedshape && sh.sides > 0 && !sh.vertices.is_empty() {
        // fixedsize=shape: the polygon's own box, not the node size with its label
        // (neatosplines.c:351-358).
        let b = sh.poly_bb();
        [
            b.ll,
            Point::new(b.ur.x, b.ll.y),
            b.ur,
            Point::new(b.ll.x, b.ur.y),
        ]
    } else {
        let width = nd.lw + nd.rw;
        let outline_width = sh.outline_width;
        let outline_lw = nd.lw * outline_width / width;
        let outline_ht = sh.outline_height;
        [
            Point::new(-outline_lw, -outline_ht / 2.0),
            Point::new(outline_lw, -outline_ht / 2.0),
            Point::new(outline_lw, outline_ht / 2.0),
            Point::new(-outline_lw, outline_ht / 2.0),
        ]
    };
    let c = nd.coord;
    let k = if pm.do_add { 1.0 } else { 0.0 };
    Some(
        vs.iter()
            .rev()
            .map(|p| Point::new(p.x * k + c.x, p.y * k + c.y))
            .collect(),
    )
}

/// `spline_edges_(g, pmargin, EDGETYPE_ORTHO)`: orthogonal routes when the
/// node boxes do not touch, straight lines otherwise.
fn ortho_spline_edges(lg: &mut LGraph<'_>, sub: &Sub, pm: &Expand, et: EdgeType) {
    let obs: Vec<Vec<Point>> = sub
        .nodes
        .iter()
        .filter_map(|&n| ortho_obstacle(lg, n, pm))
        .collect();
    let mut use_edges = nop() > 1;
    let legal = super::legal::plegal_arrangement(&obs);
    if !legal {
        // neatosplines.c:623
        graphviz_pure_core::diag::agwarningf(
            "the bounding boxes of some nodes touch - falling back to straight line edges\n",
        );
    }
    graphviz_pure_core::verbose_msg!(
        "Creating edges using {}\n",
        if legal {
            "orthogonal lines"
        } else {
            "line segments"
        }
    );
    if legal {
        crate::ortho::ortho_edges_of(lg, sub.nodes.clone(), false);
        use_edges = true;
    }
    for &n in &sub.nodes {
        let outs: Vec<usize> = lg.outs(sub, n).collect();
        for e in outs {
            let head = lg.edges[e].head;
            if use_edges && lg.edges[e].spl.is_some() {
                use_edge(lg, n, e);
            } else if lg.edges[e].count == 0 {
                continue;
            } else if n == head {
                let step = lg.nodesep as f64;
                make_self_arcs(lg, e, step);
            } else {
                make_straight_edge(lg, sub, e, et);
            }
        }
    }
}

/// `spline_edges_`'s branch for an edge that keeps its spline (`useEdges`): its port
/// labels; with `Nop == 3`, its first bezier only, clipped and installed again.
fn use_edge(g: &mut LGraph<'_>, n: usize, e: usize) {
    crate::common::splines::add_edge_labels(g, e);
    if nop() != 3 {
        return;
    }
    let Some(ed) = g.edges.get_mut(e) else { return };
    let head = ed.head;
    let Some(mut spl) = ed.spl.take().filter(|spl| !spl.is_empty()) else {
        return;
    };
    if spl.len() > 1 {
        // neatosplines.c:659: both names share agnameof's static buffer when anonymous.
        let src = g.src;
        let name = |v: usize| {
            src.nodes
                .get(v)
                .map_or(graphviz_pure_core::AgName::own(""), |_| {
                    src.agnameof(graphviz_pure_core::Obj::Node(graphviz_pure_core::NodeId(
                        v as u32,
                    )))
                })
        };
        let [t, h] = graphviz_pure_core::agnameof_buffer([name(n), name(head)]);
        graphviz_pure_core::diag::agwarningf(&format!(
            "edge {t} -> {h} : set more than one spline. First used, other dropped.\n"
        ));
    }
    let first = spl.swap_remove(0);
    if first.list.len() < 4 {
        // A straight edge between overlapping nodes can be a single point. C's
        // clip_and_install reads the three points after it, past the end of the list; the
        // port keeps the first bezier as it is.
        ed.spl = Some(vec![first]);
        return;
    }
    crate::common::splines::clip_and_install(g, e, head, &first.list);
}

/// `spline_edges0(g, set_aspect)`.
pub fn spline_edges0(g: &mut LGraph<'_>, sub: &Sub, is_root: bool, set_aspect: bool) {
    let et = g.edge_type;
    if set_aspect {
        neato_set_aspect(g, sub, is_root);
    }
    if et == EdgeType::None {
        return;
    }
    // C's `spline_edges0` passes `et` through (`neatosplines.c`). Compound is
    // not a polyline: `makeMultiSpline` gets `edgetype == EDGETYPE_PLINE`,
    // which is false for `EDGETYPE_COMPOUND`, so a multi-edge is a cubic. A
    // single edge still falls through to `makePolyline`.
    spline_edges_main(g, sub, et);
}

/// `spline_edges(g)`: translate `sub` so its bounding box starts at the
/// origin, then route its edges.
pub fn spline_edges(g: &mut LGraph<'_>, sub: &mut Sub, is_root: bool) {
    compute_bb(g, sub, is_root);
    let ll = sub.bb.ll;
    let off = Point::new(ll.x / 72.0, ll.y / 72.0);
    for &n in &sub.nodes {
        g.nodes[n].pos[0] -= off.x;
        g.nodes[n].pos[1] -= off.y;
    }
    shift_clusters(g, sub, ll);
    if is_root {
        g.bb = sub.bb;
    }
    spline_edges0(g, sub, is_root, true);
    if is_root {
        sub.bb = g.bb;
    }
}

fn shift_clusters(g: &mut LGraph<'_>, sub: &mut Sub, off: Point) {
    fn rec(g: &mut LGraph<'_>, c: usize, off: Point) {
        for k in g.clusters[c].children.clone() {
            rec(g, k, off);
        }
        let bb = &mut g.clusters[c].bb;
        bb.ur.x -= off.x;
        bb.ur.y -= off.y;
        bb.ll.x -= off.x;
        bb.ll.y -= off.y;
    }
    for c in sub.clusters.clone() {
        rec(g, c, off);
    }
    sub.bb.ur.x -= off.x;
    sub.bb.ur.y -= off.y;
    sub.bb.ll.x -= off.x;
    sub.bb.ll.y -= off.y;
}

/// `neato_translate(g)`: move the root so its bounding box starts at the origin.
pub fn neato_translate(g: &mut LGraph<'_>) {
    let ll = g.bb.ll;
    let off = Point::new(ll.x / 72.0, ll.y / 72.0);
    for n in &mut g.nodes {
        n.pos[0] -= off.x;
        n.pos[1] -= off.y;
        if let Some(l) = n.xlabel.as_mut().filter(|l| l.set) {
            l.pos.x -= ll.x;
            l.pos.y -= ll.y;
        }
    }
    for e in &mut g.edges {
        if let Some(spl) = e.spl.as_mut() {
            for b in spl.iter_mut() {
                for p in &mut b.list {
                    p.x -= ll.x;
                    p.y -= ll.y;
                }
                if b.sflag != 0 {
                    b.sp.x -= ll.x;
                    b.sp.y -= ll.y;
                }
                if b.eflag != 0 {
                    b.ep.x -= ll.x;
                    b.ep.y -= ll.y;
                }
            }
            for l in [
                &mut e.label,
                &mut e.xlabel,
                &mut e.head_label,
                &mut e.tail_label,
            ]
            .into_iter()
            .flatten()
            {
                if l.set {
                    l.pos.x -= ll.x;
                    l.pos.y -= ll.y;
                }
            }
        }
    }
    translate_g(g, ll);
}

fn translate_g(g: &mut LGraph<'_>, off: Point) {
    let sh = |b: &mut graphviz_pure_core::BoundingBox| {
        b.ur.x -= off.x;
        b.ur.y -= off.y;
        b.ll.x -= off.x;
        b.ll.y -= off.y;
    };
    sh(&mut g.bb);
    if let Some(l) = g.label.as_mut().filter(|l| l.set) {
        l.pos.x -= off.x;
        l.pos.y -= off.y;
    }
    for c in &mut g.clusters {
        sh(&mut c.bb);
        if let Some(l) = c.label.as_mut().filter(|l| l.set) {
            l.pos.x -= off.x;
            l.pos.y -= off.y;
        }
    }
}

fn scale_bb(g: &mut LGraph<'_>, xf: f64, yf: f64) {
    let sc = |b: &mut graphviz_pure_core::BoundingBox| {
        b.ur.x *= xf;
        b.ur.y *= yf;
        b.ll.x *= xf;
        b.ll.y *= yf;
    };
    sc(&mut g.bb);
    if let Some(l) = g.label.as_mut().filter(|l| l.set) {
        l.pos.x *= xf;
        l.pos.y *= yf;
    }
    for c in &mut g.clusters {
        sc(&mut c.bb);
        if let Some(l) = c.label.as_mut().filter(|l| l.set) {
            l.pos.x *= xf;
            l.pos.y *= yf;
        }
    }
}

/// `_neato_set_aspect(g)`; true if a transformation was applied.
fn neato_set_aspect_inner(g: &mut LGraph<'_>, is_root: bool) -> bool {
    if !is_root {
        return false;
    }
    let kind = g.drawing.ratio_kind;
    if kind == RatioKind::None {
        return false;
    }
    let mut translated = false;
    if g.bb.ll.x != 0.0 || g.bb.ll.y != 0.0 {
        translated = true;
        neato_translate(g);
    }
    let (xf, yf);
    match kind {
        RatioKind::Fill => {
            if g.drawing.size.x <= 0.0 {
                return translated;
            }
            let mut x = g.drawing.size.x / g.bb.ur.x;
            let mut y = g.drawing.size.y / g.bb.ur.y;
            if x < 1.0 || y < 1.0 {
                if x < y {
                    y /= x;
                    x = 1.0;
                } else {
                    x /= y;
                    y = 1.0;
                }
            }
            (xf, yf) = (x, y);
        }
        RatioKind::Expand => {
            if g.drawing.size.x <= 0.0 {
                return translated;
            }
            let x = g.drawing.size.x / g.bb.ur.x;
            let y = g.drawing.size.y / g.bb.ur.y;
            if x > 1.0 && y > 1.0 {
                let s = x.min(y);
                (xf, yf) = (s, s);
            } else {
                return translated;
            }
        }
        RatioKind::Value => {
            let desired = g.drawing.ratio;
            let actual = g.bb.ur.y / g.bb.ur.x;
            if actual < desired {
                (xf, yf) = (1.0, desired / actual);
            } else {
                (xf, yf) = (actual / desired, 1.0);
            }
        }
        _ => return translated,
    }
    if nop() > 1 {
        for e in 0..g.edges.len() {
            scale_edge(g, e, xf, yf);
        }
    }
    for n in &mut g.nodes {
        n.pos[0] *= xf;
        n.pos[1] *= yf;
    }
    scale_bb(g, xf, yf);
    true
}

/// `scaleEdge(e, xf, yf)`: the ends of a given spline move with their nodes, its inner
/// points and label scale.
fn scale_edge(g: &mut LGraph<'_>, e: usize, xf: f64, yf: f64) {
    let nodes = &g.nodes;
    let del = |v: usize| {
        let p = nodes.get(v).map_or(&[][..], |n| &n.pos[..]);
        let (x, y) = (p.first().copied(), p.get(1).copied());
        Point::new(
            72.0 * (x.unwrap_or(0.0) * (xf - 1.0)),
            72.0 * (y.unwrap_or(0.0) * (yf - 1.0)),
        )
    };
    let Some(ed) = g.edges.get_mut(e) else { return };
    let (delh, delt) = (del(ed.head), del(ed.tail));
    let Some(spl) = ed.spl.as_mut() else { return };
    let last_bez = spl.len().saturating_sub(1);
    for (i, bez) in spl.iter_mut().enumerate() {
        let last_pt = bez.list.len().saturating_sub(1);
        for (j, pt) in bez.list.iter_mut().enumerate() {
            if i == 0 && j == 0 {
                pt.x += delt.x;
                pt.y += delt.y;
            } else if i == last_bez && j == last_pt {
                pt.x += delh.x;
                pt.y += delh.y;
            } else {
                pt.x *= xf;
                pt.y *= yf;
            }
        }
        if bez.sflag != 0 {
            bez.sp.x += delt.x;
            bez.sp.y += delt.y;
        }
        if bez.eflag != 0 {
            bez.ep.x += delh.x;
            bez.ep.y += delh.y;
        }
    }
    if let Some(l) = ed.label.as_mut().filter(|l| l.set) {
        l.pos.x *= xf;
        l.pos.y *= yf;
    }
    if let Some(l) = ed.head_label.as_mut().filter(|l| l.set) {
        l.pos.x += delh.x;
        l.pos.y += delh.y;
    }
    if let Some(l) = ed.tail_label.as_mut().filter(|l| l.set) {
        l.pos.x += delt.x;
        l.pos.y += delt.y;
    }
}

/// `neato_set_aspect(g)`: apply `ratio` and copy `ND_pos` (inches) to `ND_coord` (points).
pub fn neato_set_aspect(g: &mut LGraph<'_>, sub: &Sub, is_root: bool) -> bool {
    let moved = neato_set_aspect_inner(g, is_root);
    for &n in &sub.nodes {
        let p = &g.nodes[n].pos;
        g.nodes[n].coord = Point::new(72.0 * p[0], 72.0 * p[1]);
    }
    moved
}

/// `find_all_cycles` / `get_cycle_centroid` from `routespl.c`.
fn get_cycle_centroid(g: &LGraph<'_>, sub: &Sub, edge: usize) -> Point {
    fn dfs(
        g: &LGraph<'_>,
        sub: &Sub,
        search: usize,
        visited: &mut Vec<usize>,
        end: usize,
        cycles: &mut Vec<Vec<usize>>,
    ) {
        if visited.contains(&search) {
            if search == end {
                let unique = !cycles
                    .iter()
                    .any(|c| c.len() == visited.len() && c.iter().all(|x| visited.contains(x)));
                if unique {
                    cycles.push(visited.clone());
                }
            }
        } else {
            visited.push(search);
            let outs: Vec<usize> = g.outs(sub, search).collect();
            for e in outs {
                dfs(g, sub, g.edges[e].head, visited, end, cycles);
            }
            visited.pop();
        }
    }
    let mut cycles = Vec::new();
    for &n in &sub.nodes {
        let mut visited = Vec::new();
        dfs(g, sub, n, &mut visited, n, &mut cycles);
    }
    let (start, end) = (g.edges[edge].tail, g.edges[edge].head);
    let contains = |c: &Vec<usize>| {
        let l = c.len();
        (0..l).any(|i| c[if i == 0 { l - 1 } else { i - 1 }] == start && c[i] == end)
    };
    let mut shortest: Option<&Vec<usize>> = None;
    for c in &cycles {
        if c.len() < 3 {
            continue;
        }
        if (shortest.is_none() || shortest.unwrap().len() > c.len()) && contains(c) {
            shortest = Some(c);
        }
    }
    let Some(cycle) = shortest else {
        let bb = if sub.nodes.len() == g.nodes.len() {
            g.bb
        } else {
            sub.bb
        };
        return Point::new((bb.ll.x + bb.ur.x) / 2.0, (bb.ll.y + bb.ur.y) / 2.0);
    };
    let mut sum = Point::new(0.0, 0.0);
    let mut cnt = 0.0;
    for &n in cycle {
        sum.x += g.nodes[n].coord.x;
        sum.y += g.nodes[n].coord.y;
        cnt += 1.0;
    }
    Point::new(sum.x / cnt, sum.y / cnt)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue #332: a `Nop = 0` set before a panic holds for the run's later graphs, and a
    // nop engine called on its own leaves no `Nop` on the thread, even when it panics.
    #[test]
    fn nop_is_written_back_and_cleared_on_unwind() {
        let mut run = 2;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_nop(&mut run, || {
                assert_eq!(nop(), 2);
                set_nop(0);
                panic!("layout");
            })
        }));
        assert!(caught.is_err());
        assert_eq!((run, nop()), (0, 0));

        let caught = std::panic::catch_unwind(|| {
            let _clear = ClearNop::new();
            set_nop(2);
            panic!("layout");
        });
        assert!(caught.is_err());
        assert_eq!(nop(), 0);
    }
}
