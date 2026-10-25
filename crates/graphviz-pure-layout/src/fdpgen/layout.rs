//! The fdp driver, ported from `lib/fdpgen/layout.c`: `fdp_layout`,
//! `fdpLayout`, `layout`, `deriveGraph`, `getEdgeList`, `genPorts`,
//! `expandCluster`, `finalCC`, `evalPositions`, `setBB`, `chkPos`,
//! `fdpSplines`.
//!
//! Each graph (the root or a cluster) is laid out through a *derived graph*
//! whose nodes are its own nodes, one node per child cluster and one node per
//! boundary port inherited from the parent. Derived graphs are kept as
//! [`DG`]; whenever shared neato code (overlap removal, packing, bounding
//! boxes) has to run on one, [`with_lgraph`] builds a matching [`LGraph`].
//!
//! cgraph orders derived nodes and edges by address; the port uses creation
//! order, which is what consecutive allocations give in practice.

use super::clusteredges::{COMPOUND_EDGE_TYPE, Hier, compound_edges};
use super::comp::find_ccomp;
use super::fdpinit::{get_inputscale, init_edges, initial_positions};
use super::tlayout::{TState, fdp_init_params, fdp_t_layout};
use super::xlayout::{XParams, fdp_x_layout};
use super::{ClusterTree, mk_clusters, subgraph_member_nodes};
use crate::common::init::do_graph_label;
use crate::common::postproc::gv_postprocess;
use crate::common::shapes::NodeShape;
use crate::common::types::*;
use crate::common::utils::{compute_bb, late_int};
use crate::cutil::qsort_by;
use crate::neatogen::adjust::normalize;
use crate::neatogen::neatosplines::{
    neato_set_aspect, spline_edges_by, spline_edges_main, spline_edges_route,
};
use crate::osage::register_clusters;
use crate::pack::{PackInfo, PackMode, put_graphs};
use graphviz_pure_core::math;
use graphviz_pure_core::{AttrMap, BoundingBox, Graph, GraphKind, NodeId, Obj, Point, Result};
use std::collections::HashMap;
use std::f64::consts::PI;

/// `CL_OFFSET`.
const CL_OFFSET: i64 = 8;
/// Maximum angular change between ports: 2 degrees.
const ANG: f64 = PI / 90.0;

/// What a derived node stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DKind {
    /// A child cluster (index into the [`ClusterTree`]).
    Clust(usize),
    /// A real node of the input graph.
    Real(usize),
    /// A boundary port.
    Port,
}

/// A node of a derived graph (`Agnodeinfo_t` + `dndata`).
#[derive(Debug, Clone)]
pub struct DNode {
    pub kind: DKind,
    /// Model of the node's heap address (see [`NodeHeap`]).
    pub addr: u64,
    /// Name, for `GVDBG` traces.
    pub name: String,
    /// `ND_pos`, inches.
    pub pos: [f64; 2],
    pub width: f64,
    pub height: f64,
    pub lw: f64,
    pub rw: f64,
    pub ht: f64,
    pub pinned: u8,
    pub deg: i32,
    pub wdeg: i32,
    pub disp: [f64; 2],
    /// `ND_shape(dn) = ND_shape(n)`, `ND_shape_info(dn) = ND_shape_info(n)` of a real node.
    pub shape: Option<NodeShape>,
}

impl DNode {
    fn new(kind: DKind) -> Self {
        Self {
            kind,
            addr: 0,
            name: String::new(),
            pos: [0.0, 0.0],
            width: 0.0,
            height: 0.0,
            lw: 0.0,
            rw: 0.0,
            ht: 0.0,
            pinned: 0,
            deg: 0,
            wdeg: 0,
            disp: [0.0, 0.0],
            shape: None,
        }
    }

    /// `IS_PORT(n)`.
    pub fn is_port(&self) -> bool {
        self.kind == DKind::Port
    }
}

/// A derived edge; `reals` is the list kept in `ED_to_virt`/`ED_count`.
#[derive(Debug, Clone)]
pub struct DEdge {
    pub t: usize,
    pub h: usize,
    pub dist: f64,
    pub factor: f64,
    pub reals: Vec<usize>,
}

/// `bport_t` in a derived graph.
#[derive(Debug, Clone, Copy)]
pub struct DPort {
    pub n: usize,
    pub alpha: f64,
    pub e: usize,
}

/// `bport_t` attached to a cluster: a real edge and its real endpoint inside.
#[derive(Debug, Clone, Copy)]
pub struct BPort {
    pub e: usize,
    pub n: usize,
    pub alpha: f64,
}

/// Model of the allocator slots that hold `Agnode_t`s.
///
/// cgraph orients derived edges and orders ports by node *address*. macOS
/// serves these fixed-size blocks from a size class with a LIFO free list:
/// a derived graph allocated after another was freed gets the freed slots
/// back in reverse order, then fresh, increasing addresses.
#[derive(Debug, Clone, Default)]
pub struct NodeHeap {
    free: Vec<u64>,
    next: u64,
    /// Whether freed slots are reused.
    pub recycle: bool,
}

impl NodeHeap {
    const FRESH: u64 = 1 << 40;

    /// `agalloc` of a node.
    pub fn alloc(&mut self) -> u64 {
        self.free.pop().unwrap_or_else(|| {
            self.next += 1;
            Self::FRESH + self.next
        })
    }

    /// `agfree` of a node.
    pub fn free(&mut self, addr: u64) {
        if self.recycle {
            self.free.push(addr);
        }
    }
}

/// A derived graph (strict, directed; edges run from the lower address).
#[derive(Debug, Clone, Default)]
pub struct DG {
    pub uid: u32,
    pub nodes: Vec<DNode>,
    pub edges: Vec<DEdge>,
    pub out: Vec<Vec<usize>>,
    pub inn: Vec<Vec<usize>>,
    pub ports: Vec<DPort>,
    /// `PORTS(dg) != NULL`: the graph had ports, even if none hooks to a
    /// derived node.
    pub has_ports: bool,
    /// The command-line graph attributes, and `overlap`, `sep` and `K` copied from the
    /// original graph (`copyAttr`).
    pub attrs: AttrMap,
    /// `GD_ndim(dg) = GD_ndim(agroot(g))` (`deriveGraph`, layout.c).
    pub ndim: usize,
    emap: HashMap<(usize, usize), usize>,
}

