//! Rust equivalents of the dotgen state in `lib/common/types.h`
//! (`Agraphinfo_t`, `Agnodeinfo_t`, `Agedgeinfo_t`, `rank_t`, `port`,
//! `textlabel_t`) plus the small part of cgraph that dot relies on.
//!
//! Nodes, edges and graphs live in arenas owned by [`super::Dot`] and are
//! referred to by index. Real (cgraph) nodes and edges come first and keep the
//! index of the input [`graphviz_pure_core::Graph`] object, so index order equals the
//! cgraph sequence order (`agfstnode`/`agfstout` iteration order). Virtual
//! nodes and edges are appended behind them.

use graphviz_pure_core::Point;

pub type N = usize;
pub type E = usize;
pub type G = usize;
pub type L = usize;

// node_type / edge_type (lib/common/const.h)
pub const NORMAL: u8 = 0;
pub const VIRTUAL: u8 = 1;
pub const SLACKNODE: u8 = 2;
pub const REVERSED: u8 = 3;
pub const FLATORDER: u8 = 4;
pub const CLUSTER_EDGE: u8 = 5;
pub const IGNORED: u8 = 6;

// ranktype / set_type
pub const SAMERANK: u8 = 1;
pub const MINRANK: u8 = 2;
pub const SOURCERANK: u8 = 3;
pub const MAXRANK: u8 = 4;
pub const SINKRANK: u8 = 5;
pub const LEAFSET: u8 = 6;
pub const CLUSTER: u8 = 7;

// clusterrank
pub const LOCAL: i32 = 100;
pub const GLOBAL: i32 = 101;
pub const NOCLUST: i32 = 102;

pub const CL_BACK: i32 = 10;
pub const CL_OFFSET: i32 = 8;
pub const CL_CROSS: i32 = 1000;
pub const MC_SCALE: i32 = 256;
pub const SELF_EDGE_SIZE: f64 = 18.0;

pub const BOTTOM_IX: usize = 0;
pub const RIGHT_IX: usize = 1;
pub const TOP_IX: usize = 2;
pub const LEFT_IX: usize = 3;
pub const BOTTOM: u8 = 1 << BOTTOM_IX;
pub const RIGHT: u8 = 1 << RIGHT_IX;
pub const TOP: u8 = 1 << TOP_IX;
pub const LEFT: u8 = 1 << LEFT_IX;

pub const EDGE_LABEL: u8 = 1 << 0;
pub const HEAD_LABEL: u8 = 1 << 1;
pub const TAIL_LABEL: u8 = 1 << 2;
pub const GRAPH_LABEL: u8 = 1 << 3;
pub const NODE_XLABEL: u8 = 1 << 4;
pub const EDGE_XLABEL: u8 = 1 << 5;

pub const LABEL_AT_BOTTOM: u8 = 0;
pub const LABEL_AT_TOP: u8 = 1;
pub const LABEL_AT_LEFT: u8 = 2;
pub const LABEL_AT_RIGHT: u8 = 4;

pub const NEW_RANK: u16 = 1 << 4;

pub const RANKDIR_TB: i32 = 0;
pub const RANKDIR_LR: i32 = 1;
pub const RANKDIR_BT: i32 = 2;
pub const RANKDIR_RL: i32 = 3;

/// `boxf`.
pub type BoxF = graphviz_pure_core::BoundingBox;

/// `port` (internal edge endpoint specification), shared with `lib/common`.
pub type Port = crate::common::types::PortInfo;

pub use crate::common::ports::port_center;

/// `textlabel_t` (only the fields layout needs).
#[derive(Debug, Clone, Default)]
pub struct TextLabel {
    pub text: String,
    pub fontname: String,
    pub fontsize: f64,
    pub dimen: Point,
    pub space: Point,
    pub pos: Point,
    pub set: bool,
    pub html: bool,
}

/// `layout_t` subset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RatioKind {
    #[default]
    None,
    Value,
    Fill,
    Compress,
    Auto,
    Expand,
}

#[derive(Debug, Clone, Default)]
pub struct Drawing {
    pub ratio_kind: RatioKind,
    pub ratio: f64,
    pub size: Point,
    pub page: Point,
    pub margin: Point,
    pub filled: bool,
}

