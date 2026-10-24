//! The component-packing branch of `doDot` (lib/dotgen/dotinit.c): with `pack`
//! or `packmode` set, the graph is split by `cccomps` (components that keep
//! top-level clusters whole), each component is laid out by `dotLayout` as a
//! subgraph of the root (`initSubg`), and the layouts are packed with
//! `lib/pack` (`attachPos`, `packSubgraphs`, `resetCoord`,
//! `copyClusterInfo`).
//!
//! A component is laid out by a separate [`Dot`] over a standalone copy of the
//! component (`pack::cccomps::CComp::to_graph`); `Dot::comp` and
//! `Dot::root_input` give it the root-graph view C has through `agroot`. The
//! results (C's node and edge records, which the component shares with the
//! root) are then copied into the root's [`Dot`].

use super::Dot;
use super::types::*;
use crate::common::geom::{expandbb, expandbp};
use crate::common::splines::add_label_bb_dims;
use crate::common::types::EdgeType;
use crate::pack::cccomps::{CComp, CompGraph, cccomps};
use crate::pack::{
    PackGraph, PackInfo, PackMode, PkClust, PkGraph, get_pack, get_pack_mode_info, pack_graphs_in,
    shift_beziers,
};
use graphviz_pure_core::{BoundingBox, Graph, Point};

/// One packed component: its nodes, its edges (the root's edges with both
/// ends in it), top-level clusters (indices of [`PClust`]s) and `GD_bb` of the
/// component subgraph.
struct CompInfo {
    nodes: Vec<N>,
    edges: std::collections::HashSet<E>,
    clusters: Vec<usize>,
    bb: BoundingBox,
}

/// What `doDot` decided.
pub enum DoDot {
    /// Plain `dotLayout(g)`.
    Whole,
    /// Pack the components with these parameters.
    Pack(PackInfo),
}

impl<'a> Dot<'a> {
    /// The pack parameters of `doDot`, before `cccomps`.
    pub fn do_dot_mode(&self) -> DoDot {
        let mut pack = get_pack(self.agget(0, "pack"), -1, i64::from(CL_OFFSET));
        let mut pinfo = PackInfo::default();
        let mode = get_pack_mode_info(self.agget(0, "packmode"), PackMode::Undef, &mut pinfo);
        // getPackInfo(g, l_node, CL_OFFSET, &pinfo)
        crate::pack::pack_info(
            self.agget(0, "pack"),
            self.agget(0, "packmode"),
            PackMode::Node,
            i64::from(CL_OFFSET),
            &mut pinfo,
        );
        if mode == PackMode::Undef && pack < 0 {
            return DoDot::Whole;
        }
        if mode == PackMode::Undef {
            pinfo.mode = PackMode::Graph;
        } else if pack < 0 {
            pack = i64::from(CL_OFFSET);
        }
        pinfo.margin = pack as u32;
        pinfo.fixed = None;
        DoDot::Pack(pinfo)
    }