/// A component of a derived graph (node-induced subgraph).
#[derive(Debug, Clone)]
pub struct Comp {
    pub member: Vec<bool>,
    pub nodes: Vec<usize>,
    /// The component carries the derived graph's ports (`PORTS(cg)`).
    pub has_ports: bool,
    /// `agnameof(cg)`: `cc<dg>_<i>` or `cc<dg>+<i>` (`findCComp`).
    pub name: String,
}

impl Comp {
    pub fn new(n_total: usize, mut nodes: Vec<usize>, has_ports: bool) -> Self {
        nodes.sort_unstable();
        let mut member = vec![false; n_total];
        for &n in &nodes {
            member[n] = true;
        }
        Self {
            member,
            nodes,
            has_ports,
            name: String::new(),
        }
    }

    /// `agdelete(cg, n)`.
    pub fn remove(&mut self, n: usize) {
        self.member[n] = false;
        self.nodes.retain(|&m| m != n);
    }
}

impl DG {
    fn add_node(&mut self, n: DNode) -> usize {
        self.nodes.push(n);
        self.out.push(Vec::new());
        self.inn.push(Vec::new());
        self.nodes.len() - 1
    }

    /// `agedge(dg, t, h, NULL, 1)` on a strict graph; returns the edge and
    /// whether it is new.
    fn edge(&mut self, t: usize, h: usize) -> (usize, bool) {
        if let Some(&e) = self.emap.get(&(t, h)) {
            return (e, false);
        }
        let e = self.edges.len();
        self.edges.push(DEdge {
            t,
            h,
            dist: 1.0,
            factor: 1.0,
            reals: Vec::new(),
        });
        self.out[t].push(e);
        self.inn[h].push(e);
        self.emap.insert((t, h), e);
        (e, true)
    }

    /// Orders adjacency like cgraph (`agedgeseqcmpf`: other end, then edge).
    fn sort_adjacency(&mut self) {
        let edges = &self.edges;
        for l in &mut self.out {
            l.sort_by_key(|&e| (edges[e].h, e));
        }
        for l in &mut self.inn {
            l.sort_by_key(|&e| (edges[e].t, e));
        }
    }

    /// `agfstout(cg, n)`..`agnxtout`.
    pub fn outs<'s>(&'s self, c: &'s Comp, n: usize) -> impl Iterator<Item = usize> + 's {
        self.out[n]
            .iter()
            .copied()
            .filter(move |&e| c.member[self.edges[e].h])
    }

    /// `agfstedge(cg, n)`..`agnxtedge`: out-edges, then in-edges.
    pub fn all_edges(&self, c: &Comp, n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = self.outs(c, n).collect();
        v.extend(
            self.inn[n]
                .iter()
                .copied()
                .filter(|&e| c.member[self.edges[e].t] && self.edges[e].t != n),
        );
        v
    }
}

/// Runs `f` on an [`LGraph`] holding the derived nodes `keep` (in order) and
/// the derived edges among them, then copies node positions back. The third
/// argument maps derived node indices to [`LGraph`] indices.
pub fn with_lgraph<R>(
    dg: &mut DG,
    keep: &[usize],
    f: impl FnOnce(&mut LGraph<'_>, &mut Sub, &[usize]) -> R,
) -> R {
    let mut src = Graph::new("derived", GraphKind::Directed, true);
    src.graph_attrs = dg.attrs.clone();
    let mut map = vec![usize::MAX; dg.nodes.len()];
    for (i, &n) in keep.iter().enumerate() {
        map[n] = i;
        src.ensure_node(&format!("{i}"));
    }
    let mut edges = Vec::new();
    for (ei, e) in dg.edges.iter().enumerate() {
        if map[e.t] != usize::MAX && map[e.h] != usize::MAX {
            src.add_edge(
                graphviz_pure_core::NodeId(map[e.t] as u32),
                graphviz_pure_core::NodeId(map[e.h] as u32),
                AttrMap::new(),
            );
            edges.push(ei);
        }
    }
    let nodes: Vec<LNode> = keep
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let d = &dg.nodes[n];
            // mkDeriveNode allocates GD_ndim(dg) coordinates; fdp's t_layout,
            // chkPos and copy-back only ever write the first two, so the rest
            // stay zero (layout.c:187). They still matter: fdpAdjust hands the
            // full-width array to remove_overlap, where call_tri reads it with
            // a stride of 2.
            let mut pos = vec![d.pos[0], d.pos[1]];
            pos.resize(dg.ndim.max(2), 0.0);
            LNode {
                pos,
                coord: Point::default(),
                width: d.width,
                height: d.height,
                lw: d.lw,
                rw: d.rw,
                ht: d.ht,
                pinned: d.pinned,
                id: 0,
                heapindex: -1,
                hops: 0,
                dist: 0.0,
                mark: false,
                shape: d
                    .shape
                    .clone()
                    .unwrap_or_else(|| NodeShape::new(&src.nodes[i])),
                xlabel: None,
                // ND_clust(dn) = subg for a cluster's derived node
                clust: match d.kind {
                    DKind::Clust(c) => Some(c),
                    _ => None,
                },
                lim: -1,
            }
        })
        .collect();
    let mut out = vec![Vec::new(); keep.len()];
    let mut inn = vec![Vec::new(); keep.len()];
    let ledges: Vec<LEdge> = edges
        .iter()
        .enumerate()
        .map(|(i, &ei)| {
            let e = &dg.edges[ei];
            let (t, h) = (map[e.t], map[e.h]);
            out[t].push(i);
            inn[h].push(i);
            LEdge {
                tail: t,
                head: h,
                dist: e.dist,
                factor: e.factor,
                count: 0,
                to_virt: None,
                spl: None,
                label: None,
                xlabel: None,
                head_label: None,
                tail_label: None,
                tail_port: PortInfo::default(),
                head_port: PortInfo::default(),
                path: Vec::new(),
            }
        })
        .collect();
    for l in out.iter_mut() {
        l.sort_by_key(|&e| (ledges[e].head, e));
    }
    for l in inn.iter_mut() {
        l.sort_by_key(|&e| (ledges[e].tail, e));
    }
    let mut lg = LGraph {
        src: &src,
        nodes,
        edges: ledges,
        out,
        inn,
        ndim: dg.ndim.max(2),
        odim: 0,
        nodesep: 18,
        bb: BoundingBox::default(),
        label: None,
        label_pos: 0,
        drawing: Drawing::default(),
        clusters: Vec::new(),
        root_clusters: Vec::new(),
        edge_type: EdgeType::Line,
        concentrate: false,
        has_labelangle: false,
        has_labeldistance: false,
    };
    let mut sub = lg.root_sub();
    let r = f(&mut lg, &mut sub, &map);
    for (i, &n) in keep.iter().enumerate() {
        dg.nodes[n].pos = [lg.nodes[i].pos[0], lg.nodes[i].pos[1]];
    }
    r
}