/// `rank_t`. `v` is a window `(buf, off)` into one of [`super::Dot::bufs`], because
/// in C cluster rank arrays alias windows of the root rank arrays.
#[derive(Debug, Clone, Default)]
pub struct Rank {
    pub n: i32,
    pub vbuf: usize,
    pub voff: usize,
    pub an: i32,
    pub avbuf: usize,
    pub ht1: f64,
    pub ht2: f64,
    pub pht1: f64,
    pub pht2: f64,
    pub candidate: bool,
    pub valid: bool,
    pub cache_nc: i64,
    pub flat: Option<AdjMatrix>,
}

/// `adjmatrix_t` (bit matrix that grows on demand).
#[derive(Debug, Clone, Default)]
pub struct AdjMatrix {
    pub nrows: usize,
    pub ncols: usize,
    pub data: Vec<bool>,
}

impl AdjMatrix {
    pub fn new(rows: usize, cols: usize) -> Self {
        AdjMatrix {
            nrows: rows,
            ncols: cols,
            data: vec![false; rows * cols],
        }
    }
    pub fn get(&self, r: usize, c: usize) -> bool {
        if r >= self.nrows || c >= self.ncols {
            return false;
        }
        self.data[r * self.ncols + c]
    }
    pub fn set(&mut self, r: usize, c: usize) {
        if r >= self.nrows || c >= self.ncols {
            let nrows = self.nrows.max(r + 1);
            let ncols = self.ncols.max(c + 1);
            let mut data = vec![false; nrows * ncols];
            for rr in 0..self.nrows {
                for cc in 0..self.ncols {
                    if self.data[rr * self.ncols + cc] {
                        data[rr * ncols + cc] = true;
                    }
                }
            }
            *self = AdjMatrix { nrows, ncols, data };
        }
        self.data[r * self.ncols + c] = true;
    }
}

/// `Agnodeinfo_t` plus cgraph node identity.
#[derive(Debug, Clone, Default)]
pub struct NodeInfo {
    /// Index of the input node, for real nodes of the main graph.
    pub real: Option<usize>,
    pub name: String,
    /// cgraph root this node belongs to (`None` for virtual nodes).
    pub cg_root: Option<G>,
    /// cgraph out/in edges in the node's root graph, in sequence order.
    pub cg_out: Vec<E>,
    pub cg_in: Vec<E>,

    pub coord: Point,
    pub width: f64,
    pub height: f64,
    pub ht: f64,
    pub lw: f64,
    pub rw: f64,
    pub label: Option<L>,
    pub xlabel: Option<L>,
    pub alg: Option<E>,
    pub state: i32,
    pub id: i32,
    pub hops: i32,
    pub has_port: bool,
    pub rep: Option<N>,
    pub set: Option<N>,
    pub node_type: u8,
    pub mark: usize,
    pub onstack: bool,
    pub ranktype: u8,
    pub weight_class: u8,
    pub next: Option<N>,
    pub prev: Option<N>,
    pub in_: Vec<E>,
    pub out: Vec<E>,
    pub flat_out: Vec<E>,
    pub flat_in: Vec<E>,
    pub other: Vec<E>,
    pub clust: Option<G>,
    pub uf_size: i32,
    pub uf_parent: Option<N>,
    pub rank: i32,
    pub order: i32,
    pub mval: f64,
    pub save_in: Option<Vec<E>>,
    pub save_out: Option<Vec<E>>,
    pub tree_in: Vec<E>,
    pub tree_out: Vec<E>,
    pub par: Option<E>,
    pub low: i32,
    pub lim: i32,
    pub priority: i32,
    /// `ND_subtree` (feasible_tree borrows `ND_par` for it in C).
    pub subtree: Option<usize>,
    /// Input node shape is a record (`shapeOf(n) == SH_RECORD`).
    pub is_record: bool,
    /// `ND_shape(n)` data used by the `insidefn` clipping (real nodes only).
    pub shape: Option<crate::common::shapes::NodeShape>,
    /// Boxes of the top-level record fields (`ND_shape_info(n)->fld[i]->b`),
    /// in drawing orientation relative to the node centre (`record_path`).
    pub rec_fields: Vec<BoxF>,
    /// Not in C: the node `mergevirtual` merged this virtual node into, when
    /// `dot_concentrate` deleted it (see `make_regular_edge`).
    pub conc_merged_into: Option<N>,
}

