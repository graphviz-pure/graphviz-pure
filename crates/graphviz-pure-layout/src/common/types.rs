//! Layout working state mirroring the C `Agraphinfo_t` / `Agnodeinfo_t` /
//! `Agedgeinfo_t` records (`lib/common/types.h`) for the non-dot engines.
//!
//! [`LGraph`] wraps a parsed [`Graph`] and holds per-node and per-edge layout
//! fields. Subgraphs (connected components, clusters) are [`Sub`] views over
//! node indices. Iteration helpers reproduce cgraph orders: nodes by creation
//! sequence; `agfstout`/`agnxtout` by (head, edge) sequence (`agedgeseqcmpf`); `agfstedge`/`agnxtedge`
//! as out-edges followed by non-loop in-edges.

use super::shapes::NodeShape;
use graphviz_pure_core::{BoundingBox, Graph, Point};

/// `P_SET`, `P_FIX`, `P_PIN` (`ND_pinned`).
pub const P_SET: u8 = 1;
pub const P_FIX: u8 = 2;
pub const P_PIN: u8 = 3;

/// `MAXDIM` in `neato.h`.
pub const MAXDIM: usize = 10;

/// Edge routing types (`EDGETYPE_*` in `const.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeType {
    None = 0,
    Line = 2,
    Curved = 4,
    Pline = 6,
    Ortho = 8,
    Spline = 10,
    CompoundEdge = 12,
}

/// `textlabel_t`.
#[derive(Debug, Clone, Default)]
pub struct TextLabel {
    pub text: String,
    /// Size in points.
    pub dimen: Point,
    pub pos: Point,
    pub set: bool,
    pub fontsize: f64,
    pub fontname: String,
}

/// `port` (internal edge endpoint specification), shared by all engines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortInfo {
    /// Aiming point relative to the node centre, in points (dot: in
    /// rank-direction layout space).
    pub p: Point,
    /// Slope of the edge at the port (`theta`), used when `constrained`.
    pub theta: f64,
    pub defined: bool,
    pub constrained: bool,
    pub clip: bool,
    /// `dyna`: the side is resolved per edge (`resolvePort`).
    pub dyna: bool,
    /// `order`: port position used by mincross (`MC_SCALE` units).
    pub order: u8,
    /// Box of the port's record field or HTML cell, in node coordinates.
    pub bp: Option<BoundingBox>,
    /// `BOTTOM`/`RIGHT`/`TOP`/`LEFT` bit mask.
    pub side: u8,
    /// `name`: the port name, if it was explicitly given.
    pub name: Option<String>,
}

/// `bezier`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bezier {
    pub list: Vec<Point>,
    pub sflag: u32,
    pub eflag: u32,
    pub sp: Point,
    pub ep: Point,
}

/// `Agnodeinfo_t` fields used by neato and friends.
#[derive(Debug, Clone)]
pub struct LNode {
    /// `ND_pos`, in inches, `ndim` components.
    pub pos: Vec<f64>,
    /// `ND_coord`, in points.
    pub coord: Point,
    /// `ND_width`/`ND_height` in inches.
    pub width: f64,
    pub height: f64,
    /// `ND_lw`, `ND_rw`, `ND_ht` in points.
    pub lw: f64,
    pub rw: f64,
    pub ht: f64,
    pub pinned: u8,
    pub id: usize,
    pub heapindex: i64,
    pub hops: i64,
    pub dist: f64,
    pub mark: bool,
    pub shape: NodeShape,
    pub xlabel: Option<TextLabel>,
    /// Index into [`LGraph::clusters`] of the top-level cluster holding the node.
    pub clust: Option<usize>,
    /// `ND_lim`, used by `spline_edges_` for obstacle ids.
    pub lim: i64,
}

/// `Agedgeinfo_t` fields used by neato and friends.
#[derive(Debug, Clone)]
pub struct LEdge {
    pub tail: usize,
    pub head: usize,
    /// `ED_dist` (ideal length, inches).
    pub dist: f64,
    /// `ED_factor` (weight).
    pub factor: f64,
    pub count: i64,
    pub to_virt: Option<usize>,
    pub spl: Option<Vec<Bezier>>,
    pub label: Option<TextLabel>,
    pub xlabel: Option<TextLabel>,
    pub head_label: Option<TextLabel>,
    pub tail_label: Option<TextLabel>,
    pub tail_port: PortInfo,
    pub head_port: PortInfo,
    /// `ED_path`, from the visibility-graph shortest path.
    pub path: Vec<Point>,
}

/// A cluster registered with `add_cluster` (`GD_clust`).
#[derive(Debug, Clone)]
pub struct LCluster {
    /// Index into `Graph::subgraphs`.
    pub subgraph: usize,
    pub bb: BoundingBox,
    pub label: Option<TextLabel>,
    /// `GD_label_pos` flags.
    pub label_pos: u8,
    /// `GD_border` as `[bottom, right, top, left]`.
    pub border: [Point; 4],
    pub children: Vec<usize>,
}

/// `ratio_kind` in `layout_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RatioKind {
    #[default]
    None,
    Value,
    Fill,
    Compress,
    Auto,
    Expand,
}

/// The parts of `layout_t` (`GD_drawing`) layouts consult.
#[derive(Debug, Clone, Default)]
pub struct Drawing {
    pub ratio_kind: RatioKind,
    pub ratio: f64,
    /// `size` attribute in points; zero when unset.
    pub size: Point,
    pub filled: bool,
    pub landscape: bool,
}

pub const LABEL_AT_BOTTOM: u8 = 0;
pub const LABEL_AT_TOP: u8 = 1;
pub const LABEL_AT_LEFT: u8 = 2;
pub const LABEL_AT_RIGHT: u8 = 4;