/// `erec`.
#[derive(Debug, Clone, Copy)]
struct ERec {
    e: usize,
    alpha: f64,
    dist2: f64,
}

struct Fdp<'g, 'a> {
    rg: &'g mut LGraph<'a>,
    tree: ClusterTree,
    st: TState,
    /// `PORTS(sg)` of each cluster.
    ports: Vec<Vec<BPort>>,
    /// `BB(g)` (inches) of each cluster and of the root.
    gbb: Vec<BoundingBox>,
    root_bb: BoundingBox,
    members: Vec<Vec<usize>>,
    /// Edge membership of each cluster subgraph.
    cedges: Vec<Vec<bool>>,
    /// `PARENT(n)`: smallest containing graph (`Some(None)` = root).
    parent: Vec<Option<Option<usize>>>,
    /// `DNODE(n)`: derived graph id and node.
    dnode: Vec<Option<(u32, usize)>>,
    next_uid: u32,
    /// findCComp's `counter`: the components named so far.
    cc_counter: usize,
    pack: PackInfo,
    coords_declared: bool,
    inputscale: f64,
    /// `IS_CLUST_NODE(n)`.
    is_clust: Vec<bool>,
    /// Clusters `deriveGraph` reached, which `do_graph_label` gave their label.
    labeled: Vec<bool>,
    heap: NodeHeap,
}

impl Fdp<'_, '_> {
    fn gattrs(&self, g: Option<usize>) -> &AttrMap {
        match g {
            None => &self.rg.src.graph_attrs,
            Some(c) => &self.rg.src.subgraphs[self.tree.subgraph[c]].graph_attrs,
        }
    }

    fn members_of(&self, g: Option<usize>) -> Vec<usize> {
        match g {
            None => (0..self.rg.nodes.len()).collect(),
            Some(c) => self.members[c].clone(),
        }
    }

    /// `GPARENT(g)` as a `PARENT` value (`None` = NULL for the root).
    fn gparent(&self, g: Option<usize>) -> Option<Option<usize>> {
        g.map(|c| self.tree.parent[c])
    }

    fn bb_of(&self, g: Option<usize>) -> BoundingBox {
        match g {
            None => self.root_bb,
            Some(c) => self.gbb[c],
        }
    }

    fn set_bb(&mut self, g: Option<usize>, bb: BoundingBox) {
        match g {
            None => self.root_bb = bb,
            Some(c) => self.gbb[c] = bb,
        }
    }

    /// `chkPos(g, n, infop, bbp)`: a cluster's `coords` attribute.
    fn chk_pos(&self, c: usize, dn: &mut DNode, bbp: &mut BoundingBox) {
        let attrs = self.gattrs(Some(c));
        let Some(p) = attrs.get("coords").filter(|s| !s.is_empty()) else {
            return;
        };
        let parent = self.gattrs(self.tree.parent[c]);
        if parent.get("coords").map(String::as_str).unwrap_or("") == p.as_str() {
            return;
        }
        let r = graphviz_pure_core::strtod::sscanf(p, "%lf,%lf,%lf,%lf%c");
        if r.count < 4 {
            let gname = self.rg.src.subgraph_name(self.tree.subgraph[c]);
            graphviz_pure_core::diag::agwarningf(&format!(
                "graph {gname}, coord {p}, expected four doubles\n"
            ));
            return;
        }
        let [llx, lly, urx, ury, _] = r.values;
        let mut bb = BoundingBox::new(llx, lly, urx, ury);
        if self.inputscale > 0.0 {
            let s = self.inputscale;
            bb = BoundingBox::new(bb.ll.x / s, bb.ll.y / s, bb.ur.x / s, bb.ur.y / s);
        }
        dn.pinned = match r.c {
            Some(b'!') => P_PIN,
            Some(b'?') => P_FIX,
            _ => P_SET,
        };
        *bbp = bb;
    }

