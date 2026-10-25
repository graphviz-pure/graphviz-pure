//! Edges to clusters, ported from `lib/common/utils.c`: `processClusterEdges`,
//! `checkCompound`, `clustNode`, `cloneEdge`, `mapEdge`/`insertEdge` and
//! `undoClusterEdges`/`undoCompound`/`mapN`.
//!
//! A node whose name starts with `cluster` and equals a cluster's name stands
//! for that cluster. `processClusterEdges` deletes such nodes, adds one
//! invisible box node `__0:<cluster>` inside each named cluster (C's name
//! counter never advances) and re-attaches the edges to it. cgraph mutates
//! the graph in place; the port builds the modified graph as a new
//! [`Graph`] (`work`) and maps the layout back with [`undo_cluster_edges`].

use crate::fdpgen::ClusterTree;
use graphviz_pure_core::{AgName, AttrMap, EdgeId, Graph, NodeId, NodeLayout, Obj};
use std::collections::HashMap;

/// The graph with cluster edges replaced, and how it maps to the input.
#[derive(Debug, Clone)]
pub struct ClusterEdges {
    pub work: Graph,
    /// Input node → work node (`None` for deleted cluster-named nodes).
    node_map: Vec<Option<usize>>,
    /// Input edge → work edge (the edge itself or its clone).
    edge_map: Vec<Option<usize>>,
    /// Work nodes that are cluster nodes, with their cluster.
    pub clust_nodes: Vec<(usize, usize)>,
    /// `num_clust_edges(g)`.
    pub count: usize,
}

impl ClusterEdges {
    /// `IS_CLUST_NODE(n)` for a work node.
    pub fn is_clust_node(&self, n: usize) -> bool {
        self.clust_nodes.iter().any(|&(m, _)| m == n)
    }
}

