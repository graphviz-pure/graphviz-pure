//! Port of Graphviz `lib/osage` (the `osage` clustered packing engine),
//! from `osageinit.c`: `osage_layout`, `cluster_init_graph`, `mkClusters`,
//! `layout`, `reposition`.
//!
//! Clusters are laid out bottom-up: each cluster packs its child clusters and
//! its own nodes as rectangles with `putRects` (array packing by default),
//! then `reposition` turns the cluster-relative coordinates into absolute ones.

use crate::common::init::do_graph_label;
use crate::common::postproc::gv_postprocess;
use crate::common::types::{EdgeType, LCluster, LGraph, RatioKind, Sub};
use crate::common::utils::late_int;
use crate::fdpgen::{ClusterTree, mk_clusters, subgraph_member_nodes};
use crate::neatogen::adjust::DFLT_MARGIN;
use crate::neatogen::neatosplines::{spline_edges_main, spline_edges0};
use crate::pack::{PK_USER_VALS, PackInfo, PackMode, put_rects};
use graphviz_pure_core::{AttrMap, BoundingBox, Graph, Point, Result};

/// Size of an empty cluster, in points.
const DFLT_SZ: f64 = 18.0;

enum Child {
    Clust(usize),
    Node(usize),
}

struct Osage<'g, 'a> {
    g: &'g mut LGraph<'a>,
    tree: ClusterTree,
    /// `PARENT(n)` (`ND_alg`): the cluster (or root, `Some(None)`) that placed `n`.
    parent: Vec<Option<Option<usize>>>,
    /// `GD_bb` of the root.
    root_bb: BoundingBox,
    /// Whether `sortv` is declared for graphs / nodes.
    sortv_graph: bool,
    sortv_node: bool,
}

impl Osage<'_, '_> {
    fn attrs(&self, c: Option<usize>) -> &AttrMap {
        match c {
            None => &self.g.src.graph_attrs,
            Some(c) => &self.g.src.subgraphs[self.tree.subgraph[c]].graph_attrs,
        }
    }

    fn members(&self, c: Option<usize>) -> Vec<usize> {
        match c {
            None => (0..self.g.nodes.len()).collect(),
            Some(c) => subgraph_member_nodes(self.g.src, self.tree.subgraph[c]),
        }
    }

    fn bb(&self, c: Option<usize>) -> BoundingBox {
        match c {
            None => self.root_bb,
            Some(c) => self.g.clusters[c].bb,
        }
    }

    fn set_bb(&mut self, c: Option<usize>, bb: BoundingBox) {
        match c {
            None => self.root_bb = bb,
            Some(c) => self.g.clusters[c].bb = bb,
        }
    }

