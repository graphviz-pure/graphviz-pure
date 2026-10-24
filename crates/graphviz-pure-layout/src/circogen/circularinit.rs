//! `lib/circogen/circularinit.c`: the circo driver.

use super::circular::{CData, Circ, circular_layout};
use super::graph::{CGraph, EdgeArena};
use crate::common::postproc::gv_postprocess;
use crate::common::types::{Drawing, EdgeType, LEdge, LGraph, LNode, PortInfo, Sub};
use crate::common::utils::mapbool;
use crate::neatogen::adjust::{AdjustMode, adjust_nodes, get_adjust_mode, remove_overlap_with};
use crate::neatogen::neatoinit::CL_OFFSET;
use crate::neatogen::neatosplines::spline_edges;
use crate::pack::{PackInfo, PackMode, ccomps, get_pack_info, pack_graphs};
use graphviz_pure_core::{BoundingBox, Graph, GraphKind, NodeId, Point, Result};
use std::collections::HashMap;

/// The derived graph of `circomps`.
struct Derived {
    dg: CGraph,
    /// For each derived edge, the original edge that created it (same
    /// tail and head).
    orig_edge: Vec<usize>,
}

/// `makeDerivedNode(dg, agnameof(v), 1, v)` for each node of `g`. The derived graph is a root
/// of its own, whose dictionary has no HTML strings, so nodes named `<G>` and `G` are one
/// derived node. The first of them creates it (its place in the derived graph); the last is
/// its `ORIGN` (the sizes and attributes it reads, and the node its position is copied to).
/// The others never get a position. Returns each node's derived node, named by the first
/// node, and each derived node's `ORIGN`.
fn derived_nodes(g: &LGraph<'_>) -> (Vec<usize>, Vec<usize>) {
    let n = g.nodes.len();
    let mut first: HashMap<&str, usize> = HashMap::new();
    let mut dnode = vec![0; n];
    let mut orign: Vec<usize> = (0..n).collect();
    for v in 0..n {
        let f = *first.entry(g.src.nodes[v].name.as_str()).or_insert(v);
        dnode[v] = f;
        orign[f] = v;
    }
    (dnode, orign)
}

/// `circomps(g, &cnt)` without the component split: build the strict,
/// undirected, loop-free derived graph of `g`.
fn derived_graph(g: &LGraph<'_>, dnode: &[usize], arena: &mut EdgeArena) -> Derived {
    let mut dg = CGraph::default();
    for v in 0..g.nodes.len() {
        if dnode[v] == v {
            dg.add_node(v);
        }
    }
    let mut orig_edge = Vec::new();
    for v in 0..g.nodes.len() {
        for &e in &g.out[v] {
            let (dt, dh) = (dnode[v], dnode[g.edges[e].head]);
            if dt != dh {
                let before = arena.tail.len();
                let de = dg.edge(arena, dt, dh);
                if de >= before {
                    orig_edge.resize(de + 1, usize::MAX);
                    orig_edge[de] = e;
                }
            }
        }
    }
    Derived { dg, orig_edge }
}

/// `ccomps(dg, &c_cnt, 0)`: the connected components of the derived graph, each in node
/// order, ordered by their first node.
fn derived_comps(dg: &CGraph, arena: &EdgeArena, n_total: usize) -> Vec<Sub> {
    let mut mark = vec![false; n_total];
    let mut out = Vec::new();
    for v in dg.nodes() {
        if mark[v] {
            continue;
        }
        mark[v] = true;
        let (mut c, mut stack) = (Vec::new(), vec![v]);
        while let Some(u) = stack.pop() {
            c.push(u);
            for e in dg.edges(u) {
                let w = arena.other(e, u);
                if !mark[w] {
                    mark[w] = true;
                    stack.push(w);
                }
            }
        }
        let mut sub = Sub::new(n_total, c);
        // ccomps(dg, &c_cnt, 0) names them with setPrefix's default
        sub.name = format!("_cc_{}", out.len());
        out.push(sub);
    }
    out
}

