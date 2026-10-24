//! Port of Graphviz `lib/osage` (the `osage` clustered packing engine),
//! from `osageinit.c`: `osage_layout`, `cluster_init_graph`, `mkClusters`,
//! `layout`, `reposition`.
//!
//! Clusters are laid out bottom-up: each cluster packs its child clusters and
//! its own nodes as rectangles with `putRects` (array packing by default),
//! then `reposition` turns the cluster-relative coordinates into absolute ones.

use crate::common::init::{graph_label, pad};
use crate::common::postproc::gv_postprocess;
use crate::common::types::{EdgeType, LABEL_AT_TOP, LCluster, LGraph, RatioKind, Sub, TextLabel};
use crate::common::utils::late_int;
use crate::fdpgen::{ClusterTree, mk_clusters, subgraph_member_nodes};
use crate::neatogen::adjust::DFLT_MARGIN;
use crate::neatogen::neatosplines::{spline_edges_main, spline_edges0};
use crate::pack::{PK_USER_VALS, PackInfo, PackMode, get_pack, get_pack_mode_info, put_rects};
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
        let mut pinfo = PackInfo {
            margin: get_pack(
                attrs.get("pack").map(String::as_str),
                DFLT_MARGIN as i64,
                DFLT_MARGIN as i64,
            ) as u32,
            ..PackInfo::default()
        };
        let pmode = get_pack_mode_info(
            attrs.get("packmode").map(String::as_str),
            PackMode::Array,
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
                Child::Clust(sc) => self.g.clusters[sc].bb = bb,
                Child::Node(n) => {
                    self.g.nodes[n].coord =
                        Point::new((bb.ll.x + bb.ur.x) / 2.0, (bb.ll.y + bb.ur.y) / 2.0);
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

        // Translate so that rootbb.LL is the origin.
        for ch in &children {
            match *ch {
                Child::Clust(sc) => {
                    let bb = &mut self.g.clusters[sc].bb;
                    bb.ll = Point::new(bb.ll.x - rootbb.ll.x, bb.ll.y - rootbb.ll.y);
                    bb.ur = Point::new(bb.ur.x - rootbb.ll.x, bb.ur.y - rootbb.ll.y);
                }
                Child::Node(n) => {
                    let p = &mut self.g.nodes[n].coord;
                    *p = Point::new(p.x - rootbb.ll.x, p.y - rootbb.ll.y);
                }
            }
        }
        let ur = Point::new(rootbb.ur.x - rootbb.ll.x, rootbb.ur.y - rootbb.ll.y);
        self.set_bb(c, BoundingBox::new(0.0, 0.0, ur.x, ur.y));
    }

    /// `reposition(g, depth)`.
    fn reposition(&mut self, c: Option<usize>, depth: i32) {
        let bb = self.bb(c);
        if depth > 0 {
            for n in self.members(c) {
                if self.parent[n] != Some(c) {
                    continue;
                }
                self.g.nodes[n].coord.x += bb.ll.x;
                self.g.nodes[n].coord.y += bb.ll.y;
            }
        }
        for sc in self.tree.kids(c).to_vec() {
            if depth > 0 {
                let s = &mut self.g.clusters[sc].bb;
                s.ll.x += bb.ll.x;
                s.ll.y += bb.ll.y;
                s.ur.x += bb.ll.x;
                s.ur.y += bb.ll.y;
            }
            self.reposition(Some(sc), depth + 1);
        }
    }
}

/// `do_graph_label(subg)` for the subgraph `si`: its label, `GD_label_pos` and `GD_border`.
pub fn cluster_label(g: &LGraph<'_>, si: usize) -> (Option<TextLabel>, u8, [Point; 4]) {
    let sg = &g.src.subgraphs[si];
    let name = g.src.subgraph_name(si);
    let (label, label_pos) = graph_label(&sg.graph_attrs, &name, sg.html_name, false);
    let mut border = [Point::default(); 4];
    if let Some(l) = &label {
        border[if label_pos & LABEL_AT_TOP != 0 { 2 } else { 0 }] = pad(l.dimen);
    }
    (label, label_pos, border)
}

/// `mkClusters`: register every cluster in `g.clusters` (indexed like the [`ClusterTree`]),
/// with its label and border (`do_graph_label`) when `labels`. fdp makes them later, in
/// `deriveGraph`, one level at a time.
pub fn register_clusters(g: &mut LGraph<'_>, tree: &ClusterTree, labels: bool) {
    for c in 0..tree.len() {
        let (label, label_pos, border) = if labels {
            cluster_label(g, tree.subgraph[c])
        } else {
            (None, 0, [Point::default(); 4])
        };
        g.clusters.push(LCluster {
            subgraph: tree.subgraph[c],
            bb: BoundingBox::default(),
            label,
            label_pos,
            border,
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
    register_clusters(&mut g, &tree, true);
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
        let mut et = g.edge_type;
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
            if et == EdgeType::CompoundEdge {
                // compoundEdges needs fdp's cluster hierarchy; C's
                // spline_edges_ routes it like a polyline.
                et = EdgeType::Pline;
            }
            spline_edges_main(&mut g, &root, et);
        }
    }
    gv_postprocess(&mut g, true);
    g.write_back(graph);
    Ok(())
}