    /// `layout(g, depth)`.
    fn layout(&mut self, c: Option<usize>, depth: i32) {
        self.trace(depth, &format!("layout {}", self.gname(c)));
        let kids = self.tree.kids(c).to_vec();
        let mut nvs = 0usize;
        for &sc in &kids {
            self.layout(Some(sc), depth + 1);
            nvs += self.members(Some(sc)).len();
        }
        let members = self.members(c);
        let nv = members.len();
        let total = nv as i64 - nvs as i64 + kids.len() as i64;
        let label_dimen = match c {
            None => self.g.label.as_ref().map(|l| l.dimen),
            Some(c) => self.g.clusters[c].label.as_ref().map(|l| l.dimen),
        };
        if total == 0 && label_dimen.is_none() {
            self.set_bb(c, BoundingBox::new(0.0, 0.0, DFLT_SZ, DFLT_SZ));
            return;
        }

        // getPackInfo(g, l_array, DFLT_MARGIN, &pinfo)
        let attrs = self.attrs(c);
        let mut pinfo = PackInfo::default();
        let pmode = crate::pack::pack_info(
            attrs.get("pack").map(String::as_str),
            attrs.get("packmode").map(String::as_str),
            PackMode::Array,
            DFLT_MARGIN as i64,
            &mut pinfo,
        );
        if pmode < PackMode::Graph {
            pinfo.mode = PackMode::Graph;
        }
        let (mut cattr, mut vattr) = (false, false);
        if pinfo.mode == PackMode::Array && pinfo.flags & PK_USER_VALS != 0 {
            cattr = self.sortv_graph;
            vattr = self.sortv_node;
            if !cattr && !vattr {
                // osageinit.c:104-105, without a newline.
                let name = match c {
                    None => self.g.src.root_name().to_string(),
                    Some(c) => self.g.src.subgraph_name(self.tree.subgraph[c]),
                };
                graphviz_pure_core::diag::agwarningf(&format!(
                    "Graph {name} has array packing with user values but no \"sortv\" attributes \
                     are defined."
                ));
            }
        }

        let mut gs: Vec<BoundingBox> = Vec::new();
        let mut children: Vec<Child> = Vec::new();
        let mut vals: Vec<u32> = Vec::new();
        for &sc in &kids {
            gs.push(self.g.clusters[sc].bb);
            if cattr {
                vals.push(late_int(self.attrs(Some(sc)), "sortv", 0, 0) as u32);
            }
            children.push(Child::Clust(sc));
        }
        if nv > nvs {
            for &n in &members {
                if self.parent[n].is_some() {
                    continue;
                }
                self.parent[n] = Some(c);
                let nd = &self.g.nodes[n];
                gs.push(BoundingBox::new(0.0, 0.0, nd.lw + nd.rw, nd.ht));
                if vattr {
                    vals.push(late_int(&self.g.src.nodes[n].attrs, "sortv", 0, 0) as u32);
                }
                children.push(Child::Node(n));
            }
        }

        if !vals.is_empty() {
            // C indexes `vals` by box; a short list reads past its end.
            vals.resize(gs.len(), 0);
            pinfo.vals = Some(vals);
        }
        let pts = put_rects(&gs, &pinfo).unwrap_or_else(|| vec![Point::default(); gs.len()]);

        let m = f64::MAX;
        let mut rootbb = BoundingBox::new(m, m, -m, -m);
        for (i, p) in pts.iter().enumerate() {
            let mut bb = gs[i];
            bb.ll = Point::new(bb.ll.x + p.x, bb.ll.y + p.y);
            bb.ur = Point::new(bb.ur.x + p.x, bb.ur.y + p.y);
            rootbb.ll.x = rootbb.ll.x.min(bb.ll.x);
            rootbb.ll.y = rootbb.ll.y.min(bb.ll.y);
            rootbb.ur.x = rootbb.ur.x.max(bb.ur.x);
            rootbb.ur.y = rootbb.ur.y.max(bb.ur.y);
            match children[i] {
                Child::Clust(sc) => {
                    self.g.clusters[sc].bb = bb;
                    self.trace_bb(depth, &self.gname(Some(sc)), bb);
                }
                Child::Node(n) => {
                    self.g.nodes[n].coord =
                        Point::new((bb.ll.x + bb.ur.x) / 2.0, (bb.ll.y + bb.ur.y) / 2.0);
                    self.trace_node(depth, n);
                }
            }
        }

        if let Some(pt) = label_dimen {
            if total == 0 {
                rootbb = BoundingBox::new(0.0, 0.0, pt.x, pt.y);
            }
            let mut d = pt.x - (rootbb.ur.x - rootbb.ll.x);
            if d > 0.0 {
                d /= 2.0;
                rootbb.ll.x -= d;
                rootbb.ur.x += d;
            }
        }

        let margin = if depth > 0 {
            f64::from(pinfo.margin) / 2.0
        } else {
            0.0
        };
        let border = match c {
            None => [Point::default(); 4],
            Some(c) => self.g.clusters[c].border,
        };
        rootbb.ll.x -= margin;
        rootbb.ur.x += margin;
        rootbb.ll.y -= margin + border[0].y;
        rootbb.ur.y += margin + border[2].y;
        self.trace_bb(depth, &self.gname(c), rootbb);

        // Translate so that rootbb.LL is the origin.
        for ch in &children {
            match *ch {
                Child::Clust(sc) => {
                    let bb = &mut self.g.clusters[sc].bb;
                    bb.ll = Point::new(bb.ll.x - rootbb.ll.x, bb.ll.y - rootbb.ll.y);
                    bb.ur = Point::new(bb.ur.x - rootbb.ll.x, bb.ur.y - rootbb.ll.y);
                    let bb = *bb;
                    self.trace_bb(depth, &self.gname(Some(sc)), bb);
                }
                Child::Node(n) => {
                    let p = &mut self.g.nodes[n].coord;
                    *p = Point::new(p.x - rootbb.ll.x, p.y - rootbb.ll.y);
                    self.trace_node(depth, n);
                }
            }
        }
        let ur = Point::new(rootbb.ur.x - rootbb.ll.x, rootbb.ur.y - rootbb.ll.y);
        let bb = BoundingBox::new(0.0, 0.0, ur.x, ur.y);
        self.trace_bb(depth, &self.gname(c), bb);
        self.set_bb(c, bb);
    }

    /// `agnameof(g)` of the root or a cluster.
    fn gname(&self, c: Option<usize>) -> String {
        match c.and_then(|c| self.tree.subgraph.get(c)) {
            Some(&s) => self.g.src.subgraph_name(s),
            None => self.g.src.root_name().to_string(),
        }
    }

    /// osage's `Verbose > 1` trace: `indent(depth)` and a line.
    fn trace(&self, depth: i32, line: &str) {
        if graphviz_pure_core::diag::verbose_byte() > 1 {
            let indent = "  ".repeat(depth.max(0) as usize);
            graphviz_pure_core::diag::fprintf_stderr(&format!("{indent}{line}\n"));
        }
    }

