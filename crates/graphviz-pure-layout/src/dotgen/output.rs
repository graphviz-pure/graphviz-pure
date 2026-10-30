//! Write-back of the dotgen results into the [`graphviz_pure_core::Graph`] IR: node
//! positions, cluster and graph bounding boxes, edge splines and label
//! positions (`lp`, `head_lp`, `tail_lp`, `xlp`), all in final drawing
//! coordinates after `dotneato_postprocess`.

use super::Dot;
use super::types::*;
use graphviz_pure_core::{BezierSegment, BoundingBox, Graph, Point, Spline};

/// Final layout results, detached from the [`Dot`] borrow of the input graph.
pub struct Output {
    node_pos: Vec<Option<Point>>,
    node_rank: Vec<(i32, i32)>,
    /// `ND_lw`, `ND_rw` and `ND_ht`, which the output and the renderers take as the node
    /// size, when `translate_drawing` did not reset them (`gv_nodesize` runs for
    /// rankdir != TB, unless the layout was abandoned and no postprocessing ran).
    node_size: Vec<Option<(f64, f64, f64)>>,
    /// `attach_phase_attrs(g, n)`: write `rank` (and `order` from 2) for `phase=n`.
    phase_attrs: Option<i32>,
    /// `attach_phase_attrs` of each node's pack component (`Dot::node_phase`).
    node_phase: Vec<Option<i32>>,
    bb: BoundingBox,
    cluster_bb: Vec<(usize, BoundingBox)>,
    /// Spline, `lp`, and whether the label is unplaced (`!ED_label(e)->set`).
    edges: Vec<Option<(Spline, Option<Point>, bool)>>,
    /// The rest of each edge's result (labels, ports, arrow flags).
    edge_layout: Vec<graphviz_pure_core::EdgeLayout>,
    node_xlp: Vec<Option<Point>>,
    /// `lp` of the root graph and of clusters (by subgraph index).
    graph_lp: Option<Point>,
    cluster_lp: Vec<(usize, Point)>,
    /// `GD_clust` of the root (`None`) and of each cluster, as subgraph indices.
    cluster_list: Vec<(Option<usize>, Vec<usize>)>,
    /// The `_cc_<i>` subgraphs a failed pack layout leaves (`Dot::cc_kept`), and
    /// their projected subgraphs, parents first.
    cc_subgraphs: Vec<CcSub>,
    /// The members each input subgraph has in dot's cgraph when the layout ends (by
    /// subgraph index; nodes, edges). The writers read the cgraph after layout, so they
    /// see what `node_induce` (`agsubedge`, `agdelete`) and `mark_clusters` (`agdelete`)
    /// changed.
    membership: Vec<(usize, Vec<usize>, Vec<usize>)>,
    /// Input subgraphs doDot deleted (`Dot::cc_deleted`).
    deleted: Vec<usize>,
}

/// A subgraph `cccomps` added to the root.
struct CcSub {
    name: String,
    /// The subgraph a projection copies.
    orig: Option<usize>,
    attrs: graphviz_pure_core::AttrMap,
    nodes: Vec<usize>,
    edges: Vec<usize>,
    /// Index into [`Output::cc_subgraphs`]; `None` for the root.
    parent: Option<usize>,
    /// The input subgraph it hangs under instead: the input's own `_cc_<i>`, which cccomps
    /// took as a component (#335).
    input_parent: Option<usize>,
}

/// The members of each input subgraph in `d`'s cgraph. Under a pack layout with several
/// components, dot ran on the components' projections (`projectG`), so the input
/// subgraphs still have their declared members. Objects dot created are left out:
/// `removeFill` deletes newrank's fill nodes, and where it does not run (`phase=2`, `3`)
/// C crashes.
fn membership(d: &Dot<'_>) -> Vec<(usize, Vec<usize>, Vec<usize>)> {
    d.graphs
        .iter()
        .filter_map(|s| {
            let si = s.sub_index?;
            let nodes = s
                .nodes
                .iter()
                .copied()
                .filter(|&n| n < d.input.nodes.len())
                .collect();
            let edges = s
                .edges
                .iter()
                .copied()
                .filter(|&e| e < d.input.edges.len())
                .collect();
            Some((si, nodes, edges))
        })
        .collect()
}

