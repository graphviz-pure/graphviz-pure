//! Spline edges that avoid clusters (`splines=compound`), ported from
//! `lib/fdpgen/clusteredges.c`: `makeClustObs`, `addGraphObjs`,
//! `raiseLevel`, `objectList`, `compoundEdges`; with `getPath` and
//! `makeSpline` from `lib/neatogen/neatosplines.c` (`chkPts` false).

use super::ClusterTree;
use crate::common::splines::{add_edge_labels, clip_and_install};
use crate::common::types::{EdgeType, LGraph, Sub};
use crate::neatogen::adjust::Expand;
use crate::neatogen::legal::plegal_arrangement;
use crate::neatogen::neatosplines::make_self_arcs;
use crate::neatogen::routing::make_obstacle;
use graphviz_pure_core::Point;
use graphviz_pure_geom::pathplan::{POLYID_NONE, Pedge, Vconfig, pobsopen, pobspath, proutespline};

/// The cluster hierarchy as `compoundEdges` sees it.
pub struct Hier<'a> {
    pub tree: &'a ClusterTree,
    /// `PARENT(n)`: smallest containing graph (`None` = root).
    pub parent: &'a [Option<usize>],
    /// `IS_CLUST_NODE(n)`.
    pub is_clust: &'a [bool],
    /// Nodes of each cluster (including nested ones).
    pub members: &'a [Vec<usize>],
}

/// An object excluded from an obstacle list (`tex`/`hex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ex {
    None,
    Node(usize),
    Graph(Option<usize>),
}

impl Hier<'_> {
    fn gparent(&self, g: Option<usize>) -> Option<usize> {
        g.and_then(|c| self.tree.parent[c])
    }

    fn level(&self, g: Option<usize>) -> i32 {
        self.tree.level_of(g)
    }
}

/// `makeClustObs(g, pm)`: the cluster box grown by the margin, clockwise.
fn make_clust_obs(lg: &LGraph<'_>, c: usize, pm: &Expand) -> Vec<Point> {
    let bb = lg.clusters[c].bb;
    let ctr = Point::new((bb.ur.x + bb.ll.x) / 2.0, (bb.ur.y + bb.ll.y) / 2.0);
    let (llx, lly, urx, ury) = if pm.do_add {
        (
            bb.ll.x - pm.x,
            bb.ll.y - pm.y,
            bb.ur.x + pm.x,
            bb.ur.y + pm.y,
        )
    } else {
        let dx = pm.x - 1.0;
        let dy = pm.y - 1.0;
        (
            pm.x.mul_add(bb.ll.x, -(dx * ctr.x)),
            pm.y.mul_add(bb.ll.y, -(dy * ctr.y)),
            pm.x.mul_add(bb.ur.x, -(dx * ctr.x)),
            pm.y.mul_add(bb.ur.y, -(dy * ctr.y)),
        )
    };
    vec![
        Point::new(llx, lly),
        Point::new(llx, ury),
        Point::new(urx, ury),
        Point::new(urx, lly),
    ]
}

/// `addGraphObjs(l, g, tex, hex, pm)`.
fn add_graph_objs(
    lg: &LGraph<'_>,
    h: &Hier<'_>,
    l: &mut Vec<Vec<Point>>,
    g: Option<usize>,
    tex: Ex,
    hex: Ex,
    pm: &Expand,
) {
    let nodes: Vec<usize> = match g {
        None => (0..lg.nodes.len()).collect(),
        Some(c) => h.members[c].clone(),
    };
    for n in nodes {
        if h.parent[n] == g && Ex::Node(n) != tex && Ex::Node(n) != hex && !h.is_clust[n] {
            // makeObstacle returns NULL only for a shape it does not know;
            // C then appends a NULL entry, which the port leaves out.
            if let Some(o) = make_obstacle(lg, n, pm) {
                l.push(o);
            }
        }
    }
    for &sg in h.tree.kids(g) {
        if Ex::Graph(Some(sg)) != tex && Ex::Graph(Some(sg)) != hex {
            l.push(make_clust_obs(lg, sg, pm));
        }
    }
}

/// `raiseLevel(l, maxlvl, ex, minlvl, &g, pm)`; returns the new `g`.
fn raise_level(
    lg: &LGraph<'_>,
    h: &Hier<'_>,
    l: &mut Vec<Vec<Point>>,
    maxlvl: i32,
    mut ex: Ex,
    minlvl: i32,
    g: Option<usize>,
    pm: &Expand,
) -> Ex {
    let mut g = g;
    let mut i = maxlvl;
    while i > minlvl {
        add_graph_objs(lg, h, l, g, ex, Ex::None, pm);
        ex = Ex::Graph(g);
        g = h.gparent(g);
        i -= 1;
    }
    ex
}

/// `objectList(e, pm)`: nodes and clusters to avoid when routing `e`.
fn object_list(lg: &LGraph<'_>, h: &Hier<'_>, e: usize, pm: &Expand) -> Vec<Vec<Point>> {
    let hn = lg.edges[e].head;
    let tn = lg.edges[e].tail;
    let mut hg = h.parent[hn];
    let mut tg = h.parent[tn];
    let mut list = Vec::new();
    let mut hex = if h.is_clust[hn] {
        let x = Ex::Graph(hg);
        hg = h.gparent(hg);
        x
    } else {
        Ex::Node(hn)
    };
    let mut tex = if h.is_clust[tn] {
        let x = Ex::Graph(tg);
        tg = h.gparent(tg);
        x
    } else {
        Ex::Node(tn)
    };
    let hlevel = h.level(hg);
    let tlevel = h.level(tg);
    if hlevel > tlevel {
        let ex = raise_level(lg, h, &mut list, hlevel, hex, tlevel, hg, pm);
        // raiseLevel leaves *gp at the last graph processed.
        hex = ex;
        hg = match ex {
            Ex::Graph(g) => h.gparent(g),
            _ => hg,
        };
    } else if tlevel > hlevel {
        let ex = raise_level(lg, h, &mut list, tlevel, tex, hlevel, tg, pm);
        tex = ex;
        tg = match ex {
            Ex::Graph(g) => h.gparent(g),
            _ => tg,
        };
    }
    while hg != tg {
        add_graph_objs(lg, h, &mut list, hg, Ex::None, hex, pm);
        add_graph_objs(lg, h, &mut list, tg, tex, Ex::None, pm);
        hex = Ex::Graph(hg);
        hg = h.gparent(hg);
        tex = Ex::Graph(tg);
        tg = h.gparent(tg);
    }
    add_graph_objs(lg, h, &mut list, tg, tex, hex, pm);
    list
}