/// A component of the derived graph as `circomps` fills it in: its nodes, and for each the
/// derived edges of its `ORIGN`'s out-edges. An edge whose tail was merged into another
/// node's derived node is not among them.
fn component(
    g: &LGraph<'_>,
    dg: &CGraph,
    arena: &EdgeArena,
    nodes: &[usize],
    dnode: &[usize],
    orign: &[usize],
) -> CGraph {
    let mut sg = CGraph::default();
    for &n in nodes {
        sg.add_node(n);
    }
    for &n in nodes {
        for &e in &g.out[orign[n]] {
            let dh = dnode[g.edges[e].head];
            if let Some(de) = dg.find_edge(n, dh).filter(|_| dh != n) {
                sg.subedge(arena, de);
            }
        }
    }
    sg
}

/// `copyPosns(g)`: derived positions to the original nodes (`ORIGN`).
fn copy_posns(c: &Circ, g: &mut LGraph<'_>, nodes: &[usize], orign: &[usize]) {
    for &n in nodes {
        g.nodes[orign[n]].pos[0] = c.pos[n][0];
        g.nodes[orign[n]].pos[1] = c.pos[n][1];
    }
}

/// `packSubgraphs(ncc, ccs, dg, &pinfo)`: C packs the components of the
/// derived graph, whose edges are the original ones without loops and with
/// one edge per node pair; pack them over the original nodes with the
/// adjacency temporarily reduced to those edges.
fn pack_derived(
    g: &mut LGraph<'_>,
    d: &Derived,
    arena: &EdgeArena,
    ccs: &mut [Sub],
    pinfo: &PackInfo,
) {
    let n = g.nodes.len();
    let mut out = vec![Vec::new(); n];
    let mut inn = vec![Vec::new(); n];
    // the derived edge joins derived nodes, which the original edge's ends name
    let mut ends = Vec::new();
    for v in 0..n {
        for de in d.dg.outs(v) {
            let e = d.orig_edge[de];
            out[v].push(e);
            inn[arena.head[de]].push(e);
            ends.push((e, g.edges[e].tail, g.edges[e].head));
            g.edges[e].tail = v;
            g.edges[e].head = arena.head[de];
        }
    }
    for list in inn.iter_mut() {
        list.sort_by_key(|&e| (g.edges[e].tail, e));
    }
    let saved_out = std::mem::replace(&mut g.out, out);
    let saved_inn = std::mem::replace(&mut g.inn, inn);
    pack_graphs(g, ccs, pinfo);
    g.out = saved_out;
    g.inn = saved_inn;
    for (e, t, h) in ends {
        g.edges[e].tail = t;
        g.edges[e].head = h;
    }
}

/// The derived graph of `circomps` as `adjustNodes` reads it, or `None` when `adjustNodes`
/// would do nothing there. `agopen("derived", ...)` makes a root that copies cgraph's
/// ProtoGraph: it declares only the command-line attributes, and its nodes and edges hold
/// their defaults. With no `overlap`, `normalize` or `scale` among them, `removeOverlapWith`
/// leaves every component as it is. Node `n` is derived node `n`, and the edges are the
/// components' derived edges, each node's in `agfstout(sg, n)` order.
fn derived_src(g: &LGraph<'_>, arena: &EdgeArena, sgs: &[CGraph]) -> Option<Graph> {
    // `circomps` makes a strict graph, whose derived edges are already one per node pair.
    let mut src = g
        .src
        .protograph_root("derived", GraphKind::Undirected, false);
    if !["overlap", "normalize", "scale"]
        .iter()
        .any(|k| src.graph_attrs.contains_key(*k))
    {
        return None;
    }
    for n in 0..g.nodes.len() {
        src.ensure_node(&n.to_string());
        let attrs = src.node_defaults.clone();
        if let Some(nd) = src.nodes.last_mut() {
            nd.attrs = attrs;
        }
    }
    for sg in sgs {
        for v in sg.nodes() {
            for de in sg.outs(v) {
                let (t, h) = (NodeId(v as u32), NodeId(arena.other(de, v) as u32));
                let attrs = src.edge_defaults.clone();
                src.add_edge(t, h, attrs);
            }
        }
    }
    Some(src)
}