/// The subgraph trees under `kept` (preorder).
fn cc_subgraphs(d: &Dot<'_>, kept: &[G]) -> Vec<CcSub> {
    fn rec(
        d: &Dot<'_>,
        g: G,
        parent: Option<usize>,
        input_parent: Option<usize>,
        out: &mut Vec<CcSub>,
    ) {
        let Some(s) = d.graphs.get(g) else { return };
        let me = out.len();
        out.push(CcSub {
            name: s.name.clone(),
            orig: s.proj_of,
            attrs: s.attrs.clone(),
            nodes: s
                .nodes
                .iter()
                .copied()
                .filter(|&n| n < d.input.nodes.len())
                .collect(),
            edges: s
                .edges
                .iter()
                .copied()
                .filter(|&e| e < d.input.edges.len())
                .collect(),
            parent,
            input_parent,
        });
        for &k in &s.subgs {
            rec(d, k, Some(me), None, out);
        }
    }
    let mut out = Vec::new();
    for &g in kept {
        match d.graphs.get(g).and_then(|s| s.sub_index.map(|si| (s, si))) {
            // The input's own `_cc_<i>` stays where it is; the projections cccomps added
            // go under it (its own subgraphs are input subgraphs already).
            Some((s, si)) => {
                for &k in &s.subgs {
                    if d.graphs.get(k).is_some_and(|c| c.sub_index.is_none()) {
                        rec(d, k, None, Some(si), &mut out);
                    }
                }
            }
            None => rec(d, g, None, None, &mut out),
        }
    }
    out
}