    /// The subgraphs `cccomps(g, &ncc, 0)` leaves in the root: for component
    /// `i`, `out = agsubg(g, "_cc_<i>", 1)` holding its nodes and induced
    /// edges (`unionNodes`, `graphviz_node_induce(out, NULL)`), with the
    /// root's subgraphs projected into it (`subGInduce`: `projectG` clones,
    /// `agcopyattr` of the original's values). `laid_out[i]` holds what component
    /// `i`'s own layout left in its projections, when it ran on its own. Returns
    /// the `_cc_<i>` graphs.
    fn attach_cc_subgraphs(
        &mut self,
        graph: &Graph,
        ccs: &[CComp],
        laid_out: &[Vec<Option<Membership>>],
    ) -> Vec<G> {
        // A new subgraph of the root takes the root dictionary's defaults. A
        // parsed graph always has dictionaries; a graph built without them has
        // no declared defaults, so its own values stand in.
        let defaults: graphviz_pure_core::AttrMap = match graph.dicts.first() {
            Some(dict) => graphviz_pure_core::AttrMap::from_plain(
                dict.graph
                    .iter()
                    .map(|(k, s)| (k.clone(), s.defval.clone())),
            ),
            None => graph.graph_attrs.clone(),
        };
        // The dot graph of each input subgraph, for the input's own `_cc_<i>`.
        let graph_of_sub: std::collections::HashMap<usize, G> = self
            .graphs
            .iter()
            .enumerate()
            .filter_map(|(g, gi)| gi.sub_index.map(|si| (si, g)))
            .collect();
        let mut out = Vec::with_capacity(ccs.len());
        for (i, cc) in ccs.iter().enumerate() {
            let set: std::collections::BTreeSet<N> = cc.nodes.iter().copied().collect();
            // Induced edges (`graphviz_node_induce(out, NULL)`). Components can share a
            // node of two non-nested clusters, so membership is tested per component.
            let edges = self.graphs[0]
                .edges
                .iter()
                .copied()
                .filter(|&e| set.contains(&self.edges[e].tail) && set.contains(&self.edges[e].head))
                .collect();
            // `agsubg(g, "_cc_<i>", 1)` returns the input's own subgraph of that name
            // (#335), which gains the component's nodes and edges and keeps its values.
            let user = cc.user.and_then(|u| graph_of_sub.get(&u).copied());
            let g = match user {
                Some(g) => {
                    if let Some(gi) = self.graphs.get_mut(g) {
                        gi.nodes.extend(set);
                        gi.edges.extend(edges);
                    }
                    g
                }
                None => {
                    let g = self.graphs.len();
                    self.graphs.push(GraphInfo {
                        name: format!("_cc_{i}"),
                        root: 0,
                        cg_parent: Some(0),
                        nodes: set,
                        edges,
                        attrs: defaults.clone(),
                        ..GraphInfo::default()
                    });
                    self.graphs[0].subgs.push(g);
                    g
                }
            };
            // Projected subgraphs, preorder (parents first).
            let mut ids: Vec<G> = Vec::with_capacity(cc.subgraphs.len());
            let laid_out = laid_out.get(i);
            for (k, p) in cc.subgraphs.iter().enumerate() {
                // The input's own `_cc_<i>` already holds its subgraphs.
                if user.is_some() && k < cc.user_subgraphs {
                    ids.push(G::MAX);
                    continue;
                }
                let parent = p.parent.map_or(g, |k| ids[k]);
                let s = self.graphs.len();
                let (nodes, edges) = match laid_out.and_then(|l| l.get(k)).and_then(Option::as_ref)
                {
                    Some(m) => (m.nodes.clone(), m.edges.clone()),
                    None => (
                        p.nodes.iter().copied().collect(),
                        p.edges.iter().copied().collect(),
                    ),
                };
                self.graphs.push(GraphInfo {
                    name: graph.subgraphs[p.orig].name.clone(),
                    root: 0,
                    cg_parent: Some(parent),
                    nodes,
                    edges,
                    attrs: graph.subgraphs[p.orig].graph_attrs.clone(),
                    proj_of: Some(p.orig),
                    ..GraphInfo::default()
                });
                self.graphs[parent].subgs.push(s);
                ids.push(s);
            }
            out.push(g);
        }
        out
    }

    /// `initSubg(sg, g)` + `dotLayout(sg)` state for a component of `root`.
    pub fn new_component(input: &'a Graph, root_input: &'a Graph, root: &Dot<'_>) -> Self {
        let mut c = Dot::new(input);
        c.root_input = root_input;
        c.comp = true;
        let (r, g) = (&root.graphs[0], &mut c.graphs[0]);
        g.rankdir = r.rankdir;
        g.nodesep = r.nodesep;
        g.ranksep = r.ranksep;
        // `GD_has_labels(g->root)`: the root's flags so far (its own label and
        // the components initialised before this one).
        g.has_labels = r.has_labels;
        c.root_bb = r.bb;
        c.cl_type = root.cl_type;
        c.concentrate = root.concentrate;
        c
    }