    /// `deriveGraph(g, infop)`; `None` when a node lies in two
    /// non-comparable clusters.
    ///
    /// C names each derived node (`mkDeriveNode(dg, agnameof(...))`) in a root graph of its
    /// own, whose dictionary has no HTML strings: a node `<G>` and a node `G`, two clusters of
    /// one name, or a cluster and a node of one name are one derived node there. Each call
    /// still takes the next `ND_id`, so the merged node's id reaches `agnnodes(dg)`, and C
    /// fails `bitarray_get`'s bounds assertion in `findCComp` (`comp.c`). C produces no layout
    /// for such a graph; the port keeps the derived nodes apart, as C does when the names
    /// differ.
    fn derive_graph(&mut self, g: Option<usize>) -> Option<DG> {
        if graphviz_pure_core::diag::verbose_byte() >= 2 {
            graphviz_pure_core::diag::fprintf_stderr(&format!(
                "derive graph _dg_{} of {}\n",
                self.next_uid,
                self.gname(g)
            ));
        }
        let mut dg = DG {
            uid: self.next_uid,
            ndim: self.rg.ndim,
            ..DG::default()
        };
        self.next_uid += 1;
        let uid = dg.uid;
        let src = self.rg.src;
        // agopen("derived") declares the command-line graph attributes; copyAttr then sets
        // `overlap`, `sep` and `K` from g.
        dg.attrs = src.protograph_attrs();
        for key in ["overlap", "sep", "K"] {
            let declared = src.graph_attr_declared(key);
            if declared {
                let v = self.gattrs(g).get(key).cloned().unwrap_or_default();
                dg.attrs.insert(key.to_string(), v);
            }
        }

        for c in self.tree.kids(g).to_vec() {
            self.labeled[c] = true;
            // do_graph_label(subg): a graph's clusters get their label here, before their
            // own clusters, which their layout reaches later. The order interns the labels'
            // fonts (a NaN fontsize matches whichever entry it is compared with first) and
            // prints their diagnostics.
            do_graph_label(self.rg, c);
            let m = f64::MAX;
            let mut fix_bb = BoundingBox::new(m, m, -m, -m);
            let mut dn = DNode::new(DKind::Clust(c));
            dn.addr = self.heap.alloc();
            dn.name
                .clone_from(&src.subgraphs[self.tree.subgraph[c]].name);
            if self.coords_declared {
                self.chk_pos(c, &mut dn, &mut fix_bb);
            }
            if dn.pinned != 0 {
                dn.pos = [
                    (fix_bb.ll.x + fix_bb.ur.x) / 2.0,
                    (fix_bb.ll.y + fix_bb.ur.y) / 2.0,
                ];
            }
            let id = dg.add_node(dn);
            for &n in &self.members[c] {
                self.dnode[n] = Some((uid, id));
            }
        }

        let members = self.members_of(g);
        for &n in &members {
            if self.dnode[n].is_some() {
                continue;
            }
            if self.parent[n].is_some() && self.parent[n] != self.gparent(g) {
                let src = self.rg.src;
                let gname = |g: Option<usize>| match g {
                    None => src.agnameof(Obj::Root),
                    Some(c) => src.agnameof(Obj::Subgraph(self.tree.subgraph[c])),
                };
                // layout.c:432: the three names share agnameof's static buffer when
                // anonymous, so they all show the last anonymous one.
                let [node, g, parent] = graphviz_pure_core::agnameof_buffer([
                    src.agnameof(Obj::Node(NodeId(n as u32))),
                    gname(g),
                    gname(self.parent[n].unwrap()),
                ]);
                graphviz_pure_core::diag::agerrorf(&format!(
                    "node \"{node}\" is contained in two non-comparable clusters \"{g}\" and \"{parent}\"\n"
                ));
                return None;
            }
            self.parent[n] = Some(g);
            if self.is_clust[n] {
                continue;
            }
            let rn = &self.rg.nodes[n];
            let mut dn = DNode::new(DKind::Real(n));
            dn.addr = self.heap.alloc();
            dn.name.clone_from(&src.nodes[n].name);
            dn.width = rn.width;
            dn.height = rn.height;
            dn.lw = rn.lw;
            dn.rw = rn.rw;
            dn.ht = rn.ht;
            dn.shape = Some(rn.shape.clone());
            if rn.pinned != 0 {
                dn.pos = [rn.pos[0], rn.pos[1]];
                dn.pinned = rn.pinned;
            }
            let id = dg.add_node(dn);
            self.dnode[n] = Some((uid, id));
        }

        let root = self.rg.root_sub();
        for &n in &members {
            // Cluster nodes inside their own cluster have no derived node
            // (and no edges there: cluster-edge clones live in the root).
            let Some((_, tl)) = self.dnode[n] else {
                continue;
            };
            let outs: Vec<usize> = self.rg.outs(&root, n).collect();
            for e in outs {
                if let Some(c) = g
                    && !self.cedges[c][e]
                {
                    continue;
                }
                let Some((_, hd)) = self.dnode[self.rg.edges[e].head] else {
                    continue;
                };
                if hd == tl {
                    continue;
                }
                let (de, new) = if dg.nodes[hd].addr > dg.nodes[tl].addr {
                    dg.edge(tl, hd)
                } else {
                    dg.edge(hd, tl)
                };
                dg.edges[de].dist = self.rg.edges[e].dist;
                dg.edges[de].factor = self.rg.edges[e].factor;
                dg.nodes[hd].wdeg += 1;
                dg.nodes[tl].wdeg += 1;
                if new {
                    dg.nodes[hd].deg += 1;
                    dg.nodes[tl].deg += 1;
                }
                dg.edges[de].reals.push(e);
            }
        }

        if let Some(c) = g {
            dg.has_ports = !self.ports[c].is_empty();
            for pp in self.ports[c].clone() {
                let Some((u, m)) = self.dnode[pp.n] else {
                    continue;
                };
                if u != uid {
                    continue;
                }
                let mut pn = DNode::new(DKind::Port);
                pn.addr = self.heap.alloc();
                // portName: the cluster, ND_id of the edge's ends (their index) and AGSEQ(e)
                let (t, h) = self
                    .rg
                    .edges
                    .get(pp.e)
                    .map_or((0, 0), |re| (re.tail, re.head));
                pn.name = format!(
                    "_port_{}_({t})_({h})_{}",
                    self.gname(Some(c)),
                    pp.e.wrapping_add(1)
                );
                let dn = dg.add_node(pn);
                let (de, _) = if dg.nodes[dn].addr > dg.nodes[m].addr {
                    dg.edge(m, dn)
                } else {
                    dg.edge(dn, m)
                };
                dg.edges[de].dist = self.rg.edges[pp.e].dist;
                dg.edges[de].factor = self.rg.edges[pp.e].factor;
                dg.edges[de].reals.push(pp.e);
                dg.nodes[dn].wdeg += 1;
                dg.nodes[m].wdeg += 1;
                dg.nodes[dn].deg += 1;
                dg.nodes[m].deg += 1;
                dg.ports.push(DPort {
                    n: dn,
                    alpha: pp.alpha,
                    e: de,
                });
            }
        }
        dg.sort_adjacency();
        if super::trace(3) {
            let gname = g.map_or(src.root_name().to_string(), |c| {
                src.subgraph_name(self.tree.subgraph[c])
            });
            graphviz_pure_core::diag::dump(&format!("DG {gname} nodes={}", dg.nodes.len()));
            for d in &dg.nodes {
                graphviz_pure_core::diag::dump(&format!("  {} {}", d.addr, d.name));
            }
        }
        Some(dg)
    }