    fn trace_bb(&self, depth: i32, name: &str, bb: BoundingBox) {
        let f = |v: f64| graphviz_pure_core::diag::fmt_f(v, 6);
        self.trace(
            depth,
            &format!(
                "{name} : {} {} {} {}",
                f(bb.ll.x),
                f(bb.ll.y),
                f(bb.ur.x),
                f(bb.ur.y)
            ),
        );
    }

    fn trace_node(&self, depth: i32, n: usize) {
        let f = |v: f64| graphviz_pure_core::diag::fmt_f(v, 6);
        let p = self.g.nodes.get(n).map(|nd| nd.coord).unwrap_or_default();
        self.trace(
            depth,
            &format!("{} : {} {}", self.g.node_agnameof(n).name, f(p.x), f(p.y)),
        );
    }

    /// `reposition(g, depth)`.
    fn reposition(&mut self, c: Option<usize>, depth: i32) {
        let bb = self.bb(c);
        self.trace(depth, &format!("reposition {}", self.gname(c)));
        if depth > 0 {
            for n in self.members(c) {
                if self.parent[n] != Some(c) {
                    continue;
                }
                self.g.nodes[n].coord.x += bb.ll.x;
                self.g.nodes[n].coord.y += bb.ll.y;
                self.trace_node(depth, n);
            }
        }
        for sc in self.tree.kids(c).to_vec() {
            if depth > 0 {
                let s = &mut self.g.clusters[sc].bb;
                s.ll.x += bb.ll.x;
                s.ll.y += bb.ll.y;
                s.ur.x += bb.ll.x;
                s.ur.y += bb.ll.y;
                let s = *s;
                self.trace_bb(depth, &self.gname(Some(sc)), s);
            }
            self.reposition(Some(sc), depth + 1);
        }
    }
}

/// `mkClusters`: register every cluster in `g.clusters` (indexed like the [`ClusterTree`]),
/// without a label: osage gives them all theirs up front, fdp one level at a time in
/// `deriveGraph` ([`do_graph_label`]).
pub fn register_clusters(g: &mut LGraph<'_>, tree: &ClusterTree) {
    for c in 0..tree.len() {
        g.clusters.push(LCluster {
            subgraph: tree.subgraph[c],
            bb: BoundingBox::default(),
            label: None,
            label_pos: 0,
            border: [Point::default(); 4],
            children: tree.children[c].clone(),
        });
    }
    g.root_clusters = tree.root.clone();
}

/// `osage_layout(g)`.
pub fn osage_layout(graph: &mut Graph) -> Result<()> {
    let src = graph.clone();
    let mut g = LGraph::new(&src, 2, EdgeType::Line);
    let tree = mk_clusters(&src);
    register_clusters(&mut g, &tree);
    // osage's mkClusters makes each cluster's label as it registers it, in tree order.
    for c in 0..g.clusters.len() {
        do_graph_label(&mut g, c);
    }
    let sortv_graph = src.graph_attr_declared("sortv");
    let sortv_node = g.node_attr_declared("sortv");
    let n = g.nodes.len();
    let mut o = Osage {
        g: &mut g,
        tree,
        parent: vec![None; n],
        root_bb: BoundingBox::default(),
        sortv_graph,
        sortv_node,
    };
    o.layout(None, 0);
    o.reposition(None, 0);
    let root_bb = o.root_bb;
    g.bb = root_bb;

    let root: Sub = g.root_sub();
    if g.drawing.ratio_kind != RatioKind::None {
        for nd in &mut g.nodes {
            nd.pos[0] = nd.coord.x / 72.0;
            nd.pos[1] = nd.coord.y / 72.0;
        }
        spline_edges0(&mut g, &root, true, true);
    } else {
        let et = g.edge_type;
        if et == EdgeType::Ortho {
            crate::neatogen::neatosplines::spline_edges_by(
                &mut g,
                &root,
                et,
                &mut |lg, sub, pm, et| {
                    crate::neatogen::neatosplines::spline_edges_route(lg, sub, pm, et);
                    true
                },
            );
        } else if et != EdgeType::None {
            // C's `osage_layout` calls `spline_edges1(g, et)` with the original
            // type (`osageinit.c`). `makeMultiSpline`'s polyline flag is
            // `edgetype == EDGETYPE_PLINE` (`neatosplines.c`), false for
            // `EDGETYPE_COMPOUND`, so a multi-edge is a cubic. A single edge
            // still falls through to `makePolyline`.
            spline_edges_main(&mut g, &root, et);
        }
    }
    gv_postprocess(&mut g, true);
    g.write_back(graph);
    Ok(())
}