    fn copy_label(&mut self, from: &Dot<'_>, l: Option<L>) -> Option<L> {
        let l = l?;
        self.labels.push(from.labels[l].clone());
        Some(self.labels.len() - 1)
    }

    /// Copies a laid-out component's node and edge records into the root
    /// (they are shared objects in C) and its cluster records into `pcl` (the
    /// component's own cluster subgraphs). Fills each node's `ND_clust`.
    fn absorb(
        &mut self,
        c: &Dot<'_>,
        cg: &CompGraph,
        ci: usize,
        clust_of_sub: &[Option<G>],
        pcl: &mut Vec<PClust>,
        nclust: &mut [Option<PkClust>],
    ) -> CompInfo {
        // The component's cluster tree.
        fn copy_cluster(
            root: &mut Dot<'_>,
            c: &Dot<'_>,
            cg: &CompGraph,
            lg: G,
            clust_of_sub: &[Option<G>],
            pcl: &mut Vec<PClust>,
            local: &mut std::collections::HashMap<G, usize>,
        ) -> usize {
            let si = c.graphs[lg].sub_index.expect("cluster of the component");
            let orig = clust_of_sub[cg.subgraphs[si]].expect("original cluster");
            let label = root.copy_label(c, c.graphs[lg].label);
            let me = pcl.len();
            let s = &c.graphs[lg];
            pcl.push(PClust {
                orig,
                bb: s.bb,
                label,
                label_pos: s.label_pos,
                border: s.border,
                kids: Vec::new(),
                nodes: c
                    .nodes_of(lg)
                    .into_iter()
                    .filter_map(|n| c.nodes[n].real.map(|r| cg.nodes[r]))
                    .collect(),
            });
            local.insert(lg, me);
            let kids: Vec<usize> = c.graphs[lg]
                .clust
                .iter()
                .map(|&k| copy_cluster(root, c, cg, k, clust_of_sub, pcl, local))
                .collect();
            pcl[me].kids = kids;
            me
        }
        let mut local = std::collections::HashMap::new();
        let clusters = c.graphs[0]
            .clust
            .iter()
            .map(|&k| copy_cluster(self, c, cg, k, clust_of_sub, pcl, &mut local))
            .collect();
        for (li, &ri) in cg.nodes.iter().enumerate() {
            let s = &c.nodes[li];
            let label = self.copy_label(c, s.label);
            let xlabel = self.copy_label(c, s.xlabel);
            let d = &mut self.nodes[ri];
            d.coord = s.coord;
            d.width = s.width;
            d.height = s.height;
            d.ht = s.ht;
            d.lw = s.lw;
            d.rw = s.rw;
            d.label = label;
            d.xlabel = xlabel;
            d.node_type = s.node_type;
            d.rank = s.rank;
            d.order = s.order;
            d.has_port = s.has_port;
            d.is_record = s.is_record;
            d.shape = s.shape.clone();
            d.rec_fields = s.rec_fields.clone();
            nclust[ri] = s.clust.map(|g| match local.get(&g) {
                Some(&k) => PkClust::Clust(k),
                None => PkClust::Graph(ci),
            });
        }
        for (le, &re) in cg.edges.iter().enumerate() {
            let s = &c.edges[le];
            let label = self.copy_label(c, s.label);
            let head_label = self.copy_label(c, s.head_label);
            let tail_label = self.copy_label(c, s.tail_label);
            let xlabel = self.copy_label(c, s.xlabel);
            let d = &mut self.edges[re];
            d.tail_port = s.tail_port.clone();
            d.head_port = s.head_port.clone();
            d.label = label;
            d.head_label = head_label;
            d.tail_label = tail_label;
            d.xlabel = xlabel;
            d.edge_type = s.edge_type;
            d.adjacent = s.adjacent;
            d.label_ontop = s.label_ontop;
            d.conc_opp_flag = s.conc_opp_flag;
            d.weight = s.weight;
            d.minlen = s.minlen;
            d.count = s.count;
            d.spl = s.spl.clone();
        }
        self.graphs[0].has_labels |= c.graphs[0].has_labels;
        let b = c.graphs[0].bb;
        CompInfo {
            nodes: cg.nodes.clone(),
            edges: cg.edges.iter().copied().collect(),
            clusters,
            bb: BoundingBox::new(b.ll.x, b.ll.y, b.ur.x, b.ur.y),
        }
    }