impl Output {
    /// The layout's results, with `rank` and `order` attached when `phase_attrs` is
    /// `attach_phase_attrs`'s level.
    pub fn collect(d: &Dot<'_>, phase_attrs: Option<i32>) -> Output {
        let n = d.input.nodes.len();
        let node_pos = (0..n).map(|i| Some(d.nodes[i].coord)).collect();
        let node_rank = (0..n)
            .map(|i| (d.nodes[i].rank, d.nodes[i].order))
            .collect();
        // With splines=curved, dot_splines_ swaps ND_rw and ND_mval twice (resetRW, then
        // the self-loop restore), so a node with self loops keeps the loop space in
        // ND_rw and is drawn wider.
        // An abandoned layout skips dotneato_postprocess, so a flipped rankdir keeps the
        // rotated sizes of dot_init_node's gv_nodesize(n, GD_flip(g)).
        let kept = d.graphs[0].rankdir & 3 == 0 || d.aborted;
        let node_size = (0..n)
            .map(|i| kept.then(|| (d.nodes[i].lw, d.nodes[i].rw, d.nodes[i].ht)))
            .collect();
        let b = d.graphs[0].bb;
        let bb = BoundingBox::new(b.ll.x, b.ll.y, b.ur.x, b.ur.y);
        let mut cluster_bb = Vec::new();
        fn rec(d: &Dot<'_>, g: G, out: &mut Vec<(usize, BoundingBox)>) {
            for &c in &d.graphs[g].clust {
                if let Some(si) = d.graphs[c].sub_index {
                    let b = d.graphs[c].bb;
                    out.push((si, BoundingBox::new(b.ll.x, b.ll.y, b.ur.x, b.ur.y)));
                }
                rec(d, c, out);
            }
        }
        rec(d, 0, &mut cluster_bb);
        // The order (and, after copyClusterInfo, the repeats) emit_clusters walks.
        let mut cluster_list = Vec::new();
        fn rec_list(d: &Dot<'_>, g: G, out: &mut Vec<(Option<usize>, Vec<usize>)>) {
            let list = d.graphs[g]
                .clust
                .iter()
                .filter_map(|&c| d.graphs[c].sub_index)
                .collect();
            // A graph with no subgraph of its own would otherwise land on the root.
            if g == 0 || d.graphs[g].sub_index.is_some() {
                out.push((d.graphs[g].sub_index, list));
            }
            for &c in &d.graphs[g].clust {
                rec_list(d, c, out);
            }
        }
        rec_list(d, 0, &mut cluster_list);
        let mut edges = Vec::new();
        let mut edge_layout = Vec::new();
        let lp_of = |l: Option<L>| l.filter(|&l| d.labels[l].set).map(|l| d.labels[l].pos);
        let place_of = |l: Option<L>| {
            l.map(|l| graphviz_pure_core::LabelPlace {
                pos: d.labels[l].pos,
                set: d.labels[l].set,
            })
        };
        // An IGNORED edge can still carry a spline (orthoEdges under concentrate routes
        // the first edge of each node pair whatever its type), which emit_edge_graphics
        // draws: the renderer gets it, marked so that attach_attrs writes no attributes.
        let pos_of = |l: Option<L>| l.map(|l| d.labels[l].pos);
        for ei in 0..d.input.edges.len() {
            let ed = &d.edges[ei];
            // Every field spelled out, so a new one is decided here for dot.
            let mut el = graphviz_pure_core::EdgeLayout {
                seq: None,
                xlabel: lp_of(ed.xlabel),
                head_label: place_of(ed.head_label),
                tail_label: place_of(ed.tail_label),
                // Both names, for every edge, drawn or not, before the branches below:
                // `\E` (in the title, `id`, `URL`, `tooltip`) reads `ED_tail_port(e).name`
                // and `ED_head_port(e).name` (labels.c:316) of every edge C emits, a
                // labelled edge without a spline too (#286).
                port_names: graphviz_pure_core::PortNames::Layout {
                    tail: ed.tail_port.name.clone().unwrap_or_default(),
                    head: ed.head_port.name.clone().unwrap_or_default(),
                },
                no_attrs: false,
            };
            let Some(bzs) = ed.spl.as_ref() else {
                edge_layout.push(el);
                // A lost edge (no spline) still draws its placed label (emit_edge);
                // the output writer skips edges without splines. `splines_done` is
                // C's `State == GVSPLINES`: false with `splines=none`, but true after
                // normal or pack-component spline layout, when the root's edge type is none.
                let routed = d.splines_done;
                match ed
                    .label
                    .filter(|&l| routed && ed.edge_type != IGNORED && d.labels[l].set)
                {
                    Some(l) => edges.push(Some((Spline::default(), Some(d.labels[l].pos), false))),
                    // An unset label keeps its position (`(0,0)`, moved by
                    // `shiftEdge` in the pack branch; `map_edge` skips edges
                    // without splines). `edge_in_box` (emit.c) tests
                    // `ED_label(e)` without looking at `set`, so the renderer
                    // needs that position to decide whether to emit the edge.
                    None => edges.push(
                        ed.label
                            .filter(|&l| !d.labels[l].set)
                            .map(|l| (Spline::default(), Some(d.labels[l].pos), true)),
                    ),
                }
                continue;
            };
            el.no_attrs = ed.edge_type == IGNORED;
            edge_layout.push(el);
            let spline = Spline {
                segments: bzs
                    .iter()
                    .map(|bz| BezierSegment {
                        points: bz.list.clone(),
                        start_arrow: (bz.sflag != 0).then_some(bz.sp),
                        end_arrow: (bz.eflag != 0).then_some(bz.ep),
                        sflag: bz.sflag,
                        eflag: bz.eflag,
                    })
                    .collect(),
            };
            let unplaced = ed.label.is_some_and(|l| !d.labels[l].set);
            edges.push(Some((spline, pos_of(ed.label), unplaced)));
        }
        let node_xlp = (0..n).map(|i| lp_of(d.nodes[i].xlabel)).collect();
        let graph_lp = lp_of(d.graphs[0].label);
        let mut cluster_lp = Vec::new();
        fn rec_lp(d: &Dot<'_>, g: G, out: &mut Vec<(usize, Point)>) {
            for &c in &d.graphs[g].clust {
                if let (Some(si), Some(l)) = (d.graphs[c].sub_index, d.graphs[c].label)
                    && d.labels[l].set
                {
                    out.push((si, d.labels[l].pos));
                }
                rec_lp(d, c, out);
            }
        }
        rec_lp(d, 0, &mut cluster_lp);
        Output {
            node_pos,
            node_rank,
            node_size,
            phase_attrs,
            node_phase: d.node_phase.clone(),
            bb,
            cluster_bb,
            edges,
            edge_layout,
            node_xlp,
            graph_lp,
            cluster_lp,
            cluster_list,
            cc_subgraphs: cc_subgraphs(d, &d.cc_kept),
            membership: membership(d),
            deleted: d.cc_deleted.clone(),
        }
    }