    /// `getEdgeList(n, g)`: edges of `n` sorted counterclockwise.
    fn get_edge_list(dg: &DG, comp: &Comp, n: usize) -> Vec<ERec> {
        let mut erecs: Vec<ERec> = dg
            .all_edges(comp, n)
            .into_iter()
            .map(|e| {
                let ed = &dg.edges[e];
                let m = if ed.h == n { ed.t } else { ed.h };
                let dx = dg.nodes[m].pos[0] - dg.nodes[n].pos[0];
                let dy = dg.nodes[m].pos[1] - dg.nodes[n].pos[1];
                ERec {
                    e,
                    alpha: math::atan2(dy, dx),
                    dist2: dx.mul_add(dx, dy * dy),
                }
            })
            .collect();
        qsort_by(&mut erecs, |a, b| {
            if a.alpha > b.alpha {
                1
            } else if a.alpha < b.alpha {
                -1
            } else if a.dist2 > b.dist2 {
                1
            } else if a.dist2 < b.dist2 {
                -1
            } else {
                0
            }
        });
        let deg = erecs.len();
        if deg >= 2 {
            let mut i = 0;
            while i < deg - 1 {
                let a = erecs[i].alpha;
                let mut j = i + 1;
                while j < deg && erecs[j].alpha == a {
                    j += 1;
                }
                if j == i + 1 {
                    i = j;
                } else {
                    let bnd = if j == deg { PI } else { erecs[j].alpha };
                    let delta = ((bnd - a) / (j - i) as f64).min(ANG);
                    let mut inc = 0.0;
                    while i < j {
                        erecs[i].alpha += inc;
                        inc += delta;
                        i += 1;
                    }
                }
            }
        }
        erecs
    }

    /// `genPorts(n, er, pp, idx, bnd)`.
    fn gen_ports(
        &self,
        dg: &DG,
        n: usize,
        er: &ERec,
        pp: &mut [BPort],
        idx: usize,
        bnd: f64,
    ) -> usize {
        let e = &dg.edges[er.e];
        let cnt = e.reals.len();
        let other = if e.h == n { e.t } else { e.h };
        let mut delta = ((bnd - er.alpha) / cnt as f64).min(ANG);
        let mut angle = er.alpha;
        let (mut i, inc): (isize, isize) = if dg.nodes[n].addr < dg.nodes[other].addr {
            (idx as isize, 1)
        } else {
            angle = delta.mul_add((cnt - 1) as f64, angle);
            delta = -delta;
            ((idx + cnt - 1) as isize, -1)
        };
        for &el in &e.reals {
            let t = self.rg.edges[el].tail;
            let h = self.rg.edges[el].head;
            let pn = if self.dnode[t] == Some((dg.uid, n)) {
                t
            } else {
                h
            };
            pp[i as usize] = BPort {
                e: el,
                n: pn,
                alpha: angle,
            };
            i += inc;
            angle += delta;
        }
        idx + cnt
    }

    /// `expandCluster(n, cg)`: ports of cluster `c` from the layout of `cg`.
    fn expand_cluster(&mut self, dg: &DG, comp: &Comp, n: usize, c: usize) {
        let sz = dg.nodes[n].wdeg as usize;
        if sz == 0 {
            return;
        }
        let mut pp = vec![
            BPort {
                e: 0,
                n: 0,
                alpha: 0.0
            };
            sz
        ];
        let es = Self::get_edge_list(dg, comp, n);
        let mut idx = 0;
        for k in 0..es.len() {
            let bnd = if k + 1 < es.len() {
                es[k + 1].alpha
            } else {
                2.0 * PI + es[0].alpha
            };
            idx = self.gen_ports(dg, n, &es[k], &mut pp, idx, bnd);
        }
        if super::trace(4) {
            let src = self.rg.src;
            let nm = |n: usize| src.nodes[n].name.as_str();
            graphviz_pure_core::diag::dump(&format!(
                "EC {}",
                src.subgraphs[self.tree.subgraph[c]].name
            ));
            for p in &pp {
                let e = &self.rg.edges[p.e];
                graphviz_pure_core::diag::dump(&format!(
                    "  {} {} {:?}",
                    nm(e.tail),
                    nm(e.head),
                    p.alpha
                ));
            }
            for er in &es {
                let e = &dg.edges[er.e];
                graphviz_pure_core::diag::dump(&format!(
                    "  er {} {} {:?} {:?}",
                    dg.nodes[e.t].name, dg.nodes[e.h].name, er.alpha, er.dist2
                ));
            }
        }
        self.ports[c] = pp;
    }