    /// `copyCluster(scl, cl)`: a component cluster's record onto the original.
    fn copy_cluster_info(&mut self, pcl: &[PClust], k: usize) -> G {
        let kids: Vec<G> = pcl[k]
            .kids
            .iter()
            .map(|&j| self.copy_cluster_info(pcl, j))
            .collect();
        let p = &pcl[k];
        let d = &mut self.graphs[p.orig];
        d.bb = p.bb;
        d.label_pos = p.label_pos;
        d.border = p.border;
        d.label = p.label;
        d.clust = kids;
        d.bound = true;
        p.orig
    }
}

/// The nodes and edges (root indices) of a projected subgraph after its
/// component's layout.
struct Membership {
    nodes: std::collections::BTreeSet<N>,
    edges: std::collections::BTreeSet<E>,
}

/// [`Membership`] of each of `cc`'s projected subgraphs that the component's
/// layout `c` knows (by index into `cc.subgraphs`).
fn projected_membership(c: &Dot<'_>, cg: &CompGraph, cc: &CComp) -> Vec<Option<Membership>> {
    let mut out: Vec<Option<Membership>> = (0..cc.subgraphs.len()).map(|_| None).collect();
    for s in &c.graphs {
        let Some(slot) = s.sub_index.and_then(|k| out.get_mut(k)) else {
            continue;
        };
        *slot = Some(Membership {
            nodes: s
                .nodes
                .iter()
                .filter_map(|&n| c.nodes.get(n)?.real.and_then(|r| cg.nodes.get(r)))
                .copied()
                .collect(),
            edges: s
                .edges
                .iter()
                .filter_map(|&e| c.edges.get(e)?.real.and_then(|r| cg.edges.get(r)))
                .copied()
                .collect(),
        });
    }
    out
}

/// A cluster subgraph of a component (`projectG`'s clone), with the
/// `Agraphinfo_t` fields packing uses; `copyClusterInfo` moves them to `orig`.
struct PClust {
    orig: G,
    bb: BoxF,
    label: Option<L>,
    label_pos: u8,
    border: [Point; 4],
    kids: Vec<usize>,
    /// Root node indices.
    nodes: Vec<N>,
}

/// [`PackGraph`] over the root's [`Dot`] after the components were absorbed.
struct DotPack<'d, 'a> {
    d: &'d mut Dot<'a>,
    comps: Vec<CompInfo>,
    /// `ND_pos` (inches), set by `attachPos`.
    pos: Vec<Point>,
    nclust: Vec<Option<PkClust>>,
    pcl: Vec<PClust>,
}

impl DotPack<'_, '_> {
    fn bb_of(b: BoxF) -> BoundingBox {
        BoundingBox::new(b.ll.x, b.ll.y, b.ur.x, b.ur.y)
    }