/// [`derived_src`] laid out as `circoLayout` has it before `adjustNodes`: the derived nodes
/// have the `ND_lw`, `ND_rw` and `ND_ht` that `makeDerivedNode` copies from their `ORIGN`,
/// and `ND_width` and `ND_height` 0, which the overlap removers read.
fn derived_lgraph<'s>(src: &'s Graph, g: &LGraph<'_>, orign: &[usize]) -> LGraph<'s> {
    let nodes = orign
        .iter()
        .zip(&g.nodes)
        .map(|(&o, n)| {
            let orig = g.nodes.get(o).unwrap_or(n);
            LNode {
                pos: n.pos.clone(),
                coord: Point::default(),
                width: 0.0,
                height: 0.0,
                lw: orig.lw,
                rw: orig.rw,
                ht: orig.ht,
                pinned: 0,
                id: 0,
                heapindex: -1,
                hops: 0,
                dist: 0.0,
                mark: false,
                // A derived node has no shape (`ND_shape` NULL); the only reader, makePoly
                // under voronoi and oscale, is turned off in `adjust_derived`.
                shape: orig.shape.clone(),
                xlabel: None,
                clust: None,
                lim: -1,
            }
        })
        .collect();
    let mut out = vec![Vec::new(); g.nodes.len()];
    let mut inn = vec![Vec::new(); g.nodes.len()];
    let edges: Vec<LEdge> = src
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (tail, head) = (e.tail.0 as usize, e.head.0 as usize);
            if let Some(l) = out.get_mut(tail) {
                l.push(i);
            }
            if let Some(l) = inn.get_mut(head) {
                l.push(i);
            }
            LEdge {
                tail,
                head,
                dist: 1.0,
                factor: 1.0,
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
    for list in inn.iter_mut() {
        list.sort_by_key(|&e| (edges.get(e).map(|x| x.tail), e));
    }
    LGraph {
        src,
        nodes,
        edges,
        out,
        inn,
        ndim: 2,
        odim: 0,
        nodesep: g.nodesep,
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
    }
}

/// `adjustNodes(sg)` on component `cc` of the derived graph `dg`, from and back to the
/// positions `circularLayout` gave it in `g`.
fn adjust_derived(dg: &mut LGraph<'_>, g: &mut LGraph<'_>, cc: &Sub) {
    // removeOverlapAs
    if cc.nodes.len() < 2 {
        return;
    }
    for &n in &cc.nodes {
        if let (Some(d), Some(o)) = (dg.nodes.get_mut(n), g.nodes.get(n)) {
            d.pos.clone_from(&o.pos);
        }
    }
    let mut am = get_adjust_mode(dg, dg.gattr("overlap").unwrap_or(""));
    if matches!(am.mode, AdjustMode::Vor | AdjustMode::Scale) {
        // C crashes: makeInfo's makePoly reads `ND_shape(n)->name` of a derived node, which
        // has no shape (docs/porting/crashes.md, "circo's derived nodes have no shape"). The
        // port leaves the overlaps, as in-file `overlap` values do here.
        am.mode = AdjustMode::None;
    }
    remove_overlap_with(dg, cc, &am);
    for &n in &cc.nodes {
        if let (Some(o), Some(d)) = (g.nodes.get_mut(n), dg.nodes.get(n)) {
            o.pos.clone_from(&d.pos);
        }
    }
}

/// `circoLayout(g)`.
fn circo_layout_main(g: &mut LGraph<'_>) {
    if g.nodes.is_empty() {
        return;
    }
    let nn = g.nodes.len();
    let mut arena = EdgeArena::default();
    let (dnode, orign) = derived_nodes(g);
    let d = derived_graph(g, &dnode, &mut arena);
    let mut c = Circ {
        nd: vec![CData::default(); nn],
        pos: vec![[0.0; 2]; nn],
        width: (0..nn).map(|n| g.nodes[orign[n]].width).collect(),
        height: (0..nn).map(|n| g.nodes[orign[n]].height).collect(),
        arena,
        blocks: Vec::new(),
        bl: Vec::new(),
        order_count: 1,
        min_dist: 0.0,
        rootname: None,
        n_root: false,
        node_root: (0..nn)
            .map(|n| {
                let v = g.nattr(orign[n], "root").unwrap_or("");
                !v.is_empty() && mapbool(Some(v))
            })
            .collect(),
        names: g.src.nodes.iter().map(|n| n.name.clone()).collect(),
    };
    // Without merged names, the components of the derived graph are those of g, in the same
    // order.
    let merged = dnode.iter().enumerate().any(|(v, &f)| v != f);
    let mut ccs = if merged {
        derived_comps(&d.dg, &c.arena, nn)
    } else {
        ccomps(g)
    };
    if ccs.len() == 1 {
        let sg = component(g, &d.dg, &c.arena, &ccs[0].nodes, &dnode, &orign);
        circular_layout(&mut c, &sg, g);
        copy_posns(&c, g, &ccs[0].nodes, &orign);
        let root = g.root_sub();
        adjust_nodes(g, &root);
    } else {
        let mut pinfo = PackInfo::default();
        get_pack_info(g, PackMode::Node, CL_OFFSET, &mut pinfo);
        // The port packs the derived nodes in place of the original ones: a merged node's
        // first node stands in for it with its ORIGN's size (makeDerivedNode copies `lw`,
        // `rw` and `ht`), and gets its own position and size back after packing.
        let identity: Vec<usize> = (0..nn).collect();
        let kept: Vec<(usize, Vec<f64>, [f64; 3])> = (0..nn)
            .filter(|&f| orign[f] != f)
            .map(|f| {
                let (n, o) = (&g.nodes[f], &g.nodes[orign[f]]);
                let kept = (f, n.pos.clone(), [n.lw, n.rw, n.ht]);
                let size = [o.lw, o.rw, o.ht];
                [g.nodes[f].lw, g.nodes[f].rw, g.nodes[f].ht] = size;
                kept
            })
            .collect();
        let sgs: Vec<CGraph> = ccs
            .iter()
            .map(|cc| component(g, &d.dg, &c.arena, &cc.nodes, &dnode, &orign))
            .collect();
        let derived = derived_src(g, &c.arena, &sgs);
        let mut dg = derived.as_ref().map(|src| derived_lgraph(src, g, &orign));
        for (cc, sg) in ccs.iter().zip(&sgs) {
            circular_layout(&mut c, sg, g);
            copy_posns(&c, g, &cc.nodes, &identity);
            if let Some(dg) = dg.as_mut() {
                adjust_derived(dg, g, cc);
            } else if graphviz_pure_core::diag::verbose_on() && cc.nodes.len() >= 2 {
                // adjustNodes(sg) with no overlap, normalize or scale: getAdjustMode still
                // reports the mode (none)
                let _ = get_adjust_mode(g, "");
            }
        }
        pack_derived(g, &d, &c.arena, &mut ccs, &pinfo);
        for (f, pos, [lw, rw, ht]) in kept {
            let packed = std::mem::replace(&mut g.nodes[f].pos, pos);
            g.nodes[orign[f]].pos = packed;
            [g.nodes[f].lw, g.nodes[f].rw, g.nodes[f].ht] = [lw, rw, ht];
        }
    }
}

/// `circo_layout(g)`.
pub fn circo_layout(graph: &mut Graph) -> Result<()> {
    if graph.nodes.is_empty() {
        crate::common::postproc::labels_at_origin(graph);
        return Ok(());
    }
    let src = graph.clone();
    // circo_init_graph: EDGETYPE_LINE by default, 2 dimensions.
    let mut g = LGraph::new(&src, 2, EdgeType::Line);
    circo_layout_main(&mut g);
    let mut root = g.root_sub();
    spline_edges(&mut g, &mut root, true);
    gv_postprocess(&mut g, true);
    g.write_back(graph);
    Ok(())
}