    /// `finalCC(g, cc, pts, rg, infop)`.
    fn final_cc(
        &self,
        dg: &mut DG,
        cc: &[Comp],
        bbs: &[BoundingBox],
        pts: Option<&[Point]>,
        g: Option<usize>,
    ) -> BoundingBox {
        let is_root = g.is_none();
        let mut is_empty = false;
        let mut bb;
        if !cc.is_empty() {
            bb = bbs[0];
            if cc.len() > 1 {
                // putGraphs returns NULL when packing runs out of cells (issue #174), and
                // C's `*pp++` then dereferences NULL. Use zero offsets, as the translation
                // loop below does for a NULL `pts`.
                let off = |i: usize| pts.map_or_else(Point::default, |pts| pts[i]);
                let pt = off(0);
                bb = BoundingBox::new(
                    bb.ll.x + pt.x,
                    bb.ll.y + pt.y,
                    bb.ur.x + pt.x,
                    bb.ur.y + pt.y,
                );
                for i in 1..cc.len() {
                    let b = bbs[i];
                    let pt = off(i);
                    let b = BoundingBox::new(
                        b.ll.x + pt.x,
                        b.ll.y + pt.y,
                        b.ur.x + pt.x,
                        b.ur.y + pt.y,
                    );
                    bb.ll.x = bb.ll.x.min(b.ll.x);
                    bb.ll.y = bb.ll.y.min(b.ll.y);
                    bb.ur.x = bb.ur.x.max(b.ur.x);
                    bb.ur.y = bb.ur.y.max(b.ur.y);
                }
            }
        } else {
            let a = self.gattrs(g);
            bb = BoundingBox::new(
                0.0,
                0.0,
                late_int(a, "width", 54, 3) as f64,
                late_int(a, "height", 36, 3) as f64,
            );
            is_empty = true;
        }
        let (label, border) = match g {
            None => (self.rg.label.as_ref(), [Point::default(); 4]),
            Some(c) => (
                self.rg.clusters[c].label.as_ref(),
                self.rg.clusters[c].border,
            ),
        };
        if let Some(l) = label {
            is_empty = false;
            let mut d = l.dimen.x.round() - (bb.ur.x - bb.ll.x);
            if d > 0.0 {
                d /= 2.0;
                bb.ll.x -= d;
                bb.ur.x += d;
            }
        }
        let margin = if is_root || is_empty {
            0
        } else {
            late_int(self.gattrs(g), "margin", CL_OFFSET as i32, 0)
        } as f64;
        let pt = Point::new(-bb.ll.x + margin, -bb.ll.y + margin + border[0].y);
        bb.ll = Point::new(0.0, 0.0);
        bb.ur.x += pt.x + margin;
        bb.ur.y += pt.y + margin + border[2].y;

        for (i, comp) in cc.iter().enumerate() {
            let p = match pts {
                Some(pts) => Point::new(pts[i].x + pt.x, pts[i].y + pt.y),
                None => pt,
            };
            let del = Point::new(p.x / 72.0, p.y / 72.0);
            for &n in &comp.nodes {
                dg.nodes[n].pos[0] += del.x;
                dg.nodes[n].pos[1] += del.y;
            }
        }
        BoundingBox::new(
            bb.ll.x / 72.0,
            bb.ll.y / 72.0,
            bb.ur.x / 72.0,
            bb.ur.y / 72.0,
        )
    }

    /// `layout(g, infop, counter)`; `Err` on non-comparable clusters.
    /// `agnameof(g)` of the graph laid out: the root or a cluster.
    fn gname(&self, g: Option<usize>) -> String {
        let src = self.rg.src;
        match g.and_then(|c| self.tree.subgraph.get(c)) {
            Some(&s) => src.subgraph_name(s),
            None => src.root_name().to_string(),
        }
    }

    fn layout(&mut self, g: Option<usize>) -> std::result::Result<(), ()> {
        graphviz_pure_core::verbose_msg!("layout {}\n", self.gname(g));
        for n in self.members_of(g) {
            self.dnode[n] = None;
        }
        let mut dg = self.derive_graph(g).ok_or(())?;
        let (mut cc, pinned) = find_ccomp(&dg, &mut self.cc_counter);

        for ci in 0..cc.len() {
            let mut xpms = XParams::default();
            fdp_t_layout(&mut self.st, &mut dg, &cc[ci], &mut xpms);
            self.dbg("TL", g, &dg, &cc[ci], false);
            for n in cc[ci].nodes.clone() {
                match dg.nodes[n].kind {
                    DKind::Clust(c) => {
                        self.expand_cluster(&dg, &cc[ci], n, c);
                        self.layout(Some(c))?;
                        let bb = self.gbb[c];
                        let dn = &mut dg.nodes[n];
                        dn.width = bb.ur.x;
                        dn.height = bb.ur.y;
                        let pt = Point::new(72.0 * bb.ur.x, 72.0 * bb.ur.y);
                        dn.lw = pt.x / 2.0;
                        dn.rw = pt.x / 2.0;
                        dn.ht = pt.y;
                    }
                    DKind::Port => cc[ci].remove(n),
                    DKind::Real(_) => {}
                }
            }
            let comp = &cc[ci];
            if comp.nodes.len() >= 2 {
                if g.is_none() && dg.attrs.contains_key("normalize") {
                    // normalize(cg) reads `normalize` on the derived graph: a command-line one.
                    with_lgraph(&mut dg, &comp.nodes, |lg, sub, _| {
                        normalize(lg, sub);
                    });
                }
                fdp_x_layout(&mut self.st, &mut dg, comp, &xpms);
            }
            self.dbg("XL", g, &dg, &cc[ci], true);
        }

        let keep: Vec<usize> = (0..dg.nodes.len())
            .filter(|&n| cc.iter().any(|c| c.member[n]))
            .collect();
        let mut pinfo = self.pack.clone();
        let (bbs, pts) = with_lgraph(&mut dg, &keep, |lg, _, map| {
            let mut subs: Vec<Sub> = cc
                .iter()
                .map(|c| {
                    let nodes = c.nodes.iter().map(|&n| map[n]).collect();
                    let mut s = Sub::new(lg.nodes.len(), nodes);
                    s.name.clone_from(&c.name);
                    s
                })
                .collect();
            let pts = if subs.len() > 1 {
                if pinned {
                    let mut bp = vec![false; subs.len()];
                    bp[0] = true;
                    pinfo.fixed = Some(bp);
                } else {
                    pinfo.fixed = None;
                }
                put_graphs(lg, &mut subs, &mut pinfo)
            } else {
                if let Some(s) = subs.first_mut() {
                    compute_bb(lg, s, false);
                }
                None
            };
            (subs.iter().map(|s| s.bb).collect::<Vec<_>>(), pts)
        });
        let bb = self.final_cc(&mut dg, &cc, &bbs, pts.as_deref(), g);

        for dn in &dg.nodes {
            match dn.kind {
                DKind::Clust(c) => {
                    let llx = dn.pos[0] - dn.width / 2.0;
                    let lly = dn.pos[1] - dn.height / 2.0;
                    self.gbb[c] = BoundingBox::new(llx, lly, llx + dn.width, lly + dn.height);
                }
                DKind::Real(n) => {
                    self.rg.nodes[n].pos[0] = dn.pos[0];
                    self.rg.nodes[n].pos[1] = dn.pos[1];
                }
                DKind::Port => {}
            }
        }
        self.set_bb(g, bb);
        // freeDerivedGraph: agclose frees the nodes in sequence order.
        for dn in &dg.nodes {
            self.heap.free(dn.addr);
        }
        graphviz_pure_core::verbose_msg!("end {}\n", self.gname(g));
        Ok(())
    }

