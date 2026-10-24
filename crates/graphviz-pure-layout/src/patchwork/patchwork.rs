//! The treemap layout proper, ported from `lib/patchwork/patchwork.c`:
//! `mkTree`, `mkTreeNode`, `getArea`, `fullArea`, `layoutTree`, `walkTree`,
//! `patchworkLayout`.

use super::tree_map::{Rectangle, tree_map};
use crate::common::utils::late_double;
use crate::cutil::qsort_by;
use crate::fdpgen::{ClusterTree, subgraph_member_nodes};
use graphviz_pure_core::cnum::is_exactly_zero;
use graphviz_pure_core::{AttrMap, BoundingBox, Graph, Point};

const DFLT_SZ: f64 = 1.0;
/// Scale up so that 1 is a reasonable default size.
const SCALE: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A cluster (index into the [`ClusterTree`]) or the root (`None`).
    Graph(Option<usize>),
    Node(usize),
}

/// `treenode_t`.
#[derive(Debug, Clone)]
struct TreeNode {
    area: f64,
    child_area: f64,
    r: Rectangle,
    children: Vec<usize>,
    kind: Kind,
}

/// Result of [`patchwork_layout_tree`]: node rectangles and graph boxes, in points.
#[derive(Debug, Clone, Default)]
pub struct PatchworkResult {
    /// Per node: centre and size, if the node was placed.
    pub nodes: Vec<Option<(Point, Point)>>,
    /// Per cluster of the [`ClusterTree`].
    pub clusters: Vec<BoundingBox>,
    pub root_bb: BoundingBox,
    /// The nodes in `walkTree` order, in which `finishNode` calls `common_init_node`.
    pub walk_order: Vec<usize>,
}

struct Builder<'a> {
    graph: &'a Graph,
    tree: &'a ClusterTree,
    nodes: Vec<TreeNode>,
    sparent: Vec<bool>,
}