    /// `compute_bb(g)` over `nodes`, with the top-level `clusters`. `agfstout(g, n)`
    /// walks `g`'s own out-edges: a component's are `edges` (a node of two
    /// non-nested clusters is in two components, and its edges into the other
    /// one are not in this one, #100); the root's are all of them (`None`).
    fn compute_bb(
        &self,
        nodes: &[N],
        edges: Option<&std::collections::HashSet<E>>,
        clusters: &[G],
    ) -> Option<BoundingBox> {
        let d = &*self.d;
        if nodes.is_empty() && clusters.is_empty() {
            return None;
        }
        let flip = d.graphs[0].flip();
        let m = f64::from(i32::MAX);
        let mut bb = BoundingBox::new(m, m, -m, -m);
        let lbb = |bb: &mut BoundingBox, l: Option<L>| {
            if let Some(l) = l.map(|l| &d.labels[l]).filter(|l| l.set) {
                add_label_bb_dims(bb, l.dimen, l.pos, flip);
            }
        };
        for &n in nodes {
            let p = self.pk_coord(n);
            let nd = &d.nodes[n];
            let s2 = Point::new((nd.lw + nd.rw) / 2.0, nd.ht / 2.0);
            expandbb(
                &mut bb,
                &BoundingBox::new(p.x - s2.x, p.y - s2.y, p.x + s2.x, p.y + s2.y),
            );
            lbb(&mut bb, nd.xlabel);
            for &e in &nd.cg_out {
                if edges.is_some_and(|edges| !edges.contains(&e)) {
                    continue;
                }
                let ed = &d.edges[e];
                let Some(spl) = ed.spl.as_ref() else { continue };
                for b in spl {
                    for &p in &b.list {
                        expandbp(&mut bb, p);
                    }
                }
                lbb(&mut bb, ed.label);
                lbb(&mut bb, ed.head_label);
                lbb(&mut bb, ed.tail_label);
                lbb(&mut bb, ed.xlabel);
            }
        }
        for &c in clusters {
            expandbb(&mut bb, &Self::bb_of(self.pcl[c].bb));
        }
        Some(bb)
    }
}

impl PackGraph for DotPack<'_, '_> {
    fn pk_ncomps(&self) -> usize {
        self.comps.len()
    }
    /// cccomps names its components `_cc_<i>`.
    fn pk_name(&self, i: usize) -> String {
        format!("_cc_{i}")
    }
    fn pk_compute_bb(&mut self, i: usize) {
        let c = &self.comps[i];
        if let Some(bb) = self.compute_bb(&c.nodes, Some(&c.edges), &c.clusters) {
            self.comps[i].bb = bb;
        }
        if std::env::var_os("GVDUMP").is_some() {
            let b = self.comps[i].bb;
            graphviz_pure_core::diag::dump(&format!(
                "bb[_cc_{i}] {} {} {} {}",
                b.ll.x, b.ll.y, b.ur.x, b.ur.y
            ));
        }
    }
    fn pk_graph_bb(&self, g: PkGraph) -> BoundingBox {
        match g {
            PkGraph::Comp(i) => self.comps[i].bb,
            PkGraph::Clust(c) => Self::bb_of(self.pcl[c].bb),
        }
    }
    fn pk_nodes(&self, i: usize) -> Vec<usize> {
        self.comps[i].nodes.clone()
    }
    fn pk_clusters(&self, g: PkGraph) -> Vec<usize> {
        match g {
            PkGraph::Comp(i) => self.comps[i].clusters.clone(),
            PkGraph::Clust(c) => self.pcl[c].kids.clone(),
        }
    }
    fn pk_cluster_nodes(&self, c: usize) -> Vec<usize> {
        self.pcl[c].nodes.clone()
    }
    fn pk_node_clust(&self, n: usize) -> Option<PkClust> {
        self.nclust[n]
    }
    fn pk_coord(&self, n: usize) -> Point {
        Point::new(72.0 * self.pos[n].x, 72.0 * self.pos[n].y)
    }
    fn pk_node_size(&self, n: usize) -> Point {
        let nd = &self.d.nodes[n];
        Point::new(nd.lw + nd.rw, nd.ht)
    }
    fn pk_root_outs(&self, n: usize) -> Vec<usize> {
        self.d.nodes[n].cg_out.clone()
    }
    fn pk_head(&self, e: usize) -> usize {
        self.d.edges[e].head
    }
    fn pk_spl(&self, e: usize) -> Option<&[crate::common::types::Bezier]> {
        self.d.edges[e].spl.as_deref()
    }
    fn pk_sortv(&self, _i: usize) -> Option<&str> {
        self.d.agget(0, "sortv")
    }
    fn pk_shift_node(&mut self, n: usize, dx: f64, dy: f64) {
        self.pos[n].x += dx / 72.0;
        self.pos[n].y += dy / 72.0;
        let nd = &mut self.d.nodes[n];
        nd.coord.x += dx;
        nd.coord.y += dy;
        if let Some(l) = nd.xlabel {
            let p = &mut self.d.labels[l].pos;
            p.x += dx;
            p.y += dy;
        }
    }
    fn pk_shift_edge(&mut self, e: usize, dx: f64, dy: f64) {
        let ed = &self.d.edges[e];
        for l in [ed.label, ed.xlabel, ed.head_label, ed.tail_label]
            .into_iter()
            .flatten()
        {
            let p = &mut self.d.labels[l].pos;
            p.x += dx;
            p.y += dy;
        }
        if let Some(spl) = self.d.edges[e].spl.as_mut() {
            shift_beziers(spl, dx, dy);
        }
    }
    fn pk_shift_graph_box(&mut self, g: PkGraph, dx: f64, dy: f64) {
        match g {
            PkGraph::Comp(i) => {
                let bb = &mut self.comps[i].bb;
                bb.ll.x += dx;
                bb.ur.x += dx;
                bb.ll.y += dy;
                bb.ur.y += dy;
            }
            PkGraph::Clust(c) => {
                let gi = &mut self.pcl[c];
                gi.bb.ll.x += dx;
                gi.bb.ur.x += dx;
                gi.bb.ll.y += dy;
                gi.bb.ur.y += dy;
                if let Some(l) = gi.label
                    && self.d.labels[l].set
                {
                    let p = &mut self.d.labels[l].pos;
                    p.x += dx;
                    p.y += dy;
                }
            }
        }
    }
}