    /// `GVDBG=1` trace of derived positions (matches an instrumented C build).
    fn dbg(&self, tag: &str, g: Option<usize>, dg: &DG, comp: &Comp, sizes: bool) {
        if !super::trace(1) {
            return;
        }
        let src = self.rg.src;
        let gname = g.map_or(src.root_name().to_string(), |c| {
            src.subgraph_name(self.tree.subgraph[c])
        });
        graphviz_pure_core::diag::dump(&format!("{tag} {gname}"));
        for &n in &comp.nodes {
            let d = &dg.nodes[n];
            let name = match d.kind {
                DKind::Clust(c) => src.subgraphs[self.tree.subgraph[c]].name.clone(),
                DKind::Real(r) => src.nodes[r].name.clone(),
                DKind::Port => "_port".to_string(),
            };
            if sizes {
                graphviz_pure_core::diag::dump(&format!(
                    "  {name} {:?} {:?} {:?} {:?}",
                    d.pos[0], d.pos[1], d.width, d.height
                ));
            } else {
                graphviz_pure_core::diag::dump(&format!("  {name} {:?} {:?}", d.pos[0], d.pos[1]));
            }
        }
    }

    /// `setClustNodes(root)`: a cluster node becomes a box the size of its
    /// cluster, centred in it (in the cluster's coordinates).
    fn set_clust_nodes(&mut self) {
        for n in 0..self.rg.nodes.len() {
            if !self.is_clust[n] {
                continue;
            }
            let Some(p) = self.parent[n] else { continue };
            let bb = self.bb_of(p);
            let w = bb.ur.x - bb.ll.x;
            let h = bb.ur.y - bb.ll.y;
            let w2 = 72.0 * (w / 2.0);
            let h2 = 72.0 * (h / 2.0);
            let h_pts = 72.0 * h;
            let penwidth = crate::common::utils::late_double(
                &self.rg.src.nodes[n].attrs,
                "penwidth",
                1.0,
                0.0,
            );
            let nd = &mut self.rg.nodes[n];
            nd.pos[0] = w / 2.0;
            nd.pos[1] = h / 2.0;
            nd.width = w;
            nd.height = h;
            nd.lw = w2;
            nd.rw = w2;
            nd.ht = h_pts;
            let sh = &mut nd.shape;
            sh.width = 72.0 * w;
            sh.height = 72.0 * h;
            // ND_outline_width = w + penwidth (inches plus points, as in C).
            sh.outline_width = 72.0 * (w + penwidth);
            sh.outline_height = 72.0 * (h + penwidth);
            let (rw, lw) = (w2, w2);
            let e = penwidth / 2.0;
            let v = [
                Point::new(rw, h2),
                Point::new(-lw, h2),
                Point::new(-lw, -h2),
                Point::new(rw, -h2),
                Point::new(rw + e, h2 + e),
                Point::new(-lw - e, h2 + e),
                Point::new(-lw - e, -h2 - e),
                Point::new(rw + e, -h2 - e),
            ];
            if sh.vertices.len() < 8 {
                sh.vertices.resize(8, Point::default());
            }
            sh.vertices[..8].copy_from_slice(&v);
        }
    }

    /// `evalPositions(g, rootg)`.
    fn eval_positions(&mut self, g: Option<usize>) {
        let bb = self.bb_of(g);
        if g.is_some() {
            for n in self.members_of(g) {
                if self.parent[n] != Some(g) {
                    continue;
                }
                self.rg.nodes[n].pos[0] += bb.ll.x;
                self.rg.nodes[n].pos[1] += bb.ll.y;
            }
        }
        for c in self.tree.kids(g).to_vec() {
            if g.is_some() {
                let s = &mut self.gbb[c];
                s.ll.x += bb.ll.x;
                s.ll.y += bb.ll.y;
                s.ur.x += bb.ll.x;
                s.ur.y += bb.ll.y;
            }
            self.eval_positions(Some(c));
        }
    }
}