impl Builder<'_> {
    fn gattrs(&self, c: Option<usize>) -> &AttrMap {
        match c {
            None => &self.graph.graph_attrs,
            Some(c) => &self.graph.subgraphs[self.tree.subgraph[c]].graph_attrs,
        }
    }

    /// `getArea(obj, ap)`. An undeclared attribute reads like an empty one.
    fn get_area(attrs: &AttrMap) -> f64 {
        let mut area = late_double(attrs, "area", DFLT_SZ, 0.0);
        if is_exactly_zero(area) {
            area = DFLT_SZ;
        }
        area * SCALE
    }

    /// `fullArea(p, mp)`.
    fn full_area(&self, p: &TreeNode) -> f64 {
        let Kind::Graph(c) = p.kind else {
            unreachable!()
        };
        let m = late_double(self.gattrs(c), "inset", 0.0, 0.0);
        let wid = 2.0 * m + p.child_area.sqrt();
        wid * wid
    }

    /// `mkTreeNode(n, ap)`.
    fn mk_tree_node(&mut self, n: usize) -> usize {
        let area = Self::get_area(&self.graph.nodes[n].attrs);
        self.nodes.push(TreeNode {
            area,
            child_area: 0.0,
            r: Rectangle::default(),
            children: Vec::new(),
            kind: Kind::Node(n),
        });
        self.nodes.len() - 1
    }

    /// `mkTree(g, gp, ap, mp)`.
    fn mk_tree(&mut self, c: Option<usize>) -> usize {
        let mut children = Vec::new();
        let mut area = 0.0;
        let clusters = match c {
            None => self.tree.root.clone(),
            Some(c) => self.tree.children[c].clone(),
        };
        for sc in clusters {
            let cp = self.mk_tree(Some(sc));
            area += self.nodes[cp].area;
            children.push(cp);
        }
        let members: Vec<usize> = match c {
            None => (0..self.graph.nodes.len()).collect(),
            Some(c) => subgraph_member_nodes(self.graph, self.tree.subgraph[c]),
        };
        for n in members {
            if self.sparent[n] {
                continue;
            }
            let cp = self.mk_tree_node(n);
            area += self.nodes[cp].area;
            children.push(cp);
            self.sparent[n] = true;
        }
        let mut p = TreeNode {
            area: 0.0,
            child_area: 0.0,
            r: Rectangle::default(),
            children,
            kind: Kind::Graph(c),
        };
        if !p.children.is_empty() {
            p.child_area = area;
            p.area = self.full_area(&p);
        } else {
            p.area = Self::get_area(self.gattrs(c));
        }
        self.nodes.push(p);
        self.nodes.len() - 1
    }

    /// `layoutTree(tree)`.
    fn layout_tree(&mut self, t: usize) {
        if self.nodes[t].children.is_empty() {
            return;
        }
        let mut list = self.nodes[t].children.clone();
        let nodes = &self.nodes;
        qsort_by(&mut list, |&a, &b| {
            let (a, b) = (nodes[a].area, nodes[b].area);
            if a < b {
                1
            } else if a > b {
                -1
            } else {
                0
            }
        });
        let areas_sorted: Vec<f64> = list.iter().map(|&i| self.nodes[i].area).collect();
        let tree = &self.nodes[t];
        let h = tree.r.size[1];
        let w = tree.r.size[0];
        let delta = h - w;
        // C (clang) fuses this multiply-add.
        let disc = delta.mul_add(delta, 4.0 * tree.child_area).sqrt();
        let m = (h + w - disc) / 2.0;
        let crec = Rectangle {
            x: [tree.r.x[0], tree.r.x[1]],
            size: [w - m, h - m],
        };
        if let Some(recs) = tree_map(&areas_sorted, crec) {
            use graphviz_pure_core::diag::fmt_f;
            let f = |v: f64| fmt_f(v, 6);
            if let Some(r) = self.nodes.get(t).map(|nd| nd.r) {
                graphviz_pure_core::verbose_msg!(
                    "rec {} {} {} {}\n",
                    f(r.x[0]),
                    f(r.x[1]),
                    f(r.size[0]),
                    f(r.size[1])
                );
            }
            for ((&c, &q), &area) in list.iter().zip(&recs).zip(&areas_sorted) {
                if let Some(nd) = self.nodes.get_mut(c) {
                    nd.r = q;
                }
                graphviz_pure_core::verbose_msg!(
                    "{} - {} {} {} {} = {} ({} {} {} {})\n",
                    f(area),
                    f(q.x[0] - q.size[0] * 0.5),
                    f(q.x[1] - q.size[1] * 0.5),
                    f(q.x[0] + q.size[0] * 0.5),
                    f(q.x[1] + q.size[1] * 0.5),
                    f(q.size[0] * q.size[1]),
                    f(q.x[0]),
                    f(q.x[1]),
                    f(q.size[0]),
                    f(q.size[1])
                );
            }
        }
        for c in self.nodes[t].children.clone() {
            if matches!(self.nodes[c].kind, Kind::Graph(_)) {
                self.layout_tree(c);
            }
        }
    }

    /// `walkTree(tree)`.
    fn walk_tree(&self, t: usize, out: &mut PatchworkResult) {
        let tree = &self.nodes[t];
        match tree.kind {
            Kind::Graph(c) => {
                for &p in &tree.children {
                    self.walk_tree(p, out);
                }
                let x0 = tree.r.x[0];
                let y0 = tree.r.x[1];
                let wd = tree.r.size[0];
                let ht = tree.r.size[1];
                let llx = x0 - wd / 2.0;
                let lly = y0 - ht / 2.0;
                let r = BoundingBox::new(llx, lly, llx + wd, lly + ht);
                match c {
                    None => out.root_bb = r,
                    Some(c) => out.clusters[c] = r,
                }
            }
            Kind::Node(n) => {
                let rr = tree.r;
                out.nodes[n] = Some((
                    Point::new(rr.x[0], rr.x[1]),
                    Point::new(rr.size[0], rr.size[1]),
                ));
                out.walk_order.push(n);
            }
        }
    }
}

/// `patchworkLayout(g)`.
pub fn patchwork_layout_tree(graph: &Graph, tree: &ClusterTree) -> PatchworkResult {
    let mut b = Builder {
        graph,
        tree,
        nodes: Vec::new(),
        sparent: vec![false; graph.nodes.len()],
    };
    let root = b.mk_tree(None);
    let total = b.nodes[root].area;
    let side = (total + 0.1).sqrt();
    b.nodes[root].r = Rectangle {
        x: [0.0, 0.0],
        size: [side, side],
    };
    b.layout_tree(root);
    let mut out = PatchworkResult {
        nodes: vec![None; graph.nodes.len()],
        clusters: vec![BoundingBox::default(); tree.subgraph.len()],
        root_bb: BoundingBox::default(),
        walk_order: Vec::new(),
    };
    b.walk_tree(root, &mut out);
    out
}