    pub fn apply(self, graph: &mut Graph) {
        for (i, p) in self.node_pos.into_iter().enumerate() {
            if let Some(p) = p {
                graph.nodes[i].pos = p;
            }
            if let Some((lw, rw, ht)) = self.node_size[i] {
                let nd = &mut graph.nodes[i];
                let w = lw + rw;
                if (w - nd.width * 72.0).abs() > 1e-9 {
                    nd.width = w / 72.0;
                }
                if (ht - nd.height * 72.0).abs() > 1e-9 {
                    nd.height = ht / 72.0;
                }
                if lw != rw {
                    nd.layout.lw_rw = Some((lw, rw));
                }
            }
            let (r, o) = self.node_rank[i];
            graph.nodes[i].rank = r;
            graph.nodes[i].order = o.max(0) as usize;
            // a pack component's own phase, else the root's
            if let Some(ph) = self
                .node_phase
                .get(i)
                .copied()
                .flatten()
                .or(self.phase_attrs.map(|p| p.min(2)))
            {
                let a = &mut graph.nodes[i].attrs;
                a.set("rank", r.to_string(), false);
                if ph >= 2 {
                    a.set("order", o.to_string(), false);
                }
            }
        }
        graph.bb = self.bb;
        for (si, bb) in self.cluster_bb {
            graph.subgraphs[si].bb = bb;
            // `is_a_cluster` also honours `cluster=true`.
            graph.subgraphs[si].is_cluster = true;
            // In GD_clust, so drawn even with an empty box (an abandoned layout).
            graph.subgraphs[si].layout.reg = graphviz_pure_core::ClusterReg::Registered;
        }
        for (i, e) in self.edges.into_iter().enumerate() {
            if let Some((s, lp, unplaced)) = e {
                graph.edges[i].spline = s;
                graph.edges[i].label_pos = lp;
                graph.edges[i].label_unplaced = unplaced;
            }
        }
        for (e, el) in graph.edges.iter_mut().zip(self.edge_layout) {
            e.layout = graphviz_pure_core::EdgeLayout {
                seq: e.layout.seq,
                ..el
            };
        }
        if let Some(lp) = self.graph_lp {
            graph.layout.label_pos = Some(lp);
        }
        for (si, lp) in self.cluster_lp {
            graph.subgraphs[si].layout.label_pos = Some(lp);
        }
        for (si, list) in self.cluster_list {
            let layout = match si {
                Some(si) => &mut graph.subgraphs[si].layout,
                None => &mut graph.layout,
            };
            layout.clusters = Some(list);
        }
        for (i, xlp) in self.node_xlp.into_iter().enumerate() {
            if xlp.is_some() {
                graph.nodes[i].layout.xlabel = xlp;
            }
        }
        for (si, nodes, edges) in self.membership {
            let Some(s) = graph.subgraphs.get_mut(si) else {
                continue;
            };
            s.nodes = nodes
                .into_iter()
                .map(|n| graphviz_pure_core::NodeId(n as u32))
                .collect();
            s.edges = edges
                .into_iter()
                .map(|e| graphviz_pure_core::EdgeId(e as u32))
                .collect();
        }
        let base = graph.subgraphs.len();
        let dicts = graph.dicts.len().checked_sub(1) == Some(base);
        let mut parent = vec![None; base];
        for (i, s) in graph.subgraphs.iter().enumerate() {
            for c in &s.subgraphs {
                if let Some(p) = parent.get_mut(c.0 as usize) {
                    *p = Some(i);
                }
            }
        }
        // Subgraph index of each `cc_subgraphs` entry.
        let mut ids: Vec<usize> = Vec::with_capacity(self.cc_subgraphs.len());
        for (id, mut c) in (base..).zip(self.cc_subgraphs) {
            let sid = graphviz_pure_core::SubgraphId(id as u32);
            if let Some(p) = c
                .parent
                .and_then(|p| ids.get(p))
                .copied()
                .or(c.input_parent)
                && let Some(p) = graph.subgraphs.get_mut(p)
            {
                p.subgraphs.push(sid);
            }
            ids.push(id);
            // `agsubg` gives `_cc_<i>` no declarations of its own. A projection's
            // `agcopyattr` sets every graph attribute visible from the original with
            // `agxset`, which declares it in the projection's own dictionary.
            let mut dict = graphviz_pure_core::AttrDicts::default();
            if let Some(o) = c.orig.filter(|_| dicts)
                && let Some((root_dict, sub_dicts)) = graph.dicts.split_first()
            {
                // The nearest declaration, from the original up to the root.
                let mut chain = Vec::new();
                let mut cur = Some(o);
                while let Some(k) = cur {
                    chain.extend(sub_dicts.get(k));
                    cur = parent.get(k).copied().flatten();
                }
                chain.push(root_dict);
                for d in chain {
                    for (k, sym) in &d.graph {
                        dict.graph.entry(k.clone()).or_insert_with(|| {
                            graphviz_pure_core::AttrSym {
                                defval: sym.defval.clone(),
                                html: sym.html,
                                html_defval: sym.html_defval,
                                ..Default::default()
                            }
                        });
                    }
                }
                for (k, sym) in &dict.graph {
                    c.attrs.insert(k.clone(), sym.defval.clone());
                }
            }
            let is_cluster = graphviz_pure_core::has_cluster_prefix(&c.name)
                || crate::common::utils::mapbool(c.attrs.get("cluster").map(String::as_str));
            // `projectG` names an anonymous subgraph's projection `agnameof(subg)`,
            // the original's `%<id>`.
            if let Some(&anon) = c
                .orig
                .and_then(|o| graph.anon_subgraph_ids.get(&(o as u32)))
            {
                graph.anon_subgraph_ids.insert(id as u32, anon);
            }
            let orig = c.orig.and_then(|o| graph.subgraphs.get(o));
            let html_name = orig.is_some_and(|s| s.html_name);
            let empty_name = orig.is_some_and(|s| s.empty_name);
            graph.subgraphs.push(graphviz_pure_core::SubgraphData {
                id: sid,
                name: c.name,
                is_cluster,
                graph_attrs: c.attrs,
                node_defaults: Default::default(),
                edge_defaults: Default::default(),
                nodes: c
                    .nodes
                    .into_iter()
                    .map(|n| graphviz_pure_core::NodeId(n as u32))
                    .collect(),
                edges: c
                    .edges
                    .into_iter()
                    .map(|e| graphviz_pure_core::EdgeId(e as u32))
                    .collect(),
                subgraphs: Vec::new(),
                bb: Default::default(),
                html_name,
                empty_name,
                layout: Default::default(),
            });
            if dicts {
                graph.dicts.push(dict);
            }
        } // doDot's agdelete of the input's own `_cc_<i>` subgraphs, after everything above
        // has used the input's subgraph indices.
        graph.agdelete_subgraphs(&self.deleted);
    }
}