/// `fdpSplines(g)`.
fn fdp_splines(g: &mut LGraph<'_>, has_clust_edge: bool, hier: &Hier<'_>) {
    let mut et = g.edge_type;
    let root = g.root_sub();
    let mut done = false;
    if et > EdgeType::Ortho {
        // layout.c:1057: `Nop = 0` afterwards, for the rest of the run.
        let _nop = crate::neatogen::neatosplines::ClearNop::new();
        let mut try_failed = false;
        if et == EdgeType::CompoundEdge {
            let ok = spline_edges_by(g, &root, COMPOUND_EDGE_TYPE, &mut |lg, sub, pm, _| {
                compound_edges(lg, sub, hier, pm)
            });
            if ok {
                done = true;
            } else {
                // layout.c:1046: when doing the edges again, accept those compoundEdges did.
                try_failed = true;
                crate::neatogen::neatosplines::set_nop(2);
            }
        }
        if try_failed || et != EdgeType::CompoundEdge {
            if has_clust_edge {
                graphviz_pure_core::diag::agwarningf(
                    "splines and cluster edges not supported - using line segments\n",
                );
                et = EdgeType::Line;
            } else {
                spline_edges_by(g, &root, et, &mut |lg, sub, pm, et| {
                    let et = if et == EdgeType::CompoundEdge {
                        EdgeType::Pline
                    } else {
                        et
                    };
                    spline_edges_route(lg, sub, pm, et);
                    true
                });
                done = true;
            }
        }
    }
    if !done {
        spline_edges_main(g, &root, et);
    }
}

/// `fdp_layout(g)`.
pub fn fdp_layout(graph: &mut Graph) -> Result<()> {
    let tree0 = mk_clusters(graph);
    let ce = crate::common::cluster_edges::process_cluster_edges(graph, &tree0);
    let src = ce
        .as_ref()
        .map_or_else(|| graph.clone(), |c| c.work.clone());
    let heap = NodeHeap {
        recycle: ce.is_none(),
        ..NodeHeap::default()
    };
    // With cluster edges, C's processClusterEdges frees memory of other
    // allocator zones that later derived nodes reuse; creation order
    // matches C there, so no slot is ever recycled.
    let is_clust: Vec<bool> = (0..src.nodes.len())
        .map(|n| ce.as_ref().is_some_and(|c| c.is_clust_node(n)))
        .collect();
    let attrs = &src.graph_attrs;
    let ndim = late_int(attrs, "dim", 2, 2).min(MAXDIM as i32) as usize;
    let mut g = LGraph::new(&src, ndim, EdgeType::Line);
    let tree = mk_clusters(&src);
    register_clusters(&mut g, &tree);
    let st = fdp_init_params(attrs);
    init_edges(&mut g, st.d.k);
    let inputscale = get_inputscale(&g);
    initial_positions(&mut g, inputscale);

    let mut pack = PackInfo::default();
    pack.mode = crate::pack::pack_info(
        g.gattr("pack"),
        g.gattr("packmode"),
        PackMode::Node,
        CL_OFFSET / 2,
        &mut pack,
    );

    let nc = tree.len();
    let members: Vec<Vec<usize>> = tree
        .subgraph
        .iter()
        .map(|&si| subgraph_member_nodes(&src, si))
        .collect();
    let cedges: Vec<Vec<bool>> = tree
        .subgraph
        .iter()
        .map(|&si| {
            let mut v = vec![false; src.edges.len()];
            for e in crate::neatogen::neatoinit::subgraph_edges(&src, si) {
                v[e] = true;
            }
            v
        })
        .collect();
    let coords_declared = src.graph_attr_declared("coords");
    let n = g.nodes.len();
    let mut f = Fdp {
        rg: &mut g,
        tree,
        st,
        ports: vec![Vec::new(); nc],
        gbb: vec![BoundingBox::default(); nc],
        root_bb: BoundingBox::default(),
        members,
        cedges,
        parent: vec![None; n],
        dnode: vec![None; n],
        next_uid: 0,
        cc_counter: 0,
        pack,
        coords_declared,
        inputscale,
        is_clust,
        labeled: vec![false; nc],
        heap,
    };
    let ok = f.layout(None).is_ok();
    if ok {
        f.set_clust_nodes();
        f.eval_positions(None);
    }
    let finish = |g: &LGraph<'_>, graph: &mut Graph| match &ce {
        Some(c) => {
            let mut w = c.work.clone();
            g.write_back(&mut w);
            let view = crate::common::cluster_edges::render_view(c, &w, graph);
            crate::common::cluster_edges::undo_cluster_edges(c, &w, graph);
            graph.render_view = Some(Box::new(view));
        }
        None => g.write_back(graph),
    };
    if !ok {
        // C reports the error and returns before positioning anything: every cluster
        // stays in GD_clust with an empty box, the clusters deriveGraph reached have
        // their label, and all labels stay at the origin.
        let labeled: Vec<(usize, bool)> = f
            .tree
            .subgraph
            .iter()
            .copied()
            .zip(f.labeled.iter().copied())
            .collect();
        finish(&g, graph);
        let origin = Point::default();
        let mark = |gr: &mut Graph| {
            gr.layout.label_pos = Some(origin);
            for &(sg, has_label) in &labeled {
                let l = &mut gr.subgraphs[sg].layout;
                l.reg = graphviz_pure_core::ClusterReg::Registered;
                if has_label {
                    l.label_pos = Some(origin);
                } else {
                    l.no_label = true;
                }
            }
        };
        mark(graph);
        if let Some(v) = graph.render_view.as_deref_mut() {
            mark(v);
        }
        return Ok(());
    }
    // setBB
    let root_bb = f.root_bb;
    let gbb = f.gbb.clone();
    let parent: Vec<Option<usize>> = f.parent.iter().map(|p| p.flatten()).collect();
    let is_clust = f.is_clust.clone();
    let tree = f.tree.clone();
    let members = f.members.clone();
    let sc = |b: BoundingBox| {
        BoundingBox::new(72.0 * b.ll.x, 72.0 * b.ll.y, 72.0 * b.ur.x, 72.0 * b.ur.y)
    };
    g.bb = sc(root_bb);
    for (c, b) in gbb.into_iter().enumerate() {
        g.clusters[c].bb = sc(b);
    }

    let root = g.root_sub();
    neato_set_aspect(&mut g, &root, true);
    if g.edge_type != EdgeType::None {
        let hier = Hier {
            tree: &tree,
            parent: &parent,
            is_clust: &is_clust,
            members: &members,
        };
        fdp_splines(&mut g, ce.is_some(), &hier);
    }
    gv_postprocess(&mut g, false);
    finish(&g, graph);
    Ok(())
}
