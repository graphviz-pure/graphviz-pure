//! Port of `lib/dotgen/dotsplines.c`: edge routing for dot.
//!
//! `dot_splines_` collects every edge that needs a spline (regular edges from
//! normal nodes and virtual merge points, flat edges, self loops), sorts them
//! with `edgecmp` so equivalent multi-edges are adjacent, and routes each
//! group: self loops with `makeSelfEdge`, flat edges with `make_flat_edge`
//! and regular edges with `make_regular_edge`, which builds a corridor of
//! boxes down the virtual node chain and hands it to `routesplines`.
//!
//! C passes temporary edge copies (`makefwdedge`, the `fwdedgea/b` stack
//! pairs) to the routing helpers; here they are pushed onto the edge arena
//! and dropped again when `dot_splines_` returns.

use super::Dot;
use super::qsort::qsort;
use super::types::*;
use crate::common::geom::add;
use crate::common::routespl::{
    Path, PathEnd, RealEdge, add_box, routepolylines, routesplines, simple_spline_route,
};
use crate::common::splines::{
    EDGETYPEMASK, FLATEDGE, LabelKind, REGULAREDGE, SELFNPEDGE, SELFWPEDGE, beginpath,
    clip_and_install, endpath, getsplinepoints, make_self_edge, orig_edge, place_portlabel,
    update_bb_label,
};
use crate::common::types::EdgeType;
use graphviz_pure_core::Point;
use graphviz_pure_core::cnum::{c_max, c_min};
use std::cmp::Ordering;
use std::f64::consts::PI;

/// `MINW`: minimum width of a box in the edge path.
const MINW: f64 = 16.0;
const HALFMINW: f64 = 8.0;
const FWDEDGE: i32 = 16;
const BWDEDGE: i32 = 32;
const MAINGRAPH: i32 = 64;
const AUXGRAPH: i32 = 128;
const GRAPHTYPEMASK: i32 = 192;
/// `FUDGE` in `maximal_bbox`.
const FUDGE: f64 = 4.0;
/// `LBL_SPACE`: space between labels, in points.
const LBL_SPACE: f64 = 6.0;
/// `splines=ortho` uses the `lib/ortho` port.
const ORTHO_PORTED: bool = true;

/// `spline_info_t`.
#[derive(Debug, Clone, Default)]
pub struct SplineInfoT {
    pub left_bound: f64,
    pub right_bound: f64,
    pub splinesep: f64,
    pub multisep: f64,
    pub rank_box: Vec<BoxF>,
}

/// `portcmp(p0, p1)`.
pub fn portcmp(p0: &Port, p1: &Port) -> i32 {
    if !p1.defined {
        return if p0.defined { 1 } else { 0 };
    }
    if !p0.defined {
        return -1;
    }
    if p0.p.x < p1.p.x {
        return -1;
    }
    if p0.p.x > p1.p.x {
        return 1;
    }
    if p0.p.y < p1.p.y {
        return -1;
    }
    if p0.p.y > p1.p.y {
        return 1;
    }
    0
}

/// `swap_bezier(b)`.
fn swap_bezier(b: &mut Bezier) {
    b.list.reverse();
    std::mem::swap(&mut b.sflag, &mut b.eflag);
    std::mem::swap(&mut b.sp, &mut b.ep);
}

/// `swap_spline(s)`.
pub fn swap_spline(s: &mut [Bezier]) {
    s.reverse();
    for b in s.iter_mut() {
        swap_bezier(b);
    }
}

/// `pathend_t tend, hend` of `make_regular_edge`, `make_flat_edge`,
/// `make_flat_labeled_edge` and `make_flat_bottom_edges`, as the last call of each
/// left them (#360). C declares them without an initialiser, and where `record_path`
/// writes no box, `beginpath`/`endpath` leave in them what they held: in practice,
/// what the previous call of the same function left there, on every run and on two
/// C builds measured. One slot per function is what matches C. The first two
/// functions are called from `dot_splines_`, and the other two from `make_flat_edge`,
/// one frame deeper. `edgecmp` sorts self loops and flat edges before regular ones,
/// so nothing else runs between two `make_regular_edge` calls. Between flat calls,
/// another flat router or the adjacent edges' graph can run, and the slots still
/// matched C there. A new `Dot` starts with none: C does not reuse an earlier graph's
/// or `pack` component's ends. See `docs/porting/crashes.md`.
#[derive(Debug, Default)]
pub struct StaleEnds {
    regular: (PathEnd, PathEnd),
    flat: (PathEnd, PathEnd),
    flat_labeled: (PathEnd, PathEnd),
    flat_bottom: (PathEnd, PathEnd),
}

/// The size of `pathend_t.boxes` in C.
const PATHEND_MAX_BOXES: usize = 20;

/// The boxes C's `endp` holds on entry to `beginpath`/`endpath`, for where
/// `record_path` writes none (#360): the previous call's, or `None` on a function's
/// first call, where C reads unrelated stack memory (its output then differs
/// between builds, and it often crashes) and the port uses the node box.
fn stale_boxes(endp: &PathEnd) -> Option<Vec<BoxF>> {
    (!endp.boxes.is_empty()).then(|| endp.boxes.clone())
}

/// `if (b.LL.x < b.UR.x && b.LL.y < b.UR.y) endp->boxes[endp->boxn++] = b;`. With
/// boxes kept from earlier calls (#360) the 20 boxes of `pathend_t` can be full,
/// and C then writes past them; the port fails the layout before that write.
fn push_end_box(endp: &mut PathEnd, b: BoxF) -> Result<(), String> {
    if !box_ok(&b) {
        return Ok(());
    }
    if endp.boxes.len() >= PATHEND_MAX_BOXES {
        return Err(format!(
            "dot_splines: an edge end already holds the {PATHEND_MAX_BOXES} boxes C has room \
             for, kept from earlier edges"
        ));
    }
    endp.boxes.push(b);
    Ok(())
}

/// `makeregularend(b, side, y)`.
fn makeregularend(b: BoxF, side: u8, y: f64) -> BoxF {
    if side == BOTTOM {
        BoxF::new(b.ll.x, y, b.ur.x, b.ll.y)
    } else {
        BoxF::new(b.ll.x, b.ur.y, b.ur.x, y)
    }
}

fn box_ok(b: &BoxF) -> bool {
    b.ll.x < b.ur.x && b.ll.y < b.ur.y
}

/// `leftOf(p1, p2, p3)`: whether `p3` is left of the ray `p1 -> p2`.
fn left_of(p1: Point, p2: Point, p3: Point) -> bool {
    // C (clang) fuses this multiply-subtract.
    (p1.y - p2.y).mul_add(p3.x - p2.x, -((p3.y - p2.y) * (p1.x - p2.x))) > 0.0
}

/// The attribute symbols `make_flat_adj_edges`' aux graph is laid out with (`cloneGraph`,
/// `setState`).
///
/// auxg starts with cgraph's ProtoGraph symbols (the node `label` and the `-N`/`-E` defaults)
/// at the ids the main graph gave them; cloneGraph then declares the main graph's other node
/// and edge attributes in name order (agnxtattr), then `headport`, `tailport` and `weight`
/// where the main graph has none. setState rebinds some global symbols to auxg and NULLs others
/// ([`AuxSyms::NULL_NODE`], [`AuxSyms::NULL_EDGE`]). N_penwidth, E_penwidth and E_arrowsz keep
/// the main graph's `Agsym_t`, so agxget reads the aux attribute whose id is theirs, whatever
/// its name. N_fixed and E_label_float are looked up as "fixed" and "label_float".
struct AuxSyms<'g> {
    main: &'g graphviz_pure_core::AttrDicts,
    /// Aux node attribute names by id.
    node: Vec<String>,
    /// Aux edge attribute names by id.
    edge: Vec<String>,
}

impl<'g> AuxSyms<'g> {
    const NULL_NODE: [&'static str; 6] = [
        "xlabel",
        "group",
        "style",
        "nojustify",
        "showboxes",
        "fontcolor",
    ];
    const NULL_EDGE: [&'static str; 7] = [
        "constraint",
        "minlen",
        "headlabel",
        "taillabel",
        "xlabel",
        "fontcolor",
        "labelfontcolor",
    ];

    fn new(main: &'g graphviz_pure_core::AttrDicts) -> Self {
        // gvContext declares the node `label` on the ProtoGraph first: id 0 in every root.
        let mut node = Self::names(&main.node);
        node.retain(|k| k != "label");
        node.insert(0, "label".to_string());
        let mut edge = Self::names(&main.edge);
        for k in ["headport", "tailport", "weight"] {
            if !main.edge.contains_key(k) {
                edge.push(k.to_string());
            }
        }
        AuxSyms { main, node, edge }
    }

    /// auxg's dictionaries (cloneGraph, setState). The graph dictionary holds what agopen
    /// copies from the ProtoGraph, its symbols with their ids and `-G` defaults, and `rank`
    /// with default `""`. A `-G` symbol is `fixed` (`global_def`, input.c:191), so no root
    /// statement changes its default: the main root's is the ProtoGraph's. The node and edge dictionaries hold every symbol of the main root,
    /// with the main root's default (cloneGraph's agattr updates a ProtoGraph symbol in
    /// place), including those only a subgraph's defaults declared; then `headport`,
    /// `tailport` and `weight` with `""` where the main graph has none. agnewsym makes every
    /// name a plain string and leaves `print` unset.
    fn dicts(&self) -> graphviz_pure_core::AttrDicts {
        use graphviz_pure_core::AttrSym;
        type Dict = std::collections::BTreeMap<String, AttrSym>;
        fn declare(dict: &mut Dict, main: &Dict, names: &[String], label: bool) {
            for (id, k) in names.iter().enumerate() {
                let sym = match main.get(k) {
                    Some(m) => AttrSym {
                        defval: m.defval.clone(),
                        id,
                        html_defval: m.html_defval,
                        proto: m.proto,
                        ..AttrSym::default()
                    },
                    // The node `label` the ProtoGraph declares in every gvContext.
                    None if label && k == "label" => AttrSym {
                        defval: "\\N".to_string(),
                        id,
                        proto: true,
                        ..AttrSym::default()
                    },
                    None => AttrSym {
                        id,
                        ..AttrSym::default()
                    },
                };
                dict.insert(k.clone(), sym);
            }
        }
        let mut d = graphviz_pure_core::AttrDicts::default();
        for (k, m) in self.main.graph.iter().filter(|(_, s)| s.proto) {
            let sym = AttrSym {
                defval: m.defval.clone(),
                id: m.id,
                proto: true,
                ..AttrSym::default()
            };
            d.graph.insert(k.clone(), sym);
        }
        // agattr_text(auxg, AGRAPH, "rank", ""): a ProtoGraph `rank` keeps its id.
        let id = d.graph.len();
        d.graph
            .entry("rank".to_string())
            .or_insert_with(|| AttrSym {
                id,
                ..AttrSym::default()
            })
            .defval = String::new();
        declare(&mut d.node, &self.main.node, &self.node, true);
        declare(&mut d.edge, &self.main.edge, &self.edge, false);
        d
    }

    /// The aux ids of a main dictionary's symbols: auxg copies the ProtoGraph's symbols with
    /// their ids, as the root did, so they come first in id order; cloneGraph then declares
    /// the others in name order.
    fn names(
        dict: &std::collections::BTreeMap<String, graphviz_pure_core::AttrSym>,
    ) -> Vec<String> {
        let mut proto: Vec<(usize, &String)> = dict
            .iter()
            .filter(|(_, s)| s.proto)
            .map(|(k, s)| (s.id, k))
            .collect();
        proto.sort();
        let mut names: Vec<String> = proto.into_iter().map(|(_, k)| k.clone()).collect();
        names.extend(
            dict.iter()
                .filter(|(_, s)| !s.proto)
                .map(|(k, _)| k.clone()),
        );
        names
    }

    /// Sets each of `keys` to what the main graph's symbol of that name reads on an aux object
    /// whose attributes are `raw`. A cgraph object holds a value for every attribute declared
    /// on its graph; the port's objects lack one only for an attribute declared after they
    /// were made, which cgraph gave them as "".
    fn stale(
        dict: &std::collections::BTreeMap<String, graphviz_pure_core::AttrSym>,
        names: &[String],
        raw: &graphviz_pure_core::AttrMap,
        attrs: &mut graphviz_pure_core::AttrMap,
        keys: &[&str],
    ) {
        for &k in keys {
            match dict.get(k).and_then(|sym| names.get(sym.id)) {
                Some(an) => {
                    attrs.insert(k.to_string(), raw.get(an).cloned().unwrap_or_default());
                }
                None => {
                    attrs.remove(k);
                }
            }
        }
    }

    /// A cloned node's attributes as dot_init_node(auxg) reads them.
    fn fix_node(&self, attrs: &mut graphviz_pure_core::AttrMap) {
        let raw = attrs.clone();
        Self::stale(&self.main.node, &self.node, &raw, attrs, &["penwidth"]);
        if self.main.node.contains_key("fixed") {
            let v = raw.get("fixed").cloned().unwrap_or_default();
            attrs.insert("fixedsize".to_string(), v);
        } else {
            attrs.remove("fixedsize");
        }
        for k in Self::NULL_NODE {
            attrs.remove(k);
        }
    }