/// `processClusterEdges(g)`; `None` when the graph has no cluster edges.
pub fn process_cluster_edges(graph: &Graph, tree: &ClusterTree) -> Option<ClusterEdges> {
    fn preorder<'g>(
        tree: &ClusterTree,
        cs: &[usize],
        graph: &'g Graph,
        out: &mut Vec<(AgName<'g>, usize)>,
    ) {
        for &c in cs {
            out.push((graph.agnameof(Obj::Subgraph(tree.subgraph[c])), c));
            preorder(tree, &tree.children[c], graph, out);
        }
    }
    let mut clusters = Vec::new();
    preorder(tree, &tree.root, graph, &mut clusters);
    let cmap = crate::common::utils::mk_clust_map(clusters);
    let mapc = |n: usize| -> Option<usize> {
        let name = graph.nodes[n].name.as_str();
        if name.starts_with("cluster") {
            cmap.get(name)
        } else {
            None
        }
    };
    let members: Vec<Vec<bool>> = (0..tree.len())
        .map(|c| {
            let mut v = vec![false; graph.nodes.len()];
            for n in crate::fdpgen::subgraph_member_nodes(graph, tree.subgraph[c]) {
                v[n] = true;
            }
            v
        })
        .collect();
    // agcontains(g, subg) (lib/cgraph/obj.c:241-245): g is subg or one of its ancestors.
    let mut sub_parent: Vec<Option<usize>> = vec![None; graph.subgraphs.len()];
    for (i, s) in graph.subgraphs.iter().enumerate() {
        for c in &s.subgraphs {
            sub_parent[c.0 as usize] = Some(i);
        }
    }
    let contains_subg = |g: usize, s: usize| {
        let g = tree.subgraph[g];
        let mut cur = Some(tree.subgraph[s]);
        while let Some(c) = cur {
            if c == g {
                return true;
            }
            cur = sub_parent[c];
        }
        false
    };

    // Endpoints of the work graph: input nodes, then cluster nodes (by cluster).
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum End {
        Node(usize),
        Clust(usize),
    }
    let mut clust_order: Vec<usize> = Vec::new();
    let clust_node = |c: usize, order: &mut Vec<usize>| {
        if !order.contains(&c) {
            order.push(c);
        }
        End::Clust(c)
    };
    let mut deleted = vec![false; graph.nodes.len()];
    let mut map: HashMap<(usize, usize), (End, End)> = HashMap::new();
    // Clones in creation order: (input edge, tail, head).
    let mut clones: Vec<(usize, End, End)> = Vec::new();
    let mut count = 0usize;

    let mut out: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for (i, e) in graph.edges.iter().enumerate() {
        out[e.tail.0 as usize].push(i);
    }
    for l in out.iter_mut() {
        l.sort_by_key(|&e| (graph.edges[e].head.0, e));
    }
    for n in 0..graph.nodes.len() {
        for &e in &out[n] {
            let t = graph.edges[e].tail.0 as usize;
            let h = graph.edges[e].head.0 as usize;
            let tg = mapc(t);
            let hg = mapc(h);
            if tg.is_none() && hg.is_none() {
                continue;
            }
            let nname = |n: usize| graph.agnameof(Obj::Node(NodeId(n as u32)));
            let cname = |c: usize| graph.agnameof(Obj::Subgraph(tree.subgraph[c]));
            // Each warning prints two agnameof names, which share its static buffer when
            // anonymous (utils.c:860-900).
            let warn = |fmt: fn(&str, &str) -> String, a: AgName<'_>, b: AgName<'_>| {
                let [a, b] = graphviz_pure_core::agnameof_buffer([a, b]);
                graphviz_pure_core::diag::agwarningf(&fmt(&a, &b));
            };
            if tg == hg {
                // utils.c:860-863 names the tail twice.
                warn(
                    |a, b| format!("cluster cycle {a} -- {b} not supported\n"),
                    nname(t),
                    nname(t),
                );
                continue;
            }
            if let Some(&(ct, ch)) = map.get(&(t, h)) {
                clones.push((e, ct, ch));
                count += 1;
                continue;
            }
            let (ct, ch) = match (tg, hg) {
                (Some(tc), Some(hc)) => {
                    if contains_subg(hc, tc) {
                        warn(
                            |a, b| format!("tail cluster {a} inside head cluster {b}\n"),
                            cname(tc),
                            cname(hc),
                        );
                        continue;
                    }
                    if contains_subg(tc, hc) {
                        warn(
                            |a, b| format!("head cluster {a} inside tail cluster {b}\n"),
                            cname(hc),
                            cname(tc),
                        );
                        continue;
                    }
                    let cn = clust_node(tc, &mut clust_order);
                    deleted[t] = true;
                    let cn1 = clust_node(hc, &mut clust_order);
                    deleted[h] = true;
                    (cn, cn1)
                }
                (None, Some(hc)) => {
                    if members[hc][t] {
                        warn(
                            |a, b| format!("tail node {a} inside head cluster {b}\n"),
                            nname(t),
                            cname(hc),
                        );
                        continue;
                    }
                    let cn = clust_node(hc, &mut clust_order);
                    deleted[h] = true;
                    (End::Node(t), cn)
                }
                (Some(tc), None) => {
                    if members[tc][h] {
                        warn(
                            |a, b| format!("head node {a} inside tail cluster {b}\n"),
                            nname(h),
                            cname(tc),
                        );
                        continue;
                    }
                    let cn = clust_node(tc, &mut clust_order);
                    deleted[t] = true;
                    (cn, End::Node(h))
                }
                (None, None) => unreachable!(),
            };
            clones.push((e, ct, ch));
            map.insert((t, h), (ct, ch));
            map.insert((h, t), (ch, ct));
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }

    // Build the work graph.
    let mut work = Graph::new(graph.name.clone(), graph.kind, graph.strict);
    // The work graph is the same cgraph: it keeps its objects' agnameof (the root's empty
    // name, and the anonymous ids of the subgraphs, which are copied one to one below).
    work.empty_name = graph.empty_name;
    work.anon_subgraph_ids = graph.anon_subgraph_ids.clone();
    work.graph_attrs = graph.graph_attrs.clone();
    // graph_init's `GD_drawing(g)->xdots`, which the layout and renderers read.
    work.layout.background = graph.layout.background.clone();
    work.node_defaults = graph.node_defaults.clone();
    work.edge_defaults = graph.edge_defaults.clone();
    // Its attribute dictionaries stay on it too; in particular the `-G` declarations
    // (`proto`), which the derived graph's `agopen` inherits as its protograph (`fdp
    // -Gnormalize` reads `normalize` there, lib/fdpgen/layout.c:854).
    work.dicts = graph.dicts.clone();
    let mut node_map = vec![None; graph.nodes.len()];
    for (i, nd) in graph.nodes.iter().enumerate() {
        if deleted[i] {
            continue;
        }
        let id = work.nodes.len();
        let mut d = nd.clone();
        d.id = NodeId(id as u32);
        work.node_lookup
            .insert(Graph::node_key(&d.name, d.html_name).into_owned(), d.id);
        work.nodes.push(d);
        node_map[i] = Some(id);
    }
    let mut clust_nodes = Vec::new();
    let mut input_named_nodes = Vec::new();
    let mut cnode_of: HashMap<usize, usize> = HashMap::new();
    for &c in &clust_order {
        // clustNode: `agxbprint(xb, "__%d:%s", idx, agnameof(cg))`.
        let name = format!("__0:{}", graph.subgraph_name(tree.subgraph[c]));
        // An input node of that name is the cluster node, and may be in subgraphs already.
        let input_named = work.agfindnode(&name, false).is_some();
        let id = work.ensure_node(&name);
        let a = &mut work.nodes[id.0 as usize].attrs;
        a.insert("label".into(), String::new());
        a.insert("style".into(), "invis".into());
        a.insert("shape".into(), "box".into());
        clust_nodes.push((id.0 as usize, c));
        input_named_nodes.push(input_named);
        cnode_of.insert(c, id.0 as usize);
    }
    let end_id = |x: End| -> usize {
        match x {
            End::Node(n) => node_map[n].expect("kept node"),
            End::Clust(c) => cnode_of[&c],
        }
    };
    let mut edge_map = vec![None; graph.edges.len()];
    for (i, ed) in graph.edges.iter().enumerate() {
        let (t, h) = (ed.tail.0 as usize, ed.head.0 as usize);
        let (Some(wt), Some(wh)) = (node_map[t], node_map[h]) else {
            continue;
        };
        let id = work.edges.len();
        let mut d = ed.clone();
        d.id = EdgeId(id as u32);
        d.tail = NodeId(wt as u32);
        d.head = NodeId(wh as u32);
        work.edges.push(d);
        edge_map[i] = Some(id);
    }
    for &(e, ct, ch) in &clones {
        let id = work.edges.len();
        let mut d = graph.edges[e].clone();
        d.id = EdgeId(id as u32);
        d.tail = NodeId(end_id(ct) as u32);
        d.head = NodeId(end_id(ch) as u32);
        work.edges.push(d);
        // A later clone of the same input edge (none in practice) wins.
        edge_map[e] = Some(id);
    }
    let emap_old: Vec<Option<usize>> = (0..graph.edges.len())
        .map(|i| {
            let ed = &graph.edges[i];
            if deleted[ed.tail.0 as usize] || deleted[ed.head.0 as usize] {
                None
            } else {
                edge_map[i]
            }
        })
        .collect();
    work.subgraphs = graph
        .subgraphs
        .iter()
        .map(|sg| {
            let mut s = sg.clone();
            s.nodes = sg
                .nodes
                .iter()
                .filter_map(|n| node_map[n.0 as usize].map(|m| NodeId(m as u32)))
                .collect();
            s.edges = sg
                .edges
                .iter()
                .filter_map(|e| emap_old[e.0 as usize].map(|m| EdgeId(m as u32)))
                .collect();
            s
        })
        .collect();
    // `agsubnode(cg, cn, 1)` installs the cluster node in `cg` and each ancestor, up to the
    // first that already holds it: only an input node of the same name can be there.
    for (&(n, c), &input_named) in clust_nodes.iter().zip(&input_named_nodes) {
        let mut s = Some(tree.subgraph[c]);
        while let Some(sg) = s.and_then(|si| work.subgraphs.get_mut(si)) {
            if input_named && sg.nodes.contains(&NodeId(n as u32)) {
                break;
            }
            sg.nodes.push(NodeId(n as u32));
            s = s.and_then(|si| sub_parent.get(si).copied().flatten());
        }
    }
    Some(ClusterEdges {
        work,
        node_map,
        edge_map,
        clust_nodes,
        count,
    })
}

/// The laid-out `work` graph as C's renderers draw it before `undoClusterEdges`: kept
/// objects keep their input sequence numbers; cluster nodes and cloned edges, created by
/// `processClusterEdges`, follow the input's last node and edge.
pub fn render_view(ce: &ClusterEdges, work: &Graph, input: &Graph) -> Graph {
    let mut v = work.clone();
    let mut node_seq = vec![0usize; v.nodes.len()];
    for (i, m) in ce.node_map.iter().enumerate() {
        if let Some(w) = *m {
            node_seq[w] = i + 1;
        }
    }
    let mut next = input.nodes.len();
    for s in node_seq.iter_mut().filter(|s| **s == 0) {
        next += 1;
        *s = next;
    }
    // Work edges: the kept input edges in input order, then the `count` clones.
    let kept = v.edges.len() - ce.count;
    let mut edge_seq = vec![0usize; v.edges.len()];
    for (i, m) in ce.edge_map.iter().enumerate() {
        if let Some(w) = *m
            && w < kept
        {
            edge_seq[w] = i + 1;
        }
    }
    for (k, s) in edge_seq[kept..].iter_mut().enumerate() {
        *s = input.edges.len() + k + 1;
    }
    for (e, s) in v.edges.iter_mut().zip(edge_seq) {
        e.layout.seq = Some(s);
    }
    for (n, s) in v.nodes.iter_mut().zip(node_seq) {
        n.layout.seq = Some(s);
    }
    v
}

/// `undoClusterEdges(g)`: copy the layout of `work` (after `write_back`)
/// into the input graph. Deleted cluster-named nodes come back with default
/// attributes and are marked as cluster nodes ([`NodeLayout::clust_node`]); their edges take the
/// geometry of the clones. Edges C deleted without a clone are removed.
pub fn undo_cluster_edges(ce: &ClusterEdges, work: &Graph, graph: &mut Graph) {
    for (i, m) in ce.node_map.iter().enumerate() {
        match *m {
            Some(w) => {
                let src = &work.nodes[w];
                let dst = &mut graph.nodes[i];
                dst.pos = src.pos;
                dst.width = src.width;
                dst.height = src.height;
                dst.attrs = src.attrs.clone();
                dst.layout = src.layout.clone();
            }
            None => {
                let mut a: AttrMap = graph.node_defaults.clone();
                if !a.contains_key("label") {
                    a.insert("label".to_string(), "\\N".to_string());
                }
                let node = &mut graph.nodes[i];
                node.attrs = a;
                node.layout = NodeLayout {
                    clust_node: true,
                    ..NodeLayout::default()
                };
                // mapN: `agnode(g, name, 1)` with the text after `__0:`, a plain string,
                // even for a node that was named `<cluster_x>`
                if std::mem::take(&mut node.html_name) {
                    let name = node.name.clone();
                    graph
                        .node_lookup
                        .remove(Graph::node_key(&name, true).as_ref());
                    graph.node_lookup.entry(name).or_insert(NodeId(i as u32));
                }
            }
        }
    }
    for (i, m) in ce.edge_map.iter().enumerate() {
        let Some(w) = *m else {
            continue;
        };
        let src = &work.edges[w];
        let dst = &mut graph.edges[i];
        dst.spline = src.spline.clone();
        dst.label_pos = src.label_pos;
        dst.label_unplaced = src.label_unplaced;
        dst.attrs = src.attrs.clone();
        dst.layout = src.layout.clone();
    }
    graph.bb = work.bb;
    for (dst, src) in graph.subgraphs.iter_mut().zip(&work.subgraphs) {
        dst.bb = src.bb;
        dst.graph_attrs = src.graph_attrs.clone();
        dst.layout = src.layout.clone();
    }
    graph.graph_attrs = work.graph_attrs.clone();
    graph.layout = work.layout.clone();
    // Edges of deleted nodes that were not re-attached are gone, as in C.
    let mut new_id = vec![None; graph.edges.len()];
    let mut kept = Vec::with_capacity(graph.edges.len());
    for (i, mut e) in std::mem::take(&mut graph.edges).into_iter().enumerate() {
        if ce.edge_map[i].is_some() {
            new_id[i] = Some(kept.len());
            e.id = EdgeId(kept.len() as u32);
            kept.push(e);
        }
    }
    graph.edges = kept;
    for sg in &mut graph.subgraphs {
        sg.edges = sg
            .edges
            .iter()
            .filter_map(|e| new_id[e.0 as usize].map(|n| EdgeId(n as u32)))
            .collect();
    }
    renumber_undone(ce, work, graph, &new_id);
}

/// Sequence order after `undoClusterEdges`: `undoCompound` re-creates each compound edge
/// (`cloneEdge`) after every other edge, visiting them in `agfstnode`/`agfstout` order of
/// the work graph, and `mapN` re-creates a cluster-named endpoint (in the root graph only)
/// after every other node. Out-edge iteration and the writers follow these sequences.
fn renumber_undone(ce: &ClusterEdges, work: &Graph, graph: &mut Graph, new_id: &[Option<usize>]) {
    let kept_work = work.edges.len() - ce.count;
    // Work edge -> final edge, for the clones.
    let mut from_clone: Vec<Option<usize>> = vec![None; work.edges.len()];
    for (i, m) in ce.edge_map.iter().enumerate() {
        if let (Some(w), Some(f)) = (*m, new_id[i])
            && w >= kept_work
        {
            from_clone[w] = Some(f);
        }
    }
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(); work.nodes.len()];
    for (i, e) in work.edges.iter().enumerate() {
        outs[e.tail.0 as usize].push(i);
    }
    let mut compound = Vec::new();
    for l in &mut outs {
        l.sort_by_key(|&e| (work.edges[e].head.0, e));
        compound.extend(l.iter().filter_map(|&e| from_clone[e]));
    }
    if compound.is_empty() {
        return;
    }
    // Nodes: the kept ones, then the cluster-named ones in mapN order (tail, then head).
    let deleted: Vec<bool> = ce.node_map.iter().map(Option::is_none).collect();
    let mut order: Vec<usize> = (0..graph.nodes.len()).filter(|&n| !deleted[n]).collect();
    for &e in &compound {
        for n in [
            graph.edges[e].tail.0 as usize,
            graph.edges[e].head.0 as usize,
        ] {
            if deleted[n] && !order.contains(&n) {
                order.push(n);
            }
        }
    }
    let mut node_id = vec![None; graph.nodes.len()];
    for (k, &n) in order.iter().enumerate() {
        node_id[n] = Some(NodeId(k as u32));
    }
    let mut old_nodes: Vec<Option<graphviz_pure_core::NodeData>> = std::mem::take(&mut graph.nodes)
        .into_iter()
        .map(Some)
        .collect();
    graph.node_lookup.clear();
    for (k, &n) in order.iter().enumerate() {
        let mut d = old_nodes[n].take().expect("each node once");
        d.id = NodeId(k as u32);
        graph
            .node_lookup
            .insert(Graph::node_key(&d.name, d.html_name).into_owned(), d.id);
        graph.nodes.push(d);
    }
    for sg in &mut graph.subgraphs {
        sg.nodes = sg
            .nodes
            .iter()
            .filter(|n| !deleted[n.0 as usize])
            .filter_map(|n| node_id[n.0 as usize])
            .collect();
    }
    // Edges: the others in order, then the compound ones.
    let is_compound: Vec<bool> = {
        let mut v = vec![false; graph.edges.len()];
        for &e in &compound {
            v[e] = true;
        }
        v
    };
    let eorder: Vec<usize> = (0..graph.edges.len())
        .filter(|&e| !is_compound[e])
        .chain(compound.iter().copied())
        .collect();
    let mut edge_id = vec![0u32; graph.edges.len()];
    for (k, &e) in eorder.iter().enumerate() {
        edge_id[e] = k as u32;
    }
    let mut old_edges: Vec<Option<graphviz_pure_core::EdgeData>> = std::mem::take(&mut graph.edges)
        .into_iter()
        .map(Some)
        .collect();
    for (k, &e) in eorder.iter().enumerate() {
        let mut d = old_edges[e].take().expect("each edge once");
        d.id = EdgeId(k as u32);
        d.tail = node_id[d.tail.0 as usize].expect("endpoint kept");
        d.head = node_id[d.head.0 as usize].expect("endpoint kept");
        graph.edges.push(d);
    }
    for sg in &mut graph.subgraphs {
        let mut v: Vec<EdgeId> = sg
            .edges
            .iter()
            .filter(|e| !is_compound[e.0 as usize])
            .map(|e| EdgeId(edge_id[e.0 as usize]))
            .collect();
        v.sort();
        sg.edges = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::{AttrDicts, AttrSym, GraphKind, SubgraphData, SubgraphId};

    fn subgraph(
        id: u32,
        name: &str,
        nodes: Vec<NodeId>,
        subgraphs: Vec<SubgraphId>,
    ) -> SubgraphData {
        SubgraphData {
            id: SubgraphId(id),
            name: name.to_string(),
            is_cluster: true,
            graph_attrs: AttrMap::new(),
            node_defaults: AttrMap::new(),
            edge_defaults: AttrMap::new(),
            nodes,
            edges: Vec::new(),
            subgraphs,
            bb: Default::default(),
            html_name: false,
            empty_name: false,
            layout: Default::default(),
        }
    }

    /// Issue #323: `clustNode`'s `agsubnode(cg, cn, 1)` installs the cluster node in `cg`
    /// and each of its ancestors, whose node lists hold their descendants' nodes.
    /// `digraph { subgraph cluster_a { subgraph cluster_b { x } } y -> cluster_b }`.
    #[test]
    fn cluster_node_is_in_every_ancestor_of_its_cluster() {
        let mut g = Graph::new("", GraphKind::Directed, false);
        let x = g.ensure_node("x");
        let y = g.ensure_node("y");
        let cb = g.ensure_node("cluster_b");
        g.add_edge(y, cb, AttrMap::new());
        g.subgraphs
            .push(subgraph(0, "cluster_a", vec![x], vec![SubgraphId(1)]));
        g.subgraphs
            .push(subgraph(1, "cluster_b", vec![x], Vec::new()));
        let tree = crate::fdpgen::mk_clusters(&g);
        let ce = process_cluster_edges(&g, &tree).expect("an edge to a cluster");
        let cn = ce
            .work
            .agfindnode("__0:cluster_b", false)
            .expect("cluster node");
        for s in &ce.work.subgraphs {
            assert!(s.nodes.contains(&cn), "{}: {:?}", s.name, s.nodes);
        }
    }

    /// An input node named like the cluster node is that node (`agnode(root, name, 1)`):
    /// `agsubnode` stops at the first subgraph that already holds it.
    #[test]
    fn cluster_node_named_in_the_input_is_in_each_subgraph_once() {
        let mut g = Graph::new("", GraphKind::Directed, false);
        let x = g.ensure_node("x");
        let named = g.ensure_node("__0:cluster_b");
        let y = g.ensure_node("y");
        let cb = g.ensure_node("cluster_b");
        g.add_edge(y, cb, AttrMap::new());
        g.subgraphs.push(subgraph(
            0,
            "cluster_a",
            vec![x, named],
            vec![SubgraphId(1)],
        ));
        g.subgraphs
            .push(subgraph(1, "cluster_b", vec![x, named], Vec::new()));
        let tree = crate::fdpgen::mk_clusters(&g);
        let ce = process_cluster_edges(&g, &tree).expect("an edge to a cluster");
        let cn = ce
            .work
            .agfindnode("__0:cluster_b", false)
            .expect("cluster node");
        for s in &ce.work.subgraphs {
            let n = s.nodes.iter().filter(|&&m| m == cn).count();
            assert_eq!(n, 1, "{}: {:?}", s.name, s.nodes);
        }
    }

    /// Issue #431: the work graph is the same cgraph, so it keeps the input's attribute
    /// dictionaries; `deriveGraph`'s `agopen` inherits the `-G` declarations as its
    /// protograph, where fdp's `normalize(cg)` reads `normalize`.
    #[test]
    fn work_graph_keeps_the_attribute_dictionaries() {
        let mut g = Graph::new("", GraphKind::Undirected, false);
        let x = g.ensure_node("x");
        let y = g.ensure_node("y");
        let cb = g.ensure_node("cluster_b");
        let b = g.ensure_node("b");
        g.add_edge(x, y, AttrMap::new());
        g.add_edge(x, cb, AttrMap::new());
        g.subgraphs
            .push(subgraph(0, "cluster_b", vec![cb, b], Vec::new()));
        g.dicts = vec![AttrDicts {
            graph: [(
                "normalize".to_string(),
                AttrSym {
                    defval: "30".to_string(),
                    proto: true,
                    ..AttrSym::default()
                },
            )]
            .into_iter()
            .collect(),
            ..AttrDicts::default()
        }];
        let tree = crate::fdpgen::mk_clusters(&g);
        let ce = process_cluster_edges(&g, &tree).expect("an edge to a cluster");
        assert_eq!(
            ce.work
                .protograph_attrs()
                .get("normalize")
                .map(String::as_str),
            Some("30")
        );
    }
}