/// `getPath(e, vconfig, false)`; `None` when `Pobspath` finds no path (see `pobspath`).
fn get_path(lg: &LGraph<'_>, e: usize, vconfig: &Vconfig) -> Option<Vec<Point>> {
    let (t, hn) = (lg.edges[e].tail, lg.edges[e].head);
    let p = crate::common::geom::add(lg.nodes[t].coord, lg.edges[e].tail_port.p);
    let q = crate::common::geom::add(lg.nodes[hn].coord, lg.edges[e].head_port.p);
    pobspath(vconfig, p, POLYID_NONE, q, POLYID_NONE)
}

/// `makeSpline(e, obs, npoly, false)`.
fn make_spline(lg: &mut LGraph<'_>, e: usize, obs: &[Vec<Point>]) {
    let line = lg.edges[e].path.clone();
    let mut barriers = Vec::new();
    for o in obs {
        for j in 0..o.len() {
            let k = if j + 1 >= o.len() { 0 } else { j + 1 };
            barriers.push(Pedge { a: o[j], b: o[k] });
        }
    }
    let Ok(spline) = proutespline(&barriers, &line, [Point::default(), Point::default()]) else {
        return;
    };
    let hn = lg.edges[e].head;
    crate::neatogen::routing::verbose_edge(lg, "spline", e);
    clip_and_install(lg, e, hn, &spline);
    add_edge_labels(lg, e);
}

/// The first failure's message in `compoundEdges` (`clusteredges.c:238-246`): `expand_t`
/// holds doubles (`adjust.h:41-44`), `pack` an int.
fn touch_warning(lg: &LGraph<'_>, pm: &Expand) {
    let margin = crate::neatogen::adjust::sep_factor(lg);
    let off = crate::neatogen::neatoinit::CL_OFFSET;
    let pack = crate::pack::get_pack(lg.gattr("pack"), off, off);
    let f3 = |v: f64| graphviz_pure_core::cnum::fmt_f(v, 3);
    graphviz_pure_core::diag::agwarningf(
        "compoundEdges: nodes touch - falling back to straight line edges\n",
    );
    if pack as f64 <= pm.x || pack as f64 <= pm.y {
        graphviz_pure_core::diag::agerr_prev(&format!(
            "pack value {pack} is smaller than esep ({},{})\n",
            f3(pm.x),
            f3(pm.y)
        ));
    } else if margin.x <= pm.x || margin.y <= pm.y {
        graphviz_pure_core::diag::agerr_prev(&format!(
            "sep value ({},{}) is smaller than esep ({},{})\n",
            f3(margin.x),
            f3(margin.y),
            f3(pm.x),
            f3(pm.y)
        ));
    }
}

/// `compoundEdges(g, pm, edgetype)`; returns true on success (C returns 0).
pub fn compound_edges(lg: &mut LGraph<'_>, sub: &Sub, h: &Hier<'_>, pm: &Expand) -> bool {
    let mut ok = true;
    // Kept apart from `ok` so that a path failure, which C has no branch for, leaves C's
    // "nodes touch" warning as it is.
    let mut touched = false;
    let mut no_path = false;
    for &n in &sub.nodes {
        let outs: Vec<usize> = lg.outs(sub, n).collect();
        for e in outs {
            let head = lg.edges[e].head;
            if lg.edges[e].count == 0 {
                continue;
            }
            if n == head {
                let step = lg.nodesep as f64;
                make_self_arcs(lg, e, step);
                continue;
            }
            let objl = object_list(lg, h, e, pm);
            if !plegal_arrangement(&objl) {
                if !touched {
                    touch_warning(lg, pm);
                }
                touched = true;
                ok = false;
                continue;
            }
            let vconfig = pobsopen(&objl);
            let mut e0 = Some(e);
            while let Some(ee) = e0 {
                // C's `Pobspath` loops forever when the head is unreachable (#198), or
                // routes through whatever lies before `dad`. Fail as C does when `Pobsopen`
                // fails: skip the edge and return failure. The warning is the port's own
                // and printed once; unlike C's `rv`, it doesn't hide "nodes touch", so the
                // warnings C prints when it finishes are still printed.
                let Some(path) = get_path(lg, ee, &vconfig) else {
                    if !no_path {
                        graphviz_pure_core::diag::agwarningf(
                            "compoundEdges: could not find a path - falling back to straight line edges\n",
                        );
                    }
                    no_path = true;
                    ok = false;
                    break;
                };
                lg.edges[ee].path = path;
                make_spline(lg, ee, &objl);
                e0 = lg.edges[ee].to_virt;
            }
        }
    }
    ok
}

/// Edge type for `splineEdges(g, compoundEdges, EDGETYPE_SPLINE)`.
pub const COMPOUND_EDGE_TYPE: EdgeType = EdgeType::Spline;