/// A node-induced subgraph view (component, cluster or the root).
#[derive(Debug, Clone)]
pub struct Sub {
    /// Member nodes in sequence (creation) order.
    pub nodes: Vec<usize>,
    pub member: Vec<bool>,
    pub bb: BoundingBox,
    /// Top-level clusters (indices into [`LGraph::clusters`]).
    pub clusters: Vec<usize>,
    /// `agnameof` of the subgraph C lays out (`_cc_0`, `_neato_cc1`, the root's name), which
    /// the verbose messages print.
    pub name: String,
}

impl Sub {
    pub fn new(n_total: usize, mut nodes: Vec<usize>) -> Self {
        nodes.sort_unstable();
        let mut member = vec![false; n_total];
        for &n in &nodes {
            member[n] = true;
        }
        Self {
            nodes,
            member,
            bb: BoundingBox::default(),
            clusters: Vec::new(),
            name: String::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Layout state for one root graph.
pub struct LGraph<'a> {
    pub src: &'a Graph,
    pub nodes: Vec<LNode>,
    pub edges: Vec<LEdge>,
    /// Out-edges of each node ordered by (head, edge) sequence.
    pub out: Vec<Vec<usize>>,
    /// In-edges of each node ordered by (tail, edge) sequence.
    pub inn: Vec<Vec<usize>>,
    pub ndim: usize,
    /// `GD_odim(root)`: the output dimension, which only neato and sfdp set (`dimen`).
    pub odim: usize,
    /// `GD_nodesep(root)` in points.
    pub nodesep: i64,
    pub bb: BoundingBox,
    pub label: Option<TextLabel>,
    pub label_pos: u8,
    pub drawing: Drawing,
    pub clusters: Vec<LCluster>,
    /// Top-level clusters of the root (`GD_clust(root)`).
    pub root_clusters: Vec<usize>,
    pub edge_type: EdgeType,
    pub concentrate: bool,
    /// Whether any edge sets `labelangle` / `labeldistance` (`E_labelangle`, `E_labeldistance`).
    pub has_labelangle: bool,
    pub has_labeldistance: bool,
}

impl<'a> LGraph<'a> {
    /// `agnameof(n)` of node `v` of the source graph.
    pub fn node_agnameof(&self, v: usize) -> graphviz_pure_core::AgName<'a> {
        if v < self.src.nodes.len() {
            let id = graphviz_pure_core::NodeId(v as u32);
            self.src.agnameof(graphviz_pure_core::Obj::Node(id))
        } else {
            graphviz_pure_core::AgName::own("")
        }
    }

    /// The root as a subgraph view.
    pub fn root_sub(&self) -> Sub {
        let mut s = Sub::new(self.nodes.len(), (0..self.nodes.len()).collect());
        s.bb = self.bb;
        s.clusters = self.root_clusters.clone();
        s.name = self.src.root_name().to_string();
        s
    }

    /// `agfstout`..`agnxtout` within `sub`.
    pub fn outs<'s>(&'s self, sub: &'s Sub, n: usize) -> impl Iterator<Item = usize> + 's {
        self.out[n]
            .iter()
            .copied()
            .filter(move |&e| sub.member[self.edges[e].head])
    }

    /// `agfstin`..`agnxtin` within `sub`.
    pub fn ins<'s>(&'s self, sub: &'s Sub, n: usize) -> impl Iterator<Item = usize> + 's {
        self.inn[n]
            .iter()
            .copied()
            .filter(move |&e| sub.member[self.edges[e].tail])
    }

    /// `agfstedge`..`agnxtedge` within `sub`: out-edges, then in-edges that are not loops.
    pub fn all_edges<'s>(&'s self, sub: &'s Sub, n: usize) -> impl Iterator<Item = usize> + 's {
        self.outs(sub, n).chain(
            self.ins(sub, n)
                .filter(move |&e| self.edges[e].tail != self.edges[e].head),
        )
    }

    /// Number of edges with both ends in `sub` (`agnedges`).
    pub fn nedges(&self, sub: &Sub) -> usize {
        sub.nodes.iter().map(|&n| self.outs(sub, n).count()).sum()
    }

    /// `agfindedge(g, t, h)`: an edge `t -> h`, or `h -> t` in undirected graphs.
    pub fn find_edge(&self, sub: &Sub, t: usize, h: usize) -> Option<usize> {
        let fwd = self.outs(sub, t).find(|&e| self.edges[e].head == h);
        if fwd.is_some() || self.src.kind == graphviz_pure_core::GraphKind::Directed {
            return fwd;
        }
        self.outs(sub, h).find(|&e| self.edges[e].head == t)
    }

    /// Node attribute value (`agget(n, key)`).
    pub fn nattr(&self, n: usize, key: &str) -> Option<&'a str> {
        self.src.nodes[n].attrs.get(key).map(String::as_str)
    }

    /// Edge attribute value (`agget(e, key)`).
    pub fn eattr(&self, e: usize, key: &str) -> Option<&'a str> {
        self.src.edges[e].attrs.get(key).map(String::as_str)
    }

    /// Root graph attribute value (`agget(g, key)`).
    pub fn gattr(&self, key: &str) -> Option<&'a str> {
        self.src.graph_attrs.get(key).map(String::as_str)
    }

    /// Whether edges declare `key` (`agattr(g, AGEDGE, key, 0) != NULL`).
    pub fn edge_attr_declared(&self, key: &str) -> bool {
        self.src.edge_attr_declared(key)
    }

    /// Whether nodes declare `key` (`agfindnodeattr`).
    pub fn node_attr_declared(&self, key: &str) -> bool {
        self.src.node_attr_declared(key)
    }

    pub fn is_directed(&self) -> bool {
        self.src.kind == graphviz_pure_core::GraphKind::Directed
    }
}