    /// An aux edge's attributes as dot_init_edge(auxg) and the splines read them.
    fn fix_edge(&self, attrs: &mut graphviz_pure_core::AttrMap) {
        let raw = attrs.clone();
        Self::stale(
            &self.main.edge,
            &self.edge,
            &raw,
            attrs,
            &["arrowsize", "penwidth"],
        );
        // E_label_float is looked up as "label_float", not "labelfloat" (dotsplines.c:745),
        // so ED_label_ontop of an aux edge is the value of an attribute `label_float`.
        if self.main.edge.contains_key("label_float") {
            let v = raw.get("label_float").cloned().unwrap_or_default();
            attrs.insert("labelfloat".to_string(), v);
        } else {
            attrs.remove("labelfloat");
        }
        for k in Self::NULL_EDGE {
            attrs.remove(k);
        }
    }
}

impl Dot<'_> {
    /// `EDGE_TYPE(g)`.
    pub fn edge_type(&self) -> EdgeType {
        self.et
    }

    /// `getmainedge(e)`.
    fn getmainedge(&self, e: E) -> E {
        let mut le = e;
        while let Some(v) = self.edges[le].to_virt {
            le = v;
        }
        while let Some(o) = self.edges[le].to_orig {
            le = o;
        }
        le
    }

    /// `spline_merge(n)`.
    pub fn spline_merge_p(&self, n: N) -> bool {
        let nd = &self.nodes[n];
        nd.node_type == VIRTUAL && (nd.in_.len() > 1 || nd.out.len() > 1)
    }

    /// `swap_ends_p(e)`.
    pub fn swap_ends_p(&self, e: E) -> bool {
        let mut e = e;
        while let Some(o) = self.edges[e].to_orig {
            e = o;
        }
        let (h, t) = (self.edges[e].head, self.edges[e].tail);
        if self.nodes[h].rank > self.nodes[t].rank {
            return false;
        }
        if self.nodes[h].rank < self.nodes[t].rank {
            return true;
        }
        self.nodes[h].order < self.nodes[t].order
    }

    /// Pushes a copy of edge `e` onto the arena (an `Agedgepair_t` on the C stack).
    fn copy_edge(&mut self, e: E) -> E {
        let c = self.edges[e].clone();
        self.edges.push(c);
        self.edges.len() - 1
    }

    /// `makefwdedge(new, old)`: a forward copy of the back edge `old`.
    fn makefwdedge(&mut self, old: E) -> E {
        let new = self.copy_edge(old);
        let o = &self.edges[old];
        let (t, h, tp, hp) = (o.tail, o.head, o.tail_port.clone(), o.head_port.clone());
        let ne = &mut self.edges[new];
        ne.tail = h;
        ne.head = t;
        ne.tail_port = hp;
        ne.head_port = tp;
        ne.edge_type = VIRTUAL;
        ne.to_orig = Some(old);
        new
    }

    /// `resetRW(g)`.
    fn reset_rw(&mut self) {
        for n in self.nodes_of(0) {
            let nd = &mut self.nodes[n];
            std::mem::swap(&mut nd.rw, &mut nd.mval);
        }
    }

    /// `updateBB(g, l)`.
    pub fn update_bb_l(&mut self, l: L) {
        let flip = self.graphs[0].flip();
        let lb = &self.labels[l];
        let (d, p) = (lb.dimen, lb.pos);
        crate::common::splines::add_label_bb_dims(&mut self.graphs[0].bb, d, p, flip);
    }

    /// `setEdgeLabelPos(g)` (used before ortho routing).
    fn set_edge_label_pos(&mut self) {
        let mut n = self.graphs[0].nlist;
        while let Some(v) = n {
            if self.nodes[v].node_type == VIRTUAL {
                let mut l = None;
                if let Some(fe) = self.nodes[v].alg {
                    let lab = self.edges[fe].label.expect("flat label edge");
                    self.labels[lab].pos = self.nodes[v].coord;
                    self.labels[lab].set = true;
                    l = Some(lab);
                } else if let Some(lab) = self.nodes[v].label {
                    self.place_vnlabel(v);
                    l = Some(lab);
                }
                if let Some(lab) = l {
                    self.update_bb_l(lab);
                }
            }
            n = self.nodes[v].next;
        }
    }

    /// `dot_splines(g)`.
    pub fn dot_splines(&mut self) -> Result<(), String> {
        self.dot_splines_(true)
    }

    /// `dot_splines_(g, normalize)`.
    pub fn dot_splines_(&mut self, normalize: bool) -> Result<(), String> {
        let mut et = self.edge_type();
        if et == EdgeType::Ortho && !ORTHO_PORTED {
            et = EdgeType::Spline;
        }
        if et == EdgeType::None {
            return Ok(());
        }
        let edge_base = self.edges.len();
        if et == EdgeType::Curved {
            self.reset_rw();
            if self
                .graphs
                .first()
                .is_some_and(|g| g.has_labels & EDGE_LABEL != 0)
            {
                graphviz_pure_core::diag::agwarningf(
                    "edge labels with splines=curved not supported in dot - use xlabels\n",
                );
            }
        }
        if et == EdgeType::Ortho && ORTHO_PORTED {
            self.reset_rw();
            let use_lbls = self.graphs[0].has_labels & EDGE_LABEL != 0;
            if use_lbls {
                self.set_edge_label_pos();
            }
            if !crate::ortho::ortho_edges(self, use_lbls) {
                // orthoEdges returns -1 when mkMaze fails (e.g. partition leaves
                // no free cell, #203): dot_splines_ returns it before `finish`,
                // and doDot gives up on the layout with nothing routed.
                self.aborted = true;
                self.edges.truncate(edge_base);
                return Ok(());
            }
            self.finish_splines();
            self.edges.truncate(edge_base);
            return Ok(());
        }
        self.mark_lowclusters(0);
        let route = crate::common::routespl::routesplines_init();
        let nodesep = self.graphs[0].nodesep;
        let mut sd = SplineInfoT {
            splinesep: f64::from(nodesep / 4),
            multisep: f64::from(nodesep),
            ..SplineInfoT::default()
        };
        let mut edges: Vec<E> = Vec::new();
        let (minr, maxr) = (self.graphs[0].minrank, self.graphs[0].maxrank);
        for i in minr..=maxr {
            let rn = self.rank_n(0, i);
            if let Some(n) = self.rv(0, i, 0) {
                sd.left_bound = c_min(sd.left_bound, self.nodes[n].coord.x - self.nodes[n].lw);
            }
            if rn > 0
                && let Some(n) = self.rv(0, i, rn - 1)
            {
                sd.right_bound = c_max(sd.right_bound, self.nodes[n].coord.x + self.nodes[n].rw);
            }
            sd.left_bound -= MINW;
            sd.right_bound += MINW;
            for j in 0..rn {
                let Some(n) = self.rv(0, i, j) else { continue };
                if let Some(fe) = self.nodes[n].alg {
                    let l = self.edges[fe].label.expect("flat edge label");
                    self.labels[l].pos = self.nodes[n].coord;
                    self.labels[l].set = true;
                }
                if self.nodes[n].node_type != NORMAL && !self.spline_merge_p(n) {
                    continue;
                }
                for e in self.nodes[n].out.clone() {
                    let ty = self.edges[e].edge_type;
                    if ty == FLATORDER || ty == IGNORED {
                        continue;
                    }
                    self.setflags(e, REGULAREDGE, FWDEDGE, MAINGRAPH);
                    edges.push(e);
                }
                for e in self.nodes[n].flat_out.clone() {
                    self.setflags(e, FLATEDGE, 0, AUXGRAPH);
                    edges.push(e);
                }
                // ND_other(n).list is allocated for every real node.
                if self.nodes[n].node_type == NORMAL {
                    let nd = &mut self.nodes[n];
                    std::mem::swap(&mut nd.rw, &mut nd.mval);
                }
                for e in self.nodes[n].other.clone() {
                    self.setflags(e, 0, 0, AUXGRAPH);
                    edges.push(e);
                }
            }
        }

        // Sort so that equivalent edges are contiguous.
        qsort(&mut edges, &mut |a: &E, b: &E| self.edgecmp(*a, *b).cmp(&0));
        sd.rank_box = vec![BoxF::default(); (maxr + 1).max(0) as usize];

        if et == EdgeType::Line {
            let mut n = self.graphs[0].nlist;
            while let Some(v) = n {
                if self.nodes[v].node_type == VIRTUAL && self.nodes[v].label.is_some() {
                    self.place_vnlabel(v);
                }
                n = self.nodes[v].next;
            }
        }

        let mut l = 0usize;
        while l < edges.len() {
            let ind = l;
            let e0 = edges[l];
            l += 1;
            let le0 = self.getmainedge(e0);
            let mut ea = if self.edges[e0].tail_port.defined || self.edges[e0].head_port.defined {
                e0
            } else {
                le0
            };
            if self.edges[ea].tree_index & BWDEDGE != 0 {
                ea = self.makefwdedge(ea);
            }
            let mut cnt = 1usize;
            while l < edges.len() {
                let e1 = edges[l];
                let le1 = self.getmainedge(e1);
                if le0 != le1 {
                    break;
                }
                if self.edges[e0].adjacent {
                    cnt += 1;
                    l += 1;
                    continue;
                }
                let mut eb = if self.edges[e1].tail_port.defined || self.edges[e1].head_port.defined
                {
                    e1
                } else {
                    le1
                };
                if self.edges[eb].tree_index & BWDEDGE != 0 {
                    eb = self.makefwdedge(eb);
                }
                if portcmp(&self.edges[ea].tail_port, &self.edges[eb].tail_port) != 0 {
                    break;
                }
                if portcmp(&self.edges[ea].head_port, &self.edges[eb].head_port) != 0 {
                    break;
                }
                if (self.edges[e0].tree_index & EDGETYPEMASK) == FLATEDGE
                    && self.edges[e0].label != self.edges[e1].label
                {
                    break;
                }
                if self.edges[edges[l]].tree_index & MAINGRAPH != 0 {
                    break;
                }
                cnt += 1;
                l += 1;
            }
            let group: Vec<E> = edges[ind..ind + cnt].to_vec();
            let (t0, h0) = (self.edges[e0].tail, self.edges[e0].head);
            if et == EdgeType::Curved {
                let mut edgelist = group.clone();
                edgelist[0] = self.getmainedge(edges[ind]);
                self.make_straight_edges(&edgelist, et);
            } else if t0 == h0 {
                let n = t0;
                let r = self.nodes[n].rank;
                let y = |d: &Self, rr: i32| d.nodes[d.rv(0, rr, 0).unwrap()].coord.y;
                let sizey = if r == maxr {
                    if r > 0 {
                        y(self, r - 1) - self.nodes[n].coord.y
                    } else {
                        self.nodes[n].ht
                    }
                } else if r == minr {
                    self.nodes[n].coord.y - y(self, r + 1)
                } else {
                    let upy = y(self, r - 1) - self.nodes[n].coord.y;
                    let dwny = self.nodes[n].coord.y - y(self, r + 1);
                    upy.min(dwny)
                };
                make_self_edge(self, &group, sd.multisep, sizey / 2.0);
                for &e in &group {
                    if let Some(lb) = self.edges[e].label {
                        self.update_bb_l(lb);
                    }
                }
            } else if self.nodes[t0].rank == self.nodes[h0].rank {
                self.make_flat_edge(&sd, &group, et)?;
            } else {
                self.make_regular_edge(&mut sd, &group, et)?;
            }
        }

        // place regular edge labels
        let mut n = self.graphs[0].nlist;
        while let Some(v) = n {
            if self.nodes[v].node_type == VIRTUAL
                && let Some(lb) = self.nodes[v].label
            {
                self.place_vnlabel(v);
                self.update_bb_l(lb);
            }
            n = self.nodes[v].next;
        }

        // normalize splines so they always go from tail to head
        if normalize {
            self.edge_normalize();
        }
        if et == EdgeType::Curved {
            // C calls no routesplinesterm for curved edges: its level stays open
            std::mem::forget(route);
        } else {
            drop(route);
        }
        self.finish_splines();
        self.edges.truncate(edge_base);
        Ok(())
    }

    /// The `finish:` tail of `dot_splines_`: place port labels.
    fn finish_splines(&mut self) {
        self.splines_done = true;
        let declared = |k: &str| self.edge_attr_declared(k);
        let has_head = declared("headlabel");
        let has_tail = declared("taillabel");
        if (has_head || has_tail) && (declared("labelangle") || declared("labeldistance")) {
            for n in self.nodes_of(0) {
                if has_head {
                    for e in self.in_edges(0, n) {
                        if self.edges[e].head_label.is_some() {
                            place_portlabel(self, e, true);
                            update_bb_label(self, e, LabelKind::Head);
                        }
                    }
                }
                if has_tail {
                    for e in self.out_edges(0, n) {
                        if self.edges[e].tail_label.is_some() && place_portlabel(self, e, false) {
                            update_bb_label(self, e, LabelKind::Tail);
                        }
                    }
                }
            }
        }
    }

    /// `edge_normalize(g)`.
    fn edge_normalize(&mut self) {
        for n in self.nodes_of(0) {
            for e in self.out_edges(0, n) {
                if self.swap_ends_p(e)
                    && let Some(s) = self.edges[e].spl.as_mut()
                {
                    swap_spline(s);
                }
            }
        }
    }

    /// `place_vnlabel(n)`.
    pub fn place_vnlabel(&mut self, n: N) {
        if self.nodes[n].in_.is_empty() {
            return;
        }
        let mut e = self.nodes[n].out[0];
        while self.edges[e].edge_type != NORMAL {
            e = self.edges[e]
                .to_orig
                .expect("place_vnlabel: no original edge");
        }
        let l = self.edges[e].label.expect("label vnode without label");
        let dimen = self.labels[l].dimen;
        let width = if self.graphs[0].flip() {
            dimen.y
        } else {
            dimen.x
        };
        let c = self.nodes[n].coord;
        self.labels[l].pos = Point::new(c.x + width / 2.0, c.y);
        self.labels[l].set = true;
    }

    /// `setflags(e, hint1, hint2, f3)`.
    fn setflags(&mut self, e: E, hint1: i32, hint2: i32, f3: i32) {
        let ed = &self.edges[e];
        let (t, h) = (ed.tail, ed.head);
        let f1 = if hint1 != 0 {
            hint1
        } else if t == h {
            if ed.tail_port.defined || ed.head_port.defined {
                SELFWPEDGE
            } else {
                SELFNPEDGE
            }
        } else if self.nodes[t].rank == self.nodes[h].rank {
            FLATEDGE
        } else {
            REGULAREDGE
        };
        let f2 = if hint2 != 0 {
            hint2
        } else if f1 == REGULAREDGE {
            if self.nodes[t].rank < self.nodes[h].rank {
                FWDEDGE
            } else {
                BWDEDGE
            }
        } else if f1 == FLATEDGE {
            if self.nodes[t].order < self.nodes[h].order {
                FWDEDGE
            } else {
                BWDEDGE
            }
        } else {
            FWDEDGE
        };
        self.edges[e].tree_index = f1 | f2 | f3;
    }

    /// The tail/head ports of `e` as `makefwdedge` would see them.
    fn fwd_ports(&self, e: E) -> (&Port, &Port) {
        let ed = &self.edges[e];
        if ed.tree_index & BWDEDGE != 0 {
            (&ed.head_port, &ed.tail_port)
        } else {
            (&ed.tail_port, &ed.head_port)
        }
    }

    /// `edgecmp(p0, p1)`.
    fn edgecmp(&self, e0: E, e1: E) -> i32 {
        let et0 = self.edges[e0].tree_index & EDGETYPEMASK;
        let et1 = self.edges[e1].tree_index & EDGETYPEMASK;
        if et0 < et1 {
            return 1;
        }
        if et0 > et1 {
            return -1;
        }
        let le0 = self.getmainedge(e0);
        let le1 = self.getmainedge(e1);
        let rd = |le: E| {
            let ed = &self.edges[le];
            (self.nodes[ed.tail].rank - self.nodes[ed.head].rank).abs()
        };
        let (v0, v1) = (rd(le0), rd(le1));
        if v0 < v1 {
            return -1;
        }
        if v0 > v1 {
            return 1;
        }
        let xd = |le: E| {
            let ed = &self.edges[le];
            (self.nodes[ed.tail].coord.x - self.nodes[ed.head].coord.x).abs()
        };
        let (v0, v1) = (xd(le0), xd(le1));
        if v0 < v1 {
            return -1;
        }
        if v0 > v1 {
            return 1;
        }
        let (s0, s1) = (self.edges[le0].seq, self.edges[le1].seq);
        if s0 < s1 {
            return -1;
        }
        if s0 > s1 {
            return 1;
        }
        let ea = if self.edges[e0].tail_port.defined || self.edges[e0].head_port.defined {
            e0
        } else {
            le0
        };
        let eb = if self.edges[e1].tail_port.defined || self.edges[e1].head_port.defined {
            e1
        } else {
            le1
        };
        let (ta, ha) = self.fwd_ports(ea);
        let (tb, hb) = self.fwd_ports(eb);
        let rv = portcmp(ta, tb);
        if rv != 0 {
            return rv;
        }
        let rv = portcmp(ha, hb);
        if rv != 0 {
            return rv;
        }
        let g0 = self.edges[e0].tree_index & GRAPHTYPEMASK;
        let g1 = self.edges[e1].tree_index & GRAPHTYPEMASK;
        if g0 < g1 {
            return -1;
        }
        if g0 > g1 {
            return 1;
        }
        // (C compares label pointers here when et0 == FLATEDGE, which cannot
        // hold once et0 holds the graph type bits.)
        let (s0, s1) = (self.edges[e0].seq, self.edges[e1].seq);
        if s0 < s1 {
            return -1;
        }
        if s0 > s1 {
            return 1;
        }
        0
    }

    // ---------------------------------------------------------------- flat edges

    /// `makeSimpleFlatLabels(tn, hn, edges, ind, cnt, et, n_lbls)`.
    fn make_simple_flat_labels(&mut self, tn: N, hn: N, edges: &[E], et: EdgeType, n_lbls: usize) {
        let cnt = edges.len();
        let e = edges[0];
        let mut earray = edges.to_vec();
        qsort(&mut earray, &mut |a: &E, b: &E| self.edgelblcmpfn(*a, *b));
        let tp = add(self.nodes[tn].coord, self.edges[e].tail_port.p);
        let hp = add(self.nodes[hn].coord, self.edges[e].head_port.p);
        let leftend = tp.x + self.nodes[tn].rw;
        let rightend = hp.x - self.nodes[hn].lw;
        let ctrx = (leftend + rightend) / 2.0;

        let e = earray[0];
        let points = [tp, tp, hp, hp];
        let h = self.edges[e].head;
        clip_and_install(self, e, h, &points);
        let l0 = self.edges[e].label.unwrap();
        let d0 = self.labels[l0].dimen;
        self.labels[l0].pos = Point::new(ctrx, tp.y + (d0.y + LBL_SPACE) / 2.0);
        self.labels[l0].set = true;

        let mut miny = tp.y + LBL_SPACE / 2.0;
        let mut maxy = miny + d0.y;
        let uminx = ctrx - d0.x / 2.0;
        let umaxx = ctrx + d0.x / 2.0;
        let mut lminx = 0.0;
        let mut lmaxx = 0.0;
        let polyline = et == EdgeType::Pline;
        let mut i = 1;
        while i < n_lbls {
            let e = earray[i];
            let l = self.edges[e].label.unwrap();
            let d = self.labels[l].dimen;
            let mut points = [Point::default(); 8];
            let ctry;
            if i % 2 == 1 {
                if i == 1 {
                    lminx = ctrx - d.x / 2.0;
                    lmaxx = ctrx + d.x / 2.0;
                }
                miny -= LBL_SPACE + d.y;
                points[0] = tp;
                points[1] = Point::new(tp.x, miny - LBL_SPACE);
                points[2] = Point::new(hp.x, points[1].y);
                points[3] = hp;
                points[4] = Point::new(lmaxx, hp.y);
                points[5] = Point::new(lmaxx, miny);
                points[6] = Point::new(lminx, miny);
                points[7] = Point::new(lminx, tp.y);
                ctry = miny + d.y / 2.0;
            } else {
                points[0] = tp;
                points[1] = Point::new(uminx, tp.y);
                points[2] = Point::new(uminx, maxy);
                points[3] = Point::new(umaxx, maxy);
                points[4] = Point::new(umaxx, hp.y);
                points[5] = hp;
                points[6] = Point::new(hp.x, maxy + LBL_SPACE);
                points[7] = Point::new(tp.x, maxy + LBL_SPACE);
                ctry = maxy + d.y / 2.0 + LBL_SPACE;
                maxy += d.y + LBL_SPACE;
            }
            let Some(ps) = simple_spline_route(tp, hp, &points, polyline).filter(|p| !p.is_empty())
            else {
                return;
            };
            self.labels[l].pos = Point::new(ctrx, ctry);
            self.labels[l].set = true;
            let h = self.edges[e].head;
            clip_and_install(self, e, h, &ps);
            i += 1;
        }
        // edges with no labels
        while i < cnt {
            let e = earray[i];
            let mut points = [Point::default(); 8];
            if i % 2 == 1 {
                if i == 1 {
                    lminx = (2.0 * leftend + rightend) / 3.0;
                    lmaxx = (leftend + 2.0 * rightend) / 3.0;
                }
                miny -= LBL_SPACE;
                points[0] = tp;
                points[1] = Point::new(tp.x, miny - LBL_SPACE);
                points[2] = Point::new(hp.x, points[1].y);
                points[3] = hp;
                points[4] = Point::new(lmaxx, hp.y);
                points[5] = Point::new(lmaxx, miny);
                points[6] = Point::new(lminx, miny);
                points[7] = Point::new(lminx, tp.y);
            } else {
                points[0] = tp;
                points[1] = Point::new(uminx, tp.y);
                points[2] = Point::new(uminx, maxy);
                points[3] = Point::new(umaxx, maxy);
                points[4] = Point::new(umaxx, hp.y);
                points[5] = hp;
                points[6] = Point::new(hp.x, maxy + LBL_SPACE);
                points[7] = Point::new(tp.x, maxy + LBL_SPACE);
                maxy += LBL_SPACE;
            }
            let Some(ps) = simple_spline_route(tp, hp, &points, polyline).filter(|p| !p.is_empty())
            else {
                return;
            };
            let h = self.edges[e].head;
            clip_and_install(self, e, h, &ps);
            i += 1;
        }
    }

    /// `edgelblcmpfn(x, y)`.
    fn edgelblcmpfn(&self, e0: E, e1: E) -> Ordering {
        match (self.edges[e0].label, self.edges[e1].label) {
            (Some(l0), Some(l1)) => {
                let (s0, s1) = (self.labels[l0].dimen, self.labels[l1].dimen);
                if s0.x > s1.x {
                    Ordering::Less
                } else if s0.x < s1.x {
                    Ordering::Greater
                } else if s0.y > s1.y {
                    Ordering::Less
                } else if s0.y < s1.y {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }

    /// `makeSimpleFlat(tn, hn, edges, ind, cnt, et)`.
    fn make_simple_flat(&mut self, tn: N, hn: N, edges: &[E], et: EdgeType) {
        let cnt = edges.len();
        let e = edges[0];
        let tp = add(self.nodes[tn].coord, self.edges[e].tail_port.p);
        let hp = add(self.nodes[hn].coord, self.edges[e].head_port.p);
        let stepy = if cnt > 1 {
            self.nodes[tn].ht / (cnt - 1) as f64
        } else {
            0.0
        };
        let mut dy = tp.y
            - if cnt > 1 {
                self.nodes[tn].ht / 2.0
            } else {
                0.0
            };
        for &e in edges {
            let a = Point::new((2.0 * tp.x + hp.x) / 3.0, dy);
            let b = Point::new((2.0 * hp.x + tp.x) / 3.0, dy);
            let points: Vec<Point> = if et == EdgeType::Spline || et == EdgeType::Line {
                vec![tp, a, b, hp]
            } else {
                vec![tp, tp, a, a, a, b, b, b, hp, hp]
            };
            dy += stepy;
            let h = self.edges[e].head;
            clip_and_install(self, e, h, &points);
        }
    }

    /// `make_flat_adj_edges(g, edges, ind, cnt, e0, et)`.
    fn make_flat_adj_edges(&mut self, edges: &[E], e0: E, et: EdgeType) -> Result<(), String> {
        let (tn, hn) = (self.edges[e0].tail, self.edges[e0].head);
        if self.nodes[tn].is_record || self.nodes[hn].is_record {
            // dotsplines.c:1144-1151: a `static atomic_flag`, so once per process.
            static WARNED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                graphviz_pure_core::diag::agwarningf(
                    "flat edge between adjacent nodes one of which has a record shape - replace \
                     records with HTML-like labels\n",
                );
                let op = self.edge_op();
                // dotsplines.c:1147: both names share agnameof's static buffer when
                // anonymous.
                let [t, h] = graphviz_pure_core::agnameof_buffer([
                    self.node_agnameof(tn),
                    self.node_agnameof(hn),
                ]);
                graphviz_pure_core::diag::agerr_prev(&format!("  Edge {t} {op} {h}\n"));
            }
            return Ok(());
        }
        let mut labels = 0;
        let mut ports = false;
        for &e in edges {
            if self.edges[e].label.is_some() {
                labels += 1;
            }
            if self.edges[e].tail_port.defined || self.edges[e].head_port.defined {
                ports = true;
            }
        }
        if !ports {
            if labels == 0 {
                self.make_simple_flat(tn, hn, edges, et);
            } else {
                self.make_simple_flat_labels(tn, hn, edges, et, labels);
            }
            return Ok(());
        }
        self.make_flat_adj_edges_aux(edges, tn, hn, et)
    }

    /// `agisdirected(g) ? "->" : "--"`.
    fn edge_op(&self) -> &'static str {
        if self.input.kind == graphviz_pure_core::GraphKind::Directed {
            "->"
        } else {
            "--"
        }
    }

    /// The recursive-dot part of `make_flat_adj_edges` (edges with ports):
    /// lay out the two nodes and their edges as a separate graph with the
    /// rank direction turned (`cloneGraph`, `cloneNode`, `cloneEdge`), then
    /// copy the splines back (`transformf`).
    fn make_flat_adj_edges_aux(
        &mut self,
        edges: &[E],
        tn0: N,
        hn0: N,
        et: EdgeType,
    ) -> Result<(), String> {
        let flip = self.graphs[0].flip();
        let input = self.input;
        // cloneGraph: node/edge attribute declarations, rankdir turned.
        let mut aux = graphviz_pure_core::Graph::new("auxg", input.kind, false);
        // `agnode(auxg, agnameof(orign), 1)` keeps a `%` name as a string of auxg's own.
        aux.local_node_names_kept = true;
        let empty = graphviz_pure_core::AttrDicts::default();
        let syms = AuxSyms::new(input.dicts.first().unwrap_or(&empty));
        if !input.dicts.is_empty() {
            aux.dicts = vec![syms.dicts()];
        }
        // auxg's own graph values are its dictionary's defaults: the ProtoGraph's (`-G`),
        // and `rank` "".
        match aux.dicts.first() {
            Some(d) => {
                for (k, sym) in &d.graph {
                    aux.graph_attrs.insert(k.clone(), sym.defval.clone());
                }
            }
            None => {
                aux.graph_attrs.insert("rank".to_string(), String::new());
            }
        }
        // GD_drawing(auxg)->quantum and GD_charset(auxg) come from the main graph.
        for k in ["quantum", "charset"] {
            if let Some(v) = input.graph_attrs.get(k) {
                aux.graph_attrs.insert(k.to_string(), v.clone());
            }
        }
        aux.node_defaults = input.node_defaults.clone();
        aux.edge_defaults = input.edge_defaults.clone();
        let rightx = self.nodes[hn0].coord.x;
        let leftx = self.nodes[tn0].coord.x;
        let (tn, hn) = if flip { (hn0, tn0) } else { (tn0, hn0) };
        // cloneNode: agcopyattr copies every attribute of the main node.
        let clone_node = |aux: &mut graphviz_pure_core::Graph, n: N| {
            let src = &input.nodes[self.nodes[n].real.unwrap()];
            let id = aux.ensure_node_html(&src.name, src.html_name);
            aux.nodes[id.0 as usize].attrs = src.attrs.clone();
            id
        };
        let auxt = clone_node(&mut aux, tn);
        let auxh = clone_node(&mut aux, hn);
        aux.subgraphs.push(graphviz_pure_core::SubgraphData {
            id: graphviz_pure_core::SubgraphId(0),
            name: "xxx".to_string(),
            is_cluster: false,
            graph_attrs: graphviz_pure_core::AttrMap::from_plain([(
                "rank".to_string(),
                "source".to_string(),
            )]),
            node_defaults: Default::default(),
            edge_defaults: Default::default(),
            nodes: vec![auxt],
            edges: Vec::new(),
            subgraphs: Vec::new(),
            bb: Default::default(),
            html_name: false,
            empty_name: false,
            layout: Default::default(),
        });
        // (main edge, aux edge) pairs and the "heavy" edge.
        let mut pairs: Vec<(E, usize)> = Vec::new();
        let mut hvye: Option<usize> = None;
        let mut hvye_alg = false;
        for &e0 in edges {
            let mut e = e0;
            while self.edges[e].edge_type != NORMAL {
                e = self.edges[e].to_orig.unwrap();
            }
            let src = &input.edges[self.edges[e].real.unwrap()];
            let (t, h) = if self.edges[e].tail == tn {
                (auxt, auxh)
            } else {
                (auxh, auxt)
            };
            // cloneEdge: agcopyattr copies every attribute of the main edge.
            let id = aux.add_edge(t, h, Default::default());
            let ae = &mut aux.edges[id.0 as usize];
            ae.attrs = src.attrs.clone();
            ae.tail_port = src.tail_port.clone();
            ae.head_port = src.head_port.clone();
            let auxe = id.0 as usize;
            self.edges[e].alg = Some(auxe);
            pairs.push((e, auxe));
            if hvye.is_none()
                && !self.edges[e].tail_port.defined
                && !self.edges[e].head_port.defined
            {
                hvye = Some(auxe);
                hvye_alg = true;
            }
        }
        let hvye = match hvye {
            Some(h) => h,
            None => {
                // agedge gives the pseudo-edge auxg's defaults, ports included.
                let h = aux.add_edge(auxt, auxh, Default::default()).0 as usize;
                let port = |s: Option<&String>| match s.map(String::as_str) {
                    None | Some("") => graphviz_pure_core::Port::default(),
                    Some(v) => match v.split_once(':') {
                        Some((a, b)) => graphviz_pure_core::Port {
                            name: Some(a.to_string()),
                            compass: Some(b.to_string()),
                        },
                        None => graphviz_pure_core::Port {
                            name: Some(v.to_string()),
                            compass: None,
                        },
                    },
                };
                let ae = &mut aux.edges[h];
                ae.tail_port = port(ae.attrs.get("tailport"));
                ae.head_port = port(ae.attrs.get("headport"));
                h
            }
        };
        aux.edges[hvye]
            .attrs
            .insert("weight".to_string(), "10000".to_string());
        // dot_init_node_edge(auxg) reads the attributes from here on, through the symbols
        // setState left: stale ones read the aux attribute with the same id, NULLed ones
        // nothing.
        for nd in &mut aux.nodes {
            syms.fix_node(&mut nd.attrs);
        }
        for ed in &mut aux.edges {
            syms.fix_edge(&mut ed.attrs);
        }
        for k in AuxSyms::NULL_NODE {
            aux.node_defaults.remove(k);
        }
        for k in AuxSyms::NULL_EDGE.into_iter().chain(["labelfloat"]) {
            aux.edge_defaults.remove(k);
        }
        // The aux graph's objects are measured afresh, as C's dot_init_node_edge(auxg) does.
        // Its diagnostics print there, below, through the symbols above: C repeats the
        // label diagnostics of the copies, but not `size too small for label` (N_fixed).
        let _fonts = graphviz_pure_core::font::textfont_other_graph();
        // htmlEntityUTF8 is called for auxg from here on: its invalid UTF-8 warning names
        // auxg, and can print again (lastg).
        let _charset = graphviz_pure_core::charset::enter_graph(&aux.name);
        graphviz_pure_core::diag::quietly(|| graphviz_pure_geom::size_graph_nodes(&mut aux));

        let mut d = Dot::new(&aux);
        d.graphs[0].rankdir = if flip { RANKDIR_TB } else { RANKDIR_LR };
        d.graphs[0].nodesep = self.graphs[0].nodesep;
        d.graphs[0].ranksep = self.graphs[0].ranksep;
        d.cl_type = self.cl_type;
        d.concentrate = self.concentrate;
        d.null_syms = (&AuxSyms::NULL_NODE, &AuxSyms::NULL_EDGE);
        d.et = et;
        d.dot_init_subg();
        d.dot_init_node_edge();
        d.dot_rank()?;
        d.dot_mincross()?;
        d.dot_position()?;
        // reposition
        let (auxt, auxh) = (auxt.0 as usize, auxh.0 as usize);
        let midx = (self.nodes[tn].coord.x - self.nodes[tn].rw
            + self.nodes[hn].coord.x
            + self.nodes[hn].lw)
            / 2.0;
        let midy = (d.nodes[auxt].coord.x + d.nodes[auxh].coord.x) / 2.0;
        let mut n = d.graphs[0].nlist;
        while let Some(v) = n {
            if v == auxt {
                d.nodes[v].coord = Point::new(midy, rightx);
            } else if v == auxh {
                d.nodes[v].coord = Point::new(midy, leftx);
            } else {
                d.nodes[v].coord.y = midx;
            }
            n = d.nodes[v].next;
        }
        d.dot_sameports();
        d.dot_splines_(false)?;
        d.dotneato_postprocess();

        // copy splines
        let ct = self.nodes[tn].coord;
        let at = d.nodes[auxt].coord;
        let del = if flip {
            Point::new(ct.x - at.y, ct.y + at.x)
        } else {
            Point::new(ct.x - at.x, ct.y - at.y)
        };
        let transformf = |p: Point| {
            let q = if flip { Point::new(p.y, -p.x) } else { p };
            add(q, del)
        };
        for (e, auxe) in pairs {
            if auxe == hvye && !hvye_alg {
                continue;
            }
            // dotsplines.c:1260: `auxbz = ED_spl(auxe)->list`, unchecked. The aux layout leaves
            // an edge no spline where its routing failed (it reports `lost t h edge`) or
            // concentrate merged it, and C dereferences the NULL and crashes. The layout stops
            // there, after the aux graph's messages, which are C's last output (#341).
            let Some(spl) = d.edges[auxe].spl.as_ref() else {
                let ed = &self.edges[e];
                return Err(format!(
                    "make_flat_adj_edges: the layout of the adjacent flat edges gave edge \
                     {} {} {} no spline (C dereferences its NULL spline)",
                    self.node_agnameof(ed.tail).name,
                    self.edge_op(),
                    self.node_agnameof(ed.head).name
                ));
            };
            // `new_spline` always appends a bezier, so a spline C reads has one: `list` is
            // never empty here.
            let Some(auxbz) = spl.first() else {
                continue;
            };
            let mut bz = Bezier {
                sflag: auxbz.sflag,
                sp: transformf(auxbz.sp),
                eflag: auxbz.eflag,
                ep: transformf(auxbz.ep),
                list: auxbz.list.iter().map(|&p| transformf(p)).collect(),
            };
            let mut j = 0;
            while j + 3 < bz.list.len() {
                let cp = [bz.list[j], bz.list[j + 1], bz.list[j + 2], bz.list[j + 3]];
                crate::common::splines::update_bb_bz(&mut self.graphs[0].bb, &cp);
                j += 3;
            }
            bz.list.shrink_to_fit();
            crate::common::splines::new_spline(self, e, bz);
            if let (Some(l), Some(al)) = (self.edges[e].label, d.edges[auxe].label) {
                self.labels[l].pos = transformf(d.labels[al].pos);
                self.labels[l].set = true;
                self.update_bb_l(l);
            }
        }
        Ok(())
    }

    /// `makeStraightEdges` for `splines=curved` (routespl.c).
    fn make_straight_edges(&mut self, edge_list: &[E], et: EdgeType) {
        let nodesep = i64::from(self.graphs[0].nodesep);
        let conc = self.concentrate;
        crate::common::routespl::make_straight_edges(
            self,
            edge_list,
            et,
            nodesep,
            conc,
            &|d, e| d.get_cycle_centroid(e),
        );
    }

    /// `get_cycle_centroid(g, edge)` (routespl.c): centre of the shortest cycle
    /// of length >= 3 through `edge`, else of the graph's bounding box.
    fn get_cycle_centroid(&self, edge: E) -> Point {
        fn dfs(d: &Dot<'_>, search: N, visited: &mut Vec<N>, end: N, cycles: &mut Vec<Vec<N>>) {
            if visited.contains(&search) {
                if search == end
                    && !cycles
                        .iter()
                        .any(|c| c.len() == visited.len() && c.iter().all(|x| visited.contains(x)))
                {
                    cycles.push(visited.clone());
                }
            } else {
                visited.push(search);
                for e in d.out_edges(0, search) {
                    dfs(d, d.edges[e].head, visited, end, cycles);
                }
                visited.pop();
            }
        }
        let mut cycles = Vec::new();
        for n in self.nodes_of(0) {
            let mut visited = Vec::new();
            dfs(self, n, &mut visited, n, &mut cycles);
        }
        let (start, end) = (self.edges[edge].tail, self.edges[edge].head);
        let contains = |c: &Vec<N>| {
            let l = c.len();
            (0..l).any(|i| c[if i == 0 { l - 1 } else { i - 1 }] == start && c[i] == end)
        };
        let mut shortest: Option<&Vec<N>> = None;
        for c in &cycles {
            if c.len() < 3 {
                continue;
            }
            if shortest.is_none_or(|s| s.len() > c.len()) && contains(c) {
                shortest = Some(c);
            }
        }
        let Some(cycle) = shortest else {
            let bb = self.graphs[0].bb;
            return Point::new((bb.ll.x + bb.ur.x) / 2.0, (bb.ll.y + bb.ur.y) / 2.0);
        };
        let mut sum = Point::new(0.0, 0.0);
        for &n in cycle {
            sum.x += self.nodes[n].coord.x;
            sum.y += self.nodes[n].coord.y;
        }
        let cnt = cycle.len() as f64;
        Point::new(sum.x / cnt, sum.y / cnt)
    }

    /// Runs one of the routers with its `tend`/`hend` slot of [`StaleEnds`], which
    /// keeps what the router leaves in them, whichever way it returns.
    fn with_ends<R>(
        &mut self,
        slot: fn(&mut StaleEnds) -> &mut (PathEnd, PathEnd),
        f: impl FnOnce(&mut Self, (&mut PathEnd, &mut PathEnd)) -> R,
    ) -> R {
        let mut ends = std::mem::take(slot(&mut self.stale_ends));
        let r = f(self, (&mut ends.0, &mut ends.1));
        *slot(&mut self.stale_ends) = ends;
        r
    }

    /// `makeFlatEnd(g, sp, P, n, e, endp, isBegin)`.
    fn make_flat_end(
        &mut self,
        sp: &SplineInfoT,
        p: &mut Path,
        n: N,
        e: E,
        endp: &mut PathEnd,
        is_begin: bool,
    ) -> Result<(), String> {
        let mut b = self.maximal_bbox(sp, n, None, Some(e));
        endp.nb = b;
        endp.sidemask = TOP;
        if is_begin {
            let kept = stale_boxes(endp);
            beginpath(self, p, e, FLATEDGE, endp, false, kept);
        } else {
            let kept = stale_boxes(endp);
            endpath(self, p, e, FLATEDGE, endp, false, kept);
        }
        let last = *endp.boxes.last().unwrap();
        b.ur.y = last.ur.y;
        b.ll.y = last.ll.y;
        let r = self.nodes[n].rank;
        let b = makeregularend(b, TOP, self.nodes[n].coord.y + self.rk(0, r).ht2);
        push_end_box(endp, b)
    }

    /// `makeBottomFlatEnd(g, sp, P, n, e, endp, isBegin)`.
    fn make_bottom_flat_end(
        &mut self,
        sp: &SplineInfoT,
        p: &mut Path,
        n: N,
        e: E,
        endp: &mut PathEnd,
        is_begin: bool,
    ) -> Result<(), String> {
        let mut b = self.maximal_bbox(sp, n, None, Some(e));
        endp.nb = b;
        endp.sidemask = BOTTOM;
        if is_begin {
            let kept = stale_boxes(endp);
            beginpath(self, p, e, FLATEDGE, endp, false, kept);
        } else {
            let kept = stale_boxes(endp);
            endpath(self, p, e, FLATEDGE, endp, false, kept);
        }
        let last = *endp.boxes.last().unwrap();
        b.ur.y = last.ur.y;
        b.ll.y = last.ll.y;
        let r = self.nodes[n].rank;
        let b = makeregularend(b, BOTTOM, self.nodes[n].coord.y - self.rk(0, r).ht2);
        push_end_box(endp, b)
    }

    fn route(&self, p: &mut Path, spline: bool) -> Option<Vec<Point>> {
        let real = orig_edge(self, p.data);
        let (t, h) = (self.edges[real].tail, self.edges[real].head);
        let real = RealEdge {
            tail: self.node_agnameof(t),
            head: self.node_agnameof(h),
            is_loop: t == h,
        };
        if spline {
            routesplines(p, real)
        } else {
            routepolylines(p, real)
        }
    }

    /// `make_flat_labeled_edge(g, sp, P, e, et)`.
    fn make_flat_labeled_edge(
        &mut self,
        sp: &SplineInfoT,
        e: E,
        et: EdgeType,
    ) -> Result<(), String> {
        self.with_ends(
            |s| &mut s.flat_labeled,
            |d, ends| d.make_flat_labeled_edge_with(sp, e, et, ends),
        )
    }

    fn make_flat_labeled_edge_with(
        &mut self,
        sp: &SplineInfoT,
        e: E,
        et: EdgeType,
        (tend, hend): (&mut PathEnd, &mut PathEnd),
    ) -> Result<(), String> {
        let (tn, hn) = (self.edges[e].tail, self.edges[e].head);
        let mut f = self.edges[e]
            .to_virt
            .expect("flat labeled edge without label node");
        while let Some(v) = self.edges[f].to_virt {
            f = v;
        }
        let ln = self.edges[f].tail;
        let l = self.edges[e].label.unwrap();
        self.labels[l].pos = self.nodes[ln].coord;
        self.labels[l].set = true;
        let ps: Vec<Point>;
        if et == EdgeType::Line {
            let startp = add(self.nodes[tn].coord, self.edges[e].tail_port.p);
            let endp = add(self.nodes[hn].coord, self.edges[e].head_port.p);
            let mut lp = self.labels[l].pos;
            lp.y -= self.labels[l].dimen.y / 2.0;
            ps = vec![startp, startp, lp, lp, lp, endp, endp];
        } else {
            let lnd = &self.nodes[ln];
            let rt = self.nodes[tn].rank;
            let mut lb = BoxF::default();
            lb.ll.x = lnd.coord.x - lnd.lw;
            lb.ur.x = lnd.coord.x + lnd.rw;
            lb.ur.y = lnd.coord.y + lnd.ht / 2.0;
            let mut ydelta =
                lnd.coord.y - self.rk(0, rt).ht1 - self.nodes[tn].coord.y + self.rk(0, rt).ht2;
            ydelta /= 6.0;
            lb.ll.y = lb.ur.y - c_max(5.0, ydelta);
            let mut p = Path::default();
            self.make_flat_end(sp, &mut p, tn, e, tend, true)?;
            self.make_flat_end(sp, &mut p, hn, e, hend, false)?;
            let tl = *tend.boxes.last().unwrap();
            let hl = *hend.boxes.last().unwrap();
            let boxes = [
                BoxF::new(tl.ll.x, tl.ur.y, lb.ll.x, lb.ll.y),
                BoxF::new(tl.ll.x, lb.ll.y, hl.ur.x, lb.ur.y),
                BoxF::new(lb.ur.x, hl.ur.y, hl.ur.x, lb.ll.y),
            ];
            for b in &tend.boxes {
                add_box(&mut p, *b);
            }
            for b in boxes {
                add_box(&mut p, b);
            }
            for b in hend.boxes.iter().rev() {
                add_box(&mut p, *b);
            }
            match self.route(&mut p, et == EdgeType::Spline) {
                Some(v) if !v.is_empty() => ps = v,
                _ => return Ok(()),
            }
        }
        let h = self.edges[e].head;
        clip_and_install(self, e, h, &ps);
        Ok(())
    }

    /// `make_flat_bottom_edges(g, sp, P, edges, ind, cnt, e, use_splines)`.
    fn make_flat_bottom_edges(
        &mut self,
        sp: &SplineInfoT,
        edges: &[E],
        e: E,
        use_splines: bool,
    ) -> Result<(), String> {
        self.with_ends(
            |s| &mut s.flat_bottom,
            |d, ends| d.make_flat_bottom_edges_with(sp, edges, e, use_splines, ends),
        )
    }

    fn make_flat_bottom_edges_with(
        &mut self,
        sp: &SplineInfoT,
        edges: &[E],
        e: E,
        use_splines: bool,
        (tend, hend): (&mut PathEnd, &mut PathEnd),
    ) -> Result<(), String> {
        let cnt = edges.len();
        let (tn, hn) = (self.edges[e].tail, self.edges[e].head);
        let r = self.nodes[tn].rank;
        let vspace = if r < self.graphs[0].maxrank {
            let nv0 = self.rv(0, r + 1, 0).unwrap();
            self.nodes[tn].coord.y
                - self.rk(0, r).pht1
                - (self.nodes[nv0].coord.y + self.rk(0, r + 1).pht2)
        } else {
            f64::from(self.graphs[0].ranksep)
        };
        let stepx = sp.multisep / (cnt + 1) as f64;
        let stepy = vspace / (cnt + 1) as f64;
        let mut p = Path::default();
        self.make_bottom_flat_end(sp, &mut p, tn, e, tend, true)?;
        self.make_bottom_flat_end(sp, &mut p, hn, e, hend, false)?;
        for (i, &e) in edges.iter().enumerate() {
            let fi = (i + 1) as f64;
            let b = *tend.boxes.last().unwrap();
            // C (clang) fuses the `(i + 1) * step` multiply-adds of this loop.
            let b0 = BoxF::new(
                b.ll.x,
                (-fi).mul_add(stepy, b.ll.y),
                fi.mul_add(stepx, b.ur.x),
                b.ll.y,
            );
            let tl = *tend.boxes.last().unwrap();
            let hl = *hend.boxes.last().unwrap();
            let b1 = BoxF::new(tl.ll.x, b0.ll.y - stepy, hl.ur.x, b0.ll.y);
            let b = hl;
            let b2 = BoxF::new((-fi).mul_add(stepx, b.ll.x), b1.ur.y, b.ur.x, b.ll.y);
            for bx in &tend.boxes {
                add_box(&mut p, *bx);
            }
            for bx in [b0, b1, b2] {
                add_box(&mut p, bx);
            }
            for bx in hend.boxes.iter().rev() {
                add_box(&mut p, *bx);
            }
            let Some(ps) = self.route(&mut p, use_splines).filter(|v| !v.is_empty()) else {
                return Ok(());
            };
            let h = self.edges[e].head;
            clip_and_install(self, e, h, &ps);
            p.boxes.clear();
        }
        Ok(())
    }

    /// `make_flat_edge(g, sp, P, edges, ind, cnt, et)`.
    fn make_flat_edge(
        &mut self,
        sp: &SplineInfoT,
        edges: &[E],
        et: EdgeType,
    ) -> Result<(), String> {
        self.with_ends(
            |s| &mut s.flat,
            |d, ends| d.make_flat_edge_with(sp, edges, et, ends),
        )
    }

    fn make_flat_edge_with(
        &mut self,
        sp: &SplineInfoT,
        edges: &[E],
        et: EdgeType,
        (tend, hend): (&mut PathEnd, &mut PathEnd),
    ) -> Result<(), String> {
        let cnt = edges.len();
        let mut e = edges[0];
        let mut is_adjacent = self.edges[e].adjacent;
        if self.edges[e].tree_index & BWDEDGE != 0 {
            e = self.makefwdedge(e);
        }
        for &f in &edges[1..] {
            if self.edges[f].adjacent {
                is_adjacent = true;
                break;
            }
        }
        if is_adjacent {
            return self.make_flat_adj_edges(edges, e, et);
        }
        if self.edges[e].label.is_some() {
            return self.make_flat_labeled_edge(sp, e, et);
        }
        if et == EdgeType::Line {
            let (t, h) = (self.edges[e].tail, self.edges[e].head);
            self.make_simple_flat(t, h, edges, et);
            return Ok(());
        }
        let tside = self.edges[e].tail_port.side;
        let hside = self.edges[e].head_port.side;
        if (tside == BOTTOM && hside != TOP) || (hside == BOTTOM && tside != TOP) {
            return self.make_flat_bottom_edges(sp, edges, e, et == EdgeType::Spline);
        }
        let (tn, hn) = (self.edges[e].tail, self.edges[e].head);
        let r = self.nodes[tn].rank;
        let vspace = if r > 0 {
            let pr = if self.graphs[0].has_labels & EDGE_LABEL != 0 {
                r - 2
            } else {
                r - 1
            };
            let pv0 = self.rv(0, pr, 0).unwrap();
            self.nodes[pv0].coord.y
                - self.rk(0, pr).ht1
                - self.nodes[tn].coord.y
                - self.rk(0, r).ht2
        } else {
            f64::from(self.graphs[0].ranksep)
        };
        let stepx = sp.multisep / (cnt + 1) as f64;
        let stepy = vspace / (cnt + 1) as f64;
        let mut p = Path::default();
        self.make_flat_end(sp, &mut p, tn, e, tend, true)?;
        self.make_flat_end(sp, &mut p, hn, e, hend, false)?;
        for (i, &e) in edges.iter().enumerate() {
            let fi = (i + 1) as f64;
            let b = *tend.boxes.last().unwrap();
            // C (clang) fuses the `(i + 1) * step` multiply-adds of this loop.
            let b0 = BoxF::new(
                b.ll.x,
                b.ur.y,
                fi.mul_add(stepx, b.ur.x),
                fi.mul_add(stepy, b.ur.y),
            );
            let tl = *tend.boxes.last().unwrap();
            let hl = *hend.boxes.last().unwrap();
            let b1 = BoxF::new(tl.ll.x, b0.ur.y, hl.ur.x, b0.ur.y + stepy);
            let b = hl;
            let b2 = BoxF::new((-fi).mul_add(stepx, b.ll.x), b.ur.y, b.ur.x, b1.ll.y);
            for bx in &tend.boxes {
                add_box(&mut p, *bx);
            }
            for bx in [b0, b1, b2] {
                add_box(&mut p, bx);
            }
            for bx in hend.boxes.iter().rev() {
                add_box(&mut p, *bx);
            }
            let Some(ps) = self
                .route(&mut p, et == EdgeType::Spline)
                .filter(|v| !v.is_empty())
            else {
                return Ok(());
            };
            let h = self.edges[e].head;
            clip_and_install(self, e, h, &ps);
            p.boxes.clear();
        }
        Ok(())
    }

    // ------------------------------------------------------------- regular edges

    /// `makeLineEdge(g, fe, points, &hn)`: returns the points and end node, or
    /// `None` when the edge spans adjacent ranks.
    fn make_line_edge(&self, fe: E) -> Option<(Vec<Point>, N)> {
        let mut e = fe;
        while self.edges[e].edge_type != NORMAL {
            e = self.edges[e].to_orig.unwrap();
        }
        let (hn, tn) = (self.edges[e].head, self.edges[e].tail);
        let delr = (self.nodes[hn].rank - self.nodes[tn].rank).abs();
        if delr == 1 || (delr == 2 && self.graphs[0].has_labels & EDGE_LABEL != 0) {
            return None;
        }
        let ed = &self.edges[e];
        let (hp, startp, endp) = if self.edges[fe].tail == ed.tail {
            (
                hn,
                add(self.nodes[tn].coord, ed.tail_port.p),
                add(self.nodes[hn].coord, ed.head_port.p),
            )
        } else {
            (
                tn,
                add(self.nodes[hn].coord, ed.head_port.p),
                add(self.nodes[tn].coord, ed.tail_port.p),
            )
        };
        if let Some(l) = ed.label {
            let dimen = self.labels[l].dimen;
            let (width, height) = if self.graphs[0].flip() {
                (dimen.y, dimen.x)
            } else {
                (dimen.x, dimen.y)
            };
            let mut lp = self.labels[l].pos;
            if left_of(endp, startp, lp) {
                lp.x += width / 2.0;
                lp.y -= height / 2.0;
            } else {
                lp.x -= width / 2.0;
                lp.y += height / 2.0;
            }
            Some((vec![startp, startp, lp, lp, lp, endp, endp], hp))
        } else {
            Some((vec![startp, startp, endp, endp], hp))
        }
    }

    /// `make_regular_edge(g, sp, P, edges, ind, cnt, et)`.
    ///
    /// Routing a spline or polyline, C indexes `GD_rank(g)[ND_rank(vn)]` for every node on
    /// the edge's chain (here, in `maximal_bbox` and in `rank_box`), so each must lie on one
    /// of the root's ranks. After `trouble in init_rank`, a node that mincross never placed
    /// keeps the x coordinate `set_xcoords` left in `ND_rank` (issue #114); C reads past
    /// `GD_rank` there, or has already crashed in mincross's `furthestnode`. The port fails
    /// the layout. An edge `makeLineEdge` draws as a straight line reads no rank and is not
    /// checked; the edges it declines are routed, and checked, like splines.
    fn make_regular_edge(
        &mut self,
        sp: &mut SplineInfoT,
        edges: &[E],
        et: EdgeType,
    ) -> Result<(), String> {
        self.with_ends(
            |s| &mut s.regular,
            |d, ends| d.make_regular_edge_with(sp, edges, et, ends),
        )
    }

    fn make_regular_edge_with(
        &mut self,
        sp: &mut SplineInfoT,
        edges: &[E],
        et: EdgeType,
        (tend, hend): (&mut PathEnd, &mut PathEnd),
    ) -> Result<(), String> {
        let cnt = edges.len();
        let mut e = edges[0];
        let mut hackflag = false;
        let mut fwdedgeb: Option<E> = None;
        let (et0, eh0) = (self.edges[e].tail, self.edges[e].head);
        if (self.nodes[et0].rank - self.nodes[eh0].rank).abs() > 1 {
            let a = self.copy_edge(e);
            if self.edges[e].tree_index & BWDEDGE != 0 {
                fwdedgeb = Some(self.makefwdedge(e));
                self.edges[a].tail = eh0;
                self.edges[a].tail_port = self.edges[e].head_port.clone();
            } else {
                fwdedgeb = Some(self.copy_edge(e));
                self.edges[a].tail = et0;
            }
            let mut le = self.getmainedge(e);
            while let Some(v) = self.edges[le].to_virt {
                le = v;
            }
            // With `concentrate=true`, `mergevirtual`'s up pass deletes a merged node and
            // its edges, but an edge routed alone here (a cluster edge with other ports that
            // shares a chain, `interclrep`) may still lead into the deleted node. C then
            // walks from a node with no edges and segfaults in `straight_len`. The port
            // follows the node it was merged into, as the chain's own edges do.
            let mut h = self.edges[le].head;
            while let Some(l) = self.nodes.get(h).and_then(|nd| nd.conc_merged_into) {
                h = l;
            }
            self.edges[a].head = h;
            self.edges[a].head_port.defined = false;
            self.edges[a].edge_type = VIRTUAL;
            self.edges[a].head_port.p = Point::new(0.0, 0.0);
            self.edges[a].to_orig = Some(e);
            e = a;
            hackflag = true;
        } else if self.edges[e].tree_index & BWDEDGE != 0 {
            e = self.makefwdedge(e);
        }
        let fe = e;
        let has_lbl = self.graphs[0].has_labels & EDGE_LABEL != 0;

        let mut pointfs: Vec<Point> = Vec::new();
        let mut hn;
        if let Some((pts, h)) = (et == EdgeType::Line)
            .then(|| self.make_line_edge(fe))
            .flatten()
        {
            pointfs = pts;
            hn = h;
        } else {
            self.check_chain_ranks(e)?;
            let is_spline = et == EdgeType::Spline;
            let mut p = Path::default();
            let mut boxes: Vec<BoxF> = Vec::new();
            let mut segfirst = e;
            let mut tn = self.edges[e].tail;
            hn = self.edges[e].head;
            let mut b = self.maximal_bbox(sp, tn, None, Some(e));
            tend.nb = b;
            let m = self.spline_merge_p(tn);
            let kept = stale_boxes(tend);
            beginpath(self, &mut p, e, REGULAREDGE, tend, m, kept);
            let last = *tend.boxes.last().unwrap();
            b.ur.y = last.ur.y;
            b.ll.y = last.ll.y;
            let b = makeregularend(
                b,
                BOTTOM,
                self.nodes[tn].coord.y - self.rk(0, self.nodes[tn].rank).ht1,
            );
            push_end_box(tend, b)?;
            let mut smode = false;
            let mut si = false;
            let mut sl = 0;
            while self.nodes[hn].node_type == VIRTUAL && !self.spline_merge_p(hn) {
                boxes.push(self.rank_box(sp, self.nodes[tn].rank));
                if !smode && {
                    sl = self.straight_len(hn);
                    sl >= if has_lbl { 4 + 1 } else { 2 + 1 }
                } {
                    smode = true;
                    si = true;
                    sl -= 2;
                }
                let out0 = self.nodes[hn].out[0];
                if !smode || si {
                    si = false;
                    boxes.push(self.maximal_bbox(sp, hn, Some(e), Some(out0)));
                    e = out0;
                    tn = self.edges[e].tail;
                    hn = self.edges[e].head;
                    continue;
                }
                hend.nb = self.maximal_bbox(sp, hn, Some(e), Some(out0));
                let m = self.spline_merge_p(self.edges[e].head);
                let kept = stale_boxes(hend);
                endpath(self, &mut p, e, REGULAREDGE, hend, m, kept);
                let b = makeregularend(
                    *hend.boxes.last().unwrap(),
                    TOP,
                    self.nodes[hn].coord.y + self.rk(0, self.nodes[hn].rank).ht2,
                );
                push_end_box(hend, b)?;
                p.end.theta = PI / 2.0;
                p.end.constrained = true;
                self.completeregularpath(&mut p, segfirst, e, tend, hend, &boxes);
                let ps = if is_spline {
                    self.route(&mut p, true)
                } else {
                    self.route(&mut p, false).map(|mut ps| {
                        if et == EdgeType::Line && ps.len() > 4 {
                            let last = ps[ps.len() - 1];
                            ps[1] = ps[0];
                            ps[2] = last;
                            ps[3] = last;
                            ps.truncate(4);
                        }
                        ps
                    })
                };
                let Some(ps) = ps.filter(|v| !v.is_empty()) else {
                    return Ok(());
                };
                pointfs.extend_from_slice(&ps);
                let out0 = self.nodes[hn].out[0];
                e = self.straight_path(out0, sl, &mut pointfs);
                self.recover_slack(segfirst, &p);
                segfirst = e;
                tn = self.edges[e].tail;
                hn = self.edges[e].head;
                boxes.clear();
                let in0 = self.nodes[tn].in_[0];
                tend.nb = self.maximal_bbox(sp, tn, Some(in0), Some(e));
                let m = self.spline_merge_p(tn);
                let kept = stale_boxes(tend);
                beginpath(self, &mut p, e, REGULAREDGE, tend, m, kept);
                let b = makeregularend(
                    *tend.boxes.last().unwrap(),
                    BOTTOM,
                    self.nodes[tn].coord.y - self.rk(0, self.nodes[tn].rank).ht1,
                );
                push_end_box(tend, b)?;
                p.start.theta = -PI / 2.0;
                p.start.constrained = true;
                smode = false;
            }
            boxes.push(self.rank_box(sp, self.nodes[tn].rank));
            let mut b = self.maximal_bbox(sp, hn, Some(e), None);
            hend.nb = b;
            let ee = if hackflag { fwdedgeb.unwrap() } else { e };
            let mut m = self.spline_merge_p(self.edges[e].head);
            // C (`dotsplines.c:1845`) takes the merge flag from `aghead(e)`, where the
            // chain walk stopped, but with `hackflag` hands `endpath` the original
            // edge, whose head is the real node further on. The two differ when the
            // edge shares a chain (cluster edges with other ports, `interclrep`) that
            // `dot_concentrate` merged: the walk stops at the concentrator, and
            // `conc_slope` runs on the real head. With edges on both sides of that
            // node C gets a slope and draws, and so does the port. With one side
            // empty C dereferences NULL in `conc_slope` (#234); the port uses the
            // real head's own flag, which is false, as for an unconcentrated end.
            if m && hackflag {
                let h = self.edges.get(ee).and_then(|ed| self.nodes.get(ed.head));
                if h.is_none_or(|h| h.in_.is_empty() || h.out.is_empty()) {
                    m = false;
                }
            }
            let kept = stale_boxes(hend);
            endpath(self, &mut p, ee, REGULAREDGE, hend, m, kept);
            let last = *hend.boxes.last().unwrap();
            b.ur.y = last.ur.y;
            b.ll.y = last.ll.y;
            let b = makeregularend(
                b,
                TOP,
                self.nodes[hn].coord.y + self.rk(0, self.nodes[hn].rank).ht2,
            );
            push_end_box(hend, b)?;
            self.completeregularpath(&mut p, segfirst, e, tend, hend, &boxes);
            let ps = self.route(&mut p, is_spline).map(|mut ps| {
                if et == EdgeType::Line && ps.len() > 4 {
                    let last = ps[ps.len() - 1];
                    ps[1] = ps[0];
                    ps[2] = last;
                    ps[3] = last;
                    ps.truncate(4);
                }
                ps
            });
            let Some(ps) = ps.filter(|v| !v.is_empty()) else {
                return Ok(());
            };
            pointfs.extend_from_slice(&ps);
            self.recover_slack(segfirst, &p);
            hn = if hackflag {
                self.edges[fwdedgeb.unwrap()].head
            } else {
                self.edges[e].head
            };
        }

        // make copies of the spline points, one per multi-edge
        if cnt == 1 {
            clip_and_install(self, fe, hn, &pointfs);
            return Ok(());
        }
        let dx = sp.multisep * (cnt - 1) as f64 / 2.0;
        let np = pointfs.len();
        for p in &mut pointfs[1..np - 1] {
            p.x -= dx;
        }
        clip_and_install(self, fe, hn, &pointfs.clone());
        for &ej in &edges[1..] {
            let mut e = ej;
            if self.edges[e].tree_index & BWDEDGE != 0 {
                e = self.makefwdedge(e);
            }
            for p in &mut pointfs[1..np - 1] {
                p.x += sp.multisep;
            }
            let h = self.edges[e].head;
            clip_and_install(self, e, h, &pointfs.clone());
        }
        Ok(())
    }

    /// The nodes `make_regular_edge` routes from `e` (its tail, then heads along `ND_out`
    /// while virtual and not a spline merge point) must have a rank in the root's
    /// `minrank..=maxrank`; see [`Self::make_regular_edge`].
    fn check_chain_ranks(&self, e: E) -> Result<(), String> {
        let (minr, maxr) = (self.graphs[0].minrank, self.graphs[0].maxrank);
        let mut n = self.edges[e].tail;
        let mut next = Some(self.edges[e].head);
        loop {
            let r = self.nodes[n].rank;
            if r < minr || r > maxr {
                let mut orig = e;
                while let Some(o) = self.edges[orig].to_orig {
                    orig = o;
                }
                let (t, h) = (self.edges[orig].tail, self.edges[orig].head);
                return Err(format!(
                    "make_regular_edge: edge {} -> {} reaches a node on rank {r}, outside the \
                     ranks {minr} to {maxr}",
                    self.nodes[t].name, self.nodes[h].name
                ));
            }
            let Some(h) = next else {
                return Ok(());
            };
            n = h;
            next = if self.nodes[h].node_type == VIRTUAL && !self.spline_merge_p(h) {
                self.nodes[h].out.first().map(|&o| self.edges[o].head)
            } else {
                None
            };
        }
    }

    /// `completeregularpath(P, first, last, tendp, hendp, boxes, boxn, flag)`.
    fn completeregularpath(
        &self,
        p: &mut Path,
        first: E,
        last: E,
        tendp: &PathEnd,
        hendp: &PathEnd,
        boxes: &[BoxF],
    ) {
        for f in [self.top_bound(first, -1), self.top_bound(first, 1)]
            .into_iter()
            .chain([self.bot_bound(last, -1), self.bot_bound(last, 1)])
            .flatten()
        {
            if getsplinepoints(self, f).is_none() {
                return;
            }
        }
        for b in &tendp.boxes {
            add_box(p, *b);
        }
        let fb = p.boxes.len() + 1;
        let lb = (fb + boxes.len()).wrapping_sub(3);
        for b in boxes {
            add_box(p, *b);
        }
        for b in hendp.boxes.iter().rev() {
            add_box(p, *b);
        }
        adjustregularpath(p, fb, lb);
    }

    /// `rank_box(sp, g, r)`.
    fn rank_box(&self, sp: &mut SplineInfoT, r: i32) -> BoxF {
        let mut b = sp.rank_box[r as usize];
        if b.ll.x == b.ur.x {
            let left0 = self.rv(0, r, 0).unwrap();
            let left1 = self.rv(0, r + 1, 0).unwrap();
            b.ll.x = sp.left_bound;
            b.ll.y = self.nodes[left1].coord.y + self.rk(0, r + 1).ht2;
            b.ur.x = sp.right_bound;
            b.ur.y = self.nodes[left0].coord.y - self.rk(0, r).ht1;
            sp.rank_box[r as usize] = b;
        }
        b
    }

    /// `straight_len(n)`: count of vertically aligned edges starting at `n`.
    fn straight_len(&self, n: N) -> i32 {
        let mut cnt = 0;
        let mut v = n;
        loop {
            v = self.edges[self.nodes[v].out[0]].head;
            let nd = &self.nodes[v];
            if nd.node_type != VIRTUAL {
                break;
            }
            if nd.out.len() != 1 || nd.in_.len() != 1 {
                break;
            }
            if nd.coord.x != self.nodes[n].coord.x {
                break;
            }
            cnt += 1;
        }
        cnt
    }

    /// `straight_path(e, cnt, plist)`.
    fn straight_path(&self, e: E, cnt: i32, plist: &mut Vec<Point>) -> E {
        let mut f = e;
        for _ in 0..cnt {
            f = self.nodes[self.edges[f].head].out[0];
        }
        let last = *plist.last().unwrap();
        plist.push(last);
        plist.push(last);
        f
    }

    /// `recover_slack(e, p)`.
    fn recover_slack(&mut self, e: E, p: &Path) {
        let mut b = 0usize;
        let mut vn = self.edges[e].head;
        while self.nodes[vn].node_type == VIRTUAL && !self.spline_merge_p(vn) {
            let y = self.nodes[vn].coord.y;
            while b < p.boxes.len() && p.boxes[b].ll.y > y {
                b += 1;
            }
            if b >= p.boxes.len() {
                break;
            }
            if p.boxes[b].ur.y >= y {
                let bx = p.boxes[b];
                if self.nodes[vn].label.is_some() {
                    let rw = self.nodes[vn].rw;
                    self.resize_vn(vn, bx.ll.x, bx.ur.x, bx.ur.x + rw);
                } else {
                    self.resize_vn(vn, bx.ll.x, (bx.ll.x + bx.ur.x) / 2.0, bx.ur.x);
                }
            }
            vn = self.edges[self.nodes[vn].out[0]].head;
        }
    }

    /// `resize_vn(vn, lx, cx, rx)`.
    fn resize_vn(&mut self, vn: N, lx: f64, cx: f64, rx: f64) {
        let nd = &mut self.nodes[vn];
        nd.coord.x = cx;
        nd.lw = cx - lx;
        nd.rw = rx - cx;
    }

    fn has_spl_or_orig(&self, f: E) -> bool {
        self.edges[f].spl.is_some()
            || self.edges[f]
                .to_orig
                .is_some_and(|o| self.edges[o].spl.is_some())
    }

    /// `top_bound(e, side)`.
    fn top_bound(&self, e: E, side: i32) -> Option<E> {
        let mut ans: Option<E> = None;
        let eh = self.nodes[self.edges[e].head].order;
        for &f in &self.nodes[self.edges[e].tail].out {
            let fh = self.nodes[self.edges[f].head].order;
            if side * (fh - eh) <= 0 {
                continue;
            }
            if !self.has_spl_or_orig(f) {
                continue;
            }
            if ans.is_none_or(|a| side * (self.nodes[self.edges[a].head].order - fh) > 0) {
                ans = Some(f);
            }
        }
        ans
    }

    /// `bot_bound(e, side)`.
    fn bot_bound(&self, e: E, side: i32) -> Option<E> {
        let mut ans: Option<E> = None;
        let et = self.nodes[self.edges[e].tail].order;
        for &f in &self.nodes[self.edges[e].head].in_ {
            let ft = self.nodes[self.edges[f].tail].order;
            if side * (ft - et) <= 0 {
                continue;
            }
            if !self.has_spl_or_orig(f) {
                continue;
            }
            if ans.is_none_or(|a| side * (self.nodes[self.edges[a].tail].order - ft) > 0) {
                ans = Some(f);
            }
        }
        ans
    }

    /// `cl_vninside(cl, n)`.
    fn cl_vninside(&self, cl: G, n: N) -> bool {
        let bb = self.graphs[cl].bb;
        let c = self.nodes[n].coord;
        bb.ll.x <= c.x && c.x <= bb.ur.x && bb.ll.y <= c.y && c.y <= bb.ur.y
    }

    /// `REAL_CLUSTER(n)`.
    fn real_cluster(&self, n: N) -> Option<G> {
        self.nodes[n].clust.filter(|&c| c != 0)
    }

    /// `cl_bound(g, n, adj)`: the cluster of `adj` that interferes with `n`.
    fn cl_bound(&self, n: N, adj: N) -> Option<G> {
        let (tcl, hcl) = if self.nodes[n].node_type == NORMAL {
            (self.nodes[n].clust, self.nodes[n].clust)
        } else {
            let orig = self.edges[self.nodes[n].out[0]].to_orig.unwrap();
            (
                self.nodes[self.edges[orig].tail].clust,
                self.nodes[self.edges[orig].head].clust,
            )
        };
        if self.nodes[adj].node_type == NORMAL {
            let cl = self.real_cluster(adj);
            if let Some(c) = cl
                && Some(c) != tcl
                && Some(c) != hcl
            {
                return Some(c);
            }
        } else {
            let orig = self.edges[self.nodes[adj].out[0]].to_orig.unwrap();
            let cl = self.real_cluster(self.edges[orig].tail);
            if let Some(c) = cl
                && Some(c) != tcl
                && Some(c) != hcl
                && self.cl_vninside(c, adj)
            {
                return Some(c);
            }
            let cl = self.real_cluster(self.edges[orig].head);
            if let Some(c) = cl
                && Some(c) != tcl
                && Some(c) != hcl
                && self.cl_vninside(c, adj)
            {
                return Some(c);
            }
        }
        None
    }

    /// `maximal_bbox(g, sp, vn, ie, oe)`.
    fn maximal_bbox(&self, sp: &SplineInfoT, vn: N, ie: Option<E>, oe: Option<E>) -> BoxF {
        let nodesep = f64::from(self.graphs[0].nodesep);
        let v = &self.nodes[vn];
        let mut rv = BoxF::default();
        let mut b = v.coord.x - v.lw - FUDGE;
        if let Some(left) = self.splines_neighbor(vn, ie, oe, -1) {
            let nb = if let Some(lc) = self.cl_bound(vn, left) {
                self.graphs[lc].bb.ur.x + sp.splinesep
            } else {
                let l = &self.nodes[left];
                let mut nb = l.coord.x + l.mval;
                if l.node_type == NORMAL {
                    nb += nodesep / 2.0;
                } else {
                    nb += sp.splinesep;
                }
                nb
            };
            if nb < b {
                b = nb;
            }
            rv.ll.x = b.round();
        } else {
            rv.ll.x = b.round().min(sp.left_bound);
        }
        let has_vlabel = v.node_type == VIRTUAL && v.label.is_some();
        b = if has_vlabel {
            v.coord.x + 10.0
        } else {
            v.coord.x + v.rw + FUDGE
        };
        if let Some(right) = self.splines_neighbor(vn, ie, oe, 1) {
            let nb = if let Some(rc) = self.cl_bound(vn, right) {
                self.graphs[rc].bb.ll.x - sp.splinesep
            } else {
                let r = &self.nodes[right];
                let mut nb = r.coord.x - r.lw;
                if r.node_type == NORMAL {
                    nb -= nodesep / 2.0;
                } else {
                    nb -= sp.splinesep;
                }
                nb
            };
            if nb > b {
                b = nb;
            }
            rv.ur.x = b.round();
        } else {
            rv.ur.x = b.round().max(sp.right_bound);
        }
        if has_vlabel {
            rv.ur.x -= v.rw;
            if rv.ur.x < rv.ll.x {
                rv.ur.x = v.coord.x;
            }
        }
        let rk = self.rk(0, v.rank);
        rv.ll.y = v.coord.y - rk.ht1;
        rv.ur.y = v.coord.y + rk.ht2;
        rv
    }

    /// `neighbor(g, vn, ie, oe, dir)` (dotsplines.c).
    fn splines_neighbor(&self, vn: N, ie: Option<E>, oe: Option<E>, dir: i32) -> Option<N> {
        let r = self.nodes[vn].rank;
        let rn = self.rank_n(0, r);
        let mut i = self.nodes[vn].order + dir;
        while i >= 0 && i < rn {
            let n = self.rv(0, r, i).unwrap();
            let nd = &self.nodes[n];
            if nd.node_type == VIRTUAL && nd.label.is_some() {
                return Some(n);
            }
            if nd.node_type == NORMAL {
                return Some(n);
            }
            if !self.pathscross(n, vn, ie, oe) {
                return Some(n);
            }
            i += dir;
        }
        None
    }

    /// `pathscross(n0, n1, ie1, oe1)`.
    fn pathscross(&self, n0: N, n1: N, ie1: Option<E>, oe1: Option<E>) -> bool {
        let order = self.nodes[n0].order > self.nodes[n1].order;
        if self.nodes[n0].out.len() != 1 && self.nodes[n1].out.len() != 1 {
            return false;
        }
        if self.nodes[n0].out.len() == 1
            && let Some(mut e1) = oe1
        {
            let mut e0 = self.nodes[n0].out[0];
            for _ in 0..2 {
                let na = self.edges[e0].head;
                let nb = self.edges[e1].head;
                if na == nb {
                    break;
                }
                if order != (self.nodes[na].order > self.nodes[nb].order) {
                    return true;
                }
                if self.nodes[na].out.len() != 1 || self.nodes[na].node_type == NORMAL {
                    break;
                }
                e0 = self.nodes[na].out[0];
                if self.nodes[nb].out.len() != 1 || self.nodes[nb].node_type == NORMAL {
                    break;
                }
                e1 = self.nodes[nb].out[0];
            }
        }
        if self.nodes[n0].in_.len() == 1
            && let Some(mut e1) = ie1
        {
            let mut e0 = self.nodes[n0].in_[0];
            for _ in 0..2 {
                let na = self.edges[e0].tail;
                let nb = self.edges[e1].tail;
                if na == nb {
                    break;
                }
                if order != (self.nodes[na].order > self.nodes[nb].order) {
                    return true;
                }
                if self.nodes[na].in_.len() != 1 || self.nodes[na].node_type == NORMAL {
                    break;
                }
                e0 = self.nodes[na].in_[0];
                if self.nodes[nb].in_.len() != 1 || self.nodes[nb].node_type == NORMAL {
                    break;
                }
                e1 = self.nodes[nb].in_[0];
            }
        }
        false
    }
}