/// `Agedgeinfo_t` plus cgraph edge identity.
#[derive(Debug, Clone, Default)]
pub struct EdgeInfo {
    pub tail: N,
    pub head: N,
    /// Index of the input edge, for real edges of the main graph.
    pub real: Option<usize>,
    /// `AGSEQ(e)`.
    pub seq: u64,
    pub tail_port: Port,
    pub head_port: Port,
    pub label: Option<L>,
    pub head_label: Option<L>,
    pub tail_label: Option<L>,
    pub xlabel: Option<L>,
    pub edge_type: u8,
    pub adjacent: bool,
    pub label_ontop: bool,
    pub to_orig: Option<E>,
    pub to_virt: Option<E>,
    pub dist: f64,
    pub conc_opp_flag: bool,
    pub xpenalty: i32,
    pub weight: i32,
    pub cutvalue: i32,
    pub tree_index: i32,
    pub count: i32,
    pub minlen: i32,
    /// `ED_spl(e)`.
    pub spl: Option<Vec<Bezier>>,
    /// `ED_alg(e)`: the edge's clone in the auxiliary graph of
    /// `make_flat_adj_edges` (an index into that graph's edges).
    pub alg: Option<usize>,
}

/// `Agraphinfo_t` plus cgraph graph identity and membership.
#[derive(Debug, Clone, Default)]
pub struct GraphInfo {
    pub name: String,
    /// cgraph root of this graph.
    pub root: G,
    /// The root's next anonymous id (`idmap`'s `counter`), shared by its subgraphs.
    pub next_anon_id: std::rc::Rc<std::cell::Cell<u64>>,
    /// cgraph parent (`agparent`).
    pub cg_parent: Option<G>,
    /// cgraph child subgraphs in `agfstsubg` order.
    pub subgs: Vec<G>,
    /// cgraph node membership (sorted = sequence order).
    pub nodes: std::collections::BTreeSet<N>,
    /// cgraph edge membership.
    pub edges: std::collections::BTreeSet<E>,
    /// Graph attributes as seen by `agget(g, ...)`.
    pub attrs: graphviz_pure_core::AttrMap,
    /// Index in `Graph::subgraphs` for input subgraphs.
    pub sub_index: Option<usize>,
    /// Index in `Graph::subgraphs` of the subgraph a `cccomps` projection
    /// (`projectG`) copies.
    pub proj_of: Option<usize>,

    pub drawing: Drawing,
    pub label: Option<L>,
    pub bb: BoxF,
    pub border: [Point; 4],
    pub has_labels: u8,
    pub rankdir: i32,
    pub ht1: f64,
    pub ht2: f64,
    pub flags: u16,
    /// `GD_clust(g)[1..=n_cluster]` stored 0-based.
    pub clust: Vec<G>,
    pub nlist: Option<N>,
    /// `GD_rank(g)[r]` stored at index `r + 1` (rank -1 exists after `abomination`).
    pub rank: Vec<Rank>,
    pub parent: Option<G>,
    pub level: i32,
    pub minrep: Option<N>,
    pub maxrep: Option<N>,
    pub comp: Vec<Option<N>>,
    pub minset: Option<N>,
    pub maxset: Option<N>,
    pub minrank: i32,
    pub maxrank: i32,
    pub has_flat_edges: bool,
    pub nodesep: i32,
    pub ranksep: i32,
    pub ln: Option<N>,
    pub rn: Option<N>,
    pub leader: Option<N>,
    /// `GD_rankleader(g)[r]`.
    pub rankleader: Vec<Option<N>>,
    pub expanded: bool,
    pub installed: i32,
    pub set_type: u8,
    pub label_pos: u8,
    pub exact_ranksep: bool,
    /// The graph has Agraphinfo_t bound (dot_init_subg / is a cluster).
    pub bound: bool,
}

impl GraphInfo {
    pub fn n_cluster(&self) -> i32 {
        self.clust.len() as i32
    }
    /// `GD_flip(g)`
    pub fn flip(&self) -> bool {
        (self.rankdir & 3) & 1 != 0
    }
}

/// `bezier` (one piece of an edge spline), shared with `lib/common`.
pub use crate::common::types::Bezier;