/// `doDot(g)` with the root state `d` (after `graph_init`): plain `dotLayout`,
/// or the pack branch, which packs the components whatever their `phase`.
pub fn do_dot(d: &mut Dot<'_>, graph: &Graph) -> Result<(), String> {
    let DoDot::Pack(mut pinfo) = d.do_dot_mode() else {
        return d.dot_layout();
    };
    let ccs = cccomps(graph);
    d.cg_in_id_packed = true;
    if ccs.len() == 1 || d.graphs[0].drawing.ratio_kind != RatioKind::None {
        // `dotLayout(g)` runs while the `_cc_<i>` subgraphs cccomps created
        // are still children of g, so dot sees them (e.g. a root `rank` value
        // they inherit makes each one a rank set). They are deleted only after
        // the layout succeeded (`dot_cleanup_graph` + `agdelete`).
        let ccsubs = d.attach_cc_subgraphs(graph, &ccs, &[]);
        d.dot_layout()?;
        if d.aborted {
            d.cc_kept = ccsubs;
        } else {
            // the input's own `_cc_<i>` goes with the others (agdelete)
            d.cc_deleted = ccs.iter().filter_map(|c| c.user).collect();
            let ccsubs: std::collections::BTreeSet<G> = ccsubs.into_iter().collect();
            d.graphs[0].subgs.retain(|s| !ccsubs.contains(s));
        }
        return Ok(());
    }
    pinfo.do_splines = true;
    // original subgraph index -> root graph id
    let mut clust_of_sub: Vec<Option<G>> = vec![None; graph.subgraphs.len()];
    for (g, gi) in d.graphs.iter().enumerate() {
        if let Some(si) = gi.sub_index {
            clust_of_sub[si] = Some(g);
        }
    }
    let mut nclust: Vec<Option<PkClust>> = vec![None; d.nodes.len()];
    let mut pcl: Vec<PClust> = Vec::new();
    let mut comps = Vec::with_capacity(ccs.len());
    // What each laid-out component's dot left in its projected subgraphs (dot's
    // node_induce adds the induced edges to its cluster clones and drops nodes from
    // them), which a failed layout leaves in the root.
    let mut projected: Vec<Vec<Option<Membership>>> = Vec::with_capacity(ccs.len());
    let mut any_splines = false;
    for (ci, cc) in ccs.iter().enumerate() {
        let cg = cc.to_graph(graph);
        if std::env::var_os("GVDUMP").is_some() {
            graphviz_pure_core::diag::dump(&format!(
                "CC {ci}: {} nodes {} edges {} subgraphs",
                cg.nodes.len(),
                cg.edges.len(),
                cg.subgraphs.len()
            ));
        }
        let mut c = Dot::new_component(&cg.graph, graph, d);
        c.comp_index = ci;
        c.cg_in_id_packed = true;
        c.cg_root_node = Some(cg.nodes.clone());
        c.dot_layout()?;
        // dotLayout(sg) stops at the component's own `phase` (agget(sg)) with
        // attach_phase_attrs (`phase=3` attaches as 2); a layout that failed returned first.
        let phase = super::utils::late_int(c.agget(0, "phase"), -1, 1);
        d.node_phase.resize(d.nodes.len(), None);
        if (1..=3).contains(&phase) && !c.aborted {
            for &n in &cg.nodes {
                if let Some(p) = d.node_phase.get_mut(n) {
                    *p = Some(phase.min(2));
                }
            }
        }
        // `clip_and_install` grew the root's box.
        d.graphs[0].bb = c.root_bb;
        // The component's nodes and edges are the root's objects in C, so what
        // its layout wrote before failing (dot_init_node_edge's sizes, ...)
        // stays visible to the root.
        let comp = d.absorb(&c, &cg, ci, &clust_of_sub, &mut pcl, &mut nclust);
        projected.push(projected_membership(&c, &cg, cc));
        if c.aborted {
            // doDot returns the component's failure before packing, so neither
            // dot_cleanup_graph nor agdelete runs: the `_cc_<i>` subgraphs stay.
            d.aborted = true;
            d.cc_kept = d.attach_cc_subgraphs(graph, &ccs, &projected);
            return Ok(());
        }
        any_splines |= c.splines_done;
        comps.push(comp);
    }
    // Every component laid out: doDot ends with agdelete(g, ccs[i]) (dotinit.c:503),
    // whether the layouts stopped at a `phase` or went on to be packed.
    d.cc_deleted = ccs.iter().filter_map(|c| c.user).collect();
    // attachPos
    let pos: Vec<Point> = d
        .nodes
        .iter()
        .map(|n| Point::new(n.coord.x / 72.0, n.coord.y / 72.0))
        .collect();
    let mut pk = DotPack {
        d,
        comps,
        pos,
        nclust,
        pcl,
    };
    // packSubgraphs
    if pack_graphs_in(&mut pk, &pinfo) {
        let all = pk.d.nodes_of(0);
        let mut bb = pk.compute_bb(&all, None, &[]).unwrap_or_default();
        for c in &pk.comps {
            for &k in &c.clusters {
                expandbb(&mut bb, &DotPack::bb_of(pk.pcl[k].bb));
            }
        }
        pk.d.graphs[0].bb = BoxF {
            ll: bb.ll,
            ur: bb.ur,
        };
    }
    // resetCoord
    for n in pk.d.nodes_of(0) {
        pk.d.nodes[n].coord = Point::new(72.0 * pk.pos[n].x, 72.0 * pk.pos[n].y);
    }
    // copyClusterInfo
    let mut clust: Vec<G> = Vec::new();
    for c in &pk.comps {
        for &k in &c.clusters {
            clust.push(pk.d.copy_cluster_info(&pk.pcl, k));
        }
    }
    let d = pk.d;
    d.graphs[0].clust = clust;
    // The root never had setEdgeType: EDGE_TYPE(root) is EDGETYPE_NONE.
    d.et = EdgeType::None;
    // State is GVSPLINES once a component's dot_splines_ finished (not with splines=none).
    d.splines_done = any_splines;
    Ok(())
}