/// `adjustregularpath(P, fb, lb)`: make sure the path is wide enough. `fb` and
/// `lb` are C `size_t`s and wrap the same way.
///
/// `fb` and `lb` count every box `completeregularpath` was given, but `add_box` drops the
/// empty ones, which huge sizes or scales make (#270: `quantum` or `ratio` at 1e300). C's
/// first loop then goes past `P->nbox` into the preallocated `P->boxes`. Each pass reads and
/// writes only `P->boxes[i]`, and the next path's `add_box` overwrites those slots before
/// anything reads them, so they cannot change C's output. The loop stops at the last box.
fn adjustregularpath(p: &mut Path, fb: usize, lb: usize) {
    let end = lb.wrapping_add(1).min(p.boxes.len());
    let mut i = fb.wrapping_sub(1);
    while i < end {
        let bp1 = &mut p.boxes[i];
        if i.wrapping_sub(fb).is_multiple_of(2) {
            if bp1.ll.x >= bp1.ur.x {
                let x = (bp1.ll.x + bp1.ur.x) / 2.0;
                bp1.ll.x = x - HALFMINW;
                bp1.ur.x = x + HALFMINW;
            }
        } else if bp1.ll.x + MINW > bp1.ur.x {
            let x = (bp1.ll.x + bp1.ur.x) / 2.0;
            bp1.ll.x = x - HALFMINW;
            bp1.ur.x = x + HALFMINW;
        }
        i = i.wrapping_add(1);
    }
    let nbox = p.boxes.len();
    let mut i = 0usize;
    while i + 1 < nbox {
        let (lo, hi) = p.boxes.split_at_mut(i + 1);
        let bp1 = &mut lo[i];
        let bp2 = &mut hi[0];
        if i >= fb && i <= lb && (i - fb).is_multiple_of(2) {
            if bp1.ll.x + MINW > bp2.ur.x {
                bp2.ur.x = bp1.ll.x + MINW;
            }
            if bp1.ur.x - MINW < bp2.ll.x {
                bp2.ll.x = bp1.ur.x - MINW;
            }
        } else if i + 1 >= fb && i < lb && (i + 1 - fb).is_multiple_of(2) {
            if bp1.ll.x + MINW > bp2.ur.x {
                bp1.ll.x = bp2.ur.x - MINW;
            }
            if bp1.ur.x - MINW < bp2.ll.x {
                bp1.ur.x = bp2.ll.x + MINW;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #360: ends kept from earlier edges may fill C's 20 boxes. An empty box is not
    /// stored, as in C; a box C would store past the array fails the layout.
    #[test]
    fn a_full_path_end_takes_no_more_boxes() {
        let b = BoxF::new(0.0, 0.0, 1.0, 1.0);
        let mut endp = PathEnd {
            boxes: vec![b; PATHEND_MAX_BOXES - 1],
            ..PathEnd::default()
        };
        assert!(push_end_box(&mut endp, b).is_ok());
        assert_eq!(endp.boxes.len(), PATHEND_MAX_BOXES);
        assert!(push_end_box(&mut endp, BoxF::new(0.0, 0.0, 0.0, 1.0)).is_ok());
        assert_eq!(endp.boxes.len(), PATHEND_MAX_BOXES);
        assert!(push_end_box(&mut endp, b).is_err());
        assert_eq!(endp.boxes.len(), PATHEND_MAX_BOXES);
    }

    fn dict(names: &[&str]) -> std::collections::BTreeMap<String, graphviz_pure_core::AttrSym> {
        names
            .iter()
            .enumerate()
            .map(|(id, n)| {
                let sym = graphviz_pure_core::AttrSym {
                    id,
                    proto: *n == "label",
                    ..Default::default()
                };
                (n.to_string(), sym)
            })
            .collect()
    }

    fn attrs(kv: &[(&str, &str)]) -> graphviz_pure_core::AttrMap {
        graphviz_pure_core::AttrMap::from_plain(
            kv.iter().map(|(k, v)| (k.to_string(), v.to_string())),
        )
    }

    /// #84: the main graph declares weight, constraint, headclip, headport, fontcolor and
    /// penwidth in that order. auxg declares them in name order, then `tailport`, so
    /// E_penwidth (id 5) reads the aux `weight`, which is 10000 on the heavy edge. That the
    /// heavy weight is set before these reads is checked end to end in `cli.rs`
    /// (`flat_adjacent_aux_graph_reads_symbols_by_main_id_like_c`).
    #[test]
    fn stale_edge_symbols_read_the_aux_attribute_with_their_id_after_the_heavy_weight() {
        let main = graphviz_pure_core::AttrDicts {
            edge: dict(&[
                "weight",
                "constraint",
                "headclip",
                "headport",
                "fontcolor",
                "penwidth",
            ]),
            ..Default::default()
        };
        let syms = AuxSyms::new(&main);
        assert_eq!(
            syms.edge,
            [
                "constraint",
                "fontcolor",
                "headclip",
                "headport",
                "penwidth",
                "weight",
                "tailport"
            ]
        );
        let mut heavy = attrs(&[("weight", "10000"), ("constraint", "false")]);
        syms.fix_edge(&mut heavy);
        assert_eq!(heavy.get("penwidth").map(String::as_str), Some("10000"));
        // no main `arrowsize`: E_arrowsz is NULL
        assert_eq!(heavy.get("arrowsize"), None);
        // a NULLed symbol is not read by name, but a stale one reads its value
        assert_eq!(heavy.get("constraint"), None);
        let mut e = attrs(&[("weight", "5"), ("fontcolor", "blue"), ("penwidth", "3.99")]);
        syms.fix_edge(&mut e);
        assert_eq!(e.get("penwidth").map(String::as_str), Some("5"));
        assert_eq!(e.get("fontcolor"), None);
    }

    /// #376 review: auxg's graph dictionary holds only the ProtoGraph's symbols, a ProtoGraph
    /// `rank` keeping its id with `""`; the node `label` is the ProtoGraph's `\N` even where
    /// the main dictionary lacks it; agnewsym's names are plain and unprinted, while an HTML
    /// default stays one.
    #[test]
    fn aux_graph_symbols_are_the_protographs_and_agnewsyms() {
        let mut graph = dict(&["rank", "ordering", "label"]);
        for (k, v) in [("rank", "same"), ("ordering", "out")] {
            let s = graph.get_mut(k).unwrap();
            (s.proto, s.defval) = (true, v.into());
        }
        graph.get_mut("label").unwrap().proto = false;
        let mut node = dict(&["shape"]);
        let shape = node.get_mut("shape").unwrap();
        (shape.print, shape.html, shape.html_defval) = (true, true, true);
        shape.defval = "box".to_string();
        let main = graphviz_pure_core::AttrDicts {
            graph,
            node,
            ..Default::default()
        };
        let d = AuxSyms::new(&main).dicts();
        let g = |k: &str| d.graph.get(k).map(|s| (s.id, s.defval.as_str()));
        assert_eq!(g("rank"), Some((0, "")));
        assert_eq!(g("ordering"), Some((1, "out")));
        assert_eq!(g("label"), None);
        let label = &d.node["label"];
        assert_eq!(
            (label.id, label.defval.as_str(), label.proto),
            (0, "\\N", true)
        );
        let shape = &d.node["shape"];
        assert_eq!(
            (
                shape.id,
                shape.defval.as_str(),
                shape.print,
                shape.html,
                shape.html_defval
            ),
            (1, "box", false, false, true)
        );
    }

    /// #376 review: setState NULLs some globals although cloneGraph declared them on auxg,
    /// so dot reads them as undeclared there, and as declared on the main graph.
    #[test]
    fn symbols_set_state_nulls_read_as_undeclared_on_the_aux_graph() {
        let mut g =
            graphviz_pure_core::Graph::new("auxg", graphviz_pure_core::GraphKind::Directed, false);
        g.dicts = vec![graphviz_pure_core::AttrDicts {
            node: dict(&["label", "style", "shape", "group"]),
            edge: dict(&["headlabel", "taillabel", "minlen", "color", "xlabel"]),
            ..Default::default()
        }];
        let mut d = Dot::new(&g);
        assert!(d.edge_attr_declared("headlabel") && d.node_attr_declared("style"));
        d.null_syms = (&AuxSyms::NULL_NODE, &AuxSyms::NULL_EDGE);
        for k in ["headlabel", "taillabel", "minlen", "xlabel"] {
            assert!(!d.edge_attr_declared(k), "{k}");
        }
        assert!(!d.node_attr_declared("style") && !d.node_attr_declared("group"));
        assert!(d.edge_attr_declared("color") && d.node_attr_declared("shape"));
    }

    /// #376: cloneGraph declares every node and edge symbol of the main root on auxg, with
    /// its default, including one only a subgraph's defaults declared (`samehead` here,
    /// default `""`), then `headport`/`tailport`/`weight`; auxg's graph dictionary holds
    /// the ProtoGraph's symbols and `rank`.
    #[test]
    fn aux_graph_declares_every_main_symbol_as_clone_graph_does() {
        let mut graph = dict(&["size", "label", "rankdir"]);
        graph.get_mut("size").unwrap().proto = true;
        graph.get_mut("label").unwrap().proto = false;
        let mut node = dict(&["label", "shape"]);
        node.get_mut("label").unwrap().defval = "\\N".to_string();
        node.get_mut("shape").unwrap().defval = "box".to_string();
        let main = graphviz_pure_core::AttrDicts {
            graph,
            node,
            edge: dict(&["samehead", "color"]),
        };
        let d = AuxSyms::new(&main).dicts();
        fn ids(
            m: &std::collections::BTreeMap<String, graphviz_pure_core::AttrSym>,
        ) -> Vec<(usize, &str, &str)> {
            let mut v: Vec<(usize, &str, &str)> = m
                .iter()
                .map(|(k, s)| (s.id, k.as_str(), s.defval.as_str()))
                .collect();
            v.sort();
            v
        }
        assert_eq!(ids(&d.graph), [(0, "size", ""), (1, "rank", "")]);
        assert_eq!(ids(&d.node), [(0, "label", "\\N"), (1, "shape", "box")]);
        assert_eq!(
            ids(&d.edge),
            [
                (0, "color", ""),
                (1, "samehead", ""),
                (2, "headport", ""),
                (3, "tailport", ""),
                (4, "weight", "")
            ]
        );
    }

    /// `-N`/`-E` declare their attributes on cgraph's ProtoGraph, which auxg copies with the
    /// ids the main graph gave them; the main graph's other attributes follow in name order.
    #[test]
    fn protograph_symbols_keep_their_main_ids_in_the_aux_graph() {
        let mut edge = dict(&["zzz", "penwidth", "arrowsize", "color"]);
        for k in ["zzz", "penwidth"] {
            edge.get_mut(k).unwrap().proto = true;
        }
        let main = graphviz_pure_core::AttrDicts {
            node: dict(&["label", "width"]),
            edge,
            ..Default::default()
        };
        let syms = AuxSyms::new(&main);
        assert_eq!(syms.node, ["label", "width"]);
        assert_eq!(
            syms.edge,
            [
                "zzz",
                "penwidth",
                "arrowsize",
                "color",
                "headport",
                "tailport",
                "weight"
            ]
        );
        let mut e = attrs(&[("penwidth", "2"), ("arrowsize", "3"), ("zzz", "1")]);
        syms.fix_edge(&mut e);
        assert_eq!(e.get("penwidth").map(String::as_str), Some("2"));
        assert_eq!(e.get("arrowsize").map(String::as_str), Some("3"));
    }

    /// #113: `fixed` declared after the node was made: cgraph gave the node "", not the
    /// later default, so N_fixed (looked up as "fixed") does not fix its aux copy's size.
    /// A stale symbol whose aux attribute the node lacks reads "" too.
    #[test]
    fn attributes_declared_after_the_node_read_empty_on_its_aux_copy() {
        let mut node = dict(&["label", "fixed", "penwidth"]);
        node.get_mut("fixed").unwrap().defval = "true".to_string();
        let main = graphviz_pure_core::AttrDicts {
            node,
            ..Default::default()
        };
        let syms = AuxSyms::new(&main);
        assert_eq!(syms.node, ["label", "fixed", "penwidth"]);
        let mut n = attrs(&[("label", "a long label here")]);
        syms.fix_node(&mut n);
        assert_eq!(n.get("fixedsize").map(String::as_str), Some(""));
        assert_eq!(n.get("penwidth").map(String::as_str), Some(""));
        let mut n = attrs(&[("label", "x"), ("fixed", "true"), ("fixedsize", "false")]);
        syms.fix_node(&mut n);
        assert_eq!(n.get("fixedsize").map(String::as_str), Some("true"));
    }

    /// #270: `add_box` dropped an empty box, so `fb` and `lb` reach past the boxes kept. C's
    /// first loop reads and writes slots past `P->nbox`, which nothing reads; the kept boxes
    /// are adjusted as C adjusts them.
    #[test]
    fn adjustregularpath_stops_at_the_boxes_add_box_kept() {
        let mut p = Path {
            boxes: vec![
                BoxF::new(0.0, 10.0, 20.0, 20.0),
                BoxF::new(5.0, 0.0, 5.0, 10.0),
            ],
            ..Path::default()
        };
        // Two path boxes after one tail box were expected: fb = 2, lb = 3.
        adjustregularpath(&mut p, 2, 3);
        assert_eq!(p.boxes.len(), 2);
        assert_eq!(p.boxes[1], BoxF::new(-3.0, 0.0, 13.0, 10.0));
        // Every box dropped: C's loop starts past the end.
        let mut p = Path::default();
        adjustregularpath(&mut p, 1, 0);
        assert!(p.boxes.is_empty());
    }
}
