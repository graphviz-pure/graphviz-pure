//! `cccomps` (lib/pack/ccomps.c): connected components where nodes are
//! connected by an edge or by sharing a top-level cluster, with the root's
//! subgraphs projected into each component (`subGInduce`/`projectG`).
//!
//! Works on the parsed [`graphviz_pure_core::Graph`] (the cgraph objects). Each component
//! can be turned into a standalone [`Graph`] that an engine lays out exactly as
//! C lays out the component subgraph `_cc_<i>`.

use crate::common::utils::mapbool;
use graphviz_pure_core::{AgName, EdgeId, Graph, NodeId, Obj, SubgraphData, SubgraphId};
use std::collections::BTreeSet;

/// A subgraph projected into a component (`projectG`).
#[derive(Debug, Clone)]
pub struct ProjSub {
    /// Index of the original subgraph in `Graph::subgraphs` (`mapClust`).
    pub orig: usize,
    /// Projected parent (index into [`CComp::subgraphs`]); `None` for the component.
    pub parent: Option<usize>,
    /// Nodes of the original subgraph (and its descendants) in the component.
    pub nodes: Vec<usize>,
    /// Edges of the original subgraph (and its descendants) in the component.
    pub edges: Vec<usize>,
}

/// One component `_cc_<i>`.
#[derive(Debug, Clone, Default)]
pub struct CComp {
    /// Root node indices, in sequence order.
    pub nodes: Vec<usize>,
    /// Projected subgraphs in creation order (preorder, parents first).
    pub subgraphs: Vec<ProjSub>,
    /// The root's own subgraph named `_cc_<i>`, which `agsubg(g, "_cc_<i>", 1)` returns
    /// instead of making one (#335): the component is that subgraph, with its graph
    /// attributes and its subgraphs, and `doDot` deletes it after packing.
    pub user: Option<usize>,
    /// How many of [`CComp::subgraphs`], from the start, are the user's subgraph's own.
    pub user_subgraphs: usize,
}

/// A component turned into a standalone graph, with the maps back to the root.
pub struct CompGraph {
    pub graph: Graph,
    /// Local node index -> root node index.
    pub nodes: Vec<usize>,
    /// Local edge index -> root edge index.
    pub edges: Vec<usize>,
    /// Local subgraph index -> original subgraph index.
    pub subgraphs: Vec<usize>,
}

/// `is_a_cluster(g)` for a subgraph.
pub fn is_a_cluster(s: &SubgraphData) -> bool {
    graphviz_pure_core::has_cluster_prefix(&s.name)
        || mapbool(s.graph_attrs.get("cluster").map(String::as_str))
}

/// What a derived node stands for (`ccgnodeinfo_t.ptr`).
#[derive(Debug, Clone, Copy)]
enum DPtr {
    Node(usize),
    Clust(usize),
}

/// The derived graph `dg` of `deriveGraph`: nodes by name, in creation order.
#[derive(Default)]
struct DerivedGraph {
    by_name: std::collections::HashMap<String, usize>,
    ptr: Vec<DPtr>,
}

impl DerivedGraph {
    /// `agnode(dg, name, 1)`.
    fn agnode(&mut self, name: &str) -> usize {
        if let Some(&d) = self.by_name.get(name) {
            return d;
        }
        let d = self.ptr.len();
        self.by_name.insert(name.to_string(), d);
        self.ptr.push(DPtr::Node(usize::MAX));
        d
    }
}

struct Tree<'a> {
    g: &'a Graph,
    /// Top-level subgraphs (`agfstsubg(root)`), in creation order.
    top: Vec<usize>,
    /// All nodes of each subgraph, nested ones included.
    nodes: Vec<BTreeSet<usize>>,
    /// All edges of each subgraph, nested ones included.
    edges: Vec<BTreeSet<usize>>,
    /// The top-level subgraphs named `_cc_<i>` (a plain string, `i` as `%d` prints it),
    /// which `agsubg(root, "_cc_<i>", 1)` finds: by `i`, and `i` by subgraph.
    cc_by_index: std::collections::HashMap<usize, usize>,
    cc_index_of: std::collections::HashMap<usize, usize>,
}

impl<'a> Tree<'a> {
    fn new(g: &'a Graph) -> Self {
        let ns = g.subgraphs.len();
        let mut is_child = vec![false; ns];
        for s in &g.subgraphs {
            for c in &s.subgraphs {
                is_child[c.0 as usize] = true;
            }
        }
        let top: Vec<usize> = (0..ns).filter(|&i| !is_child[i]).collect();
        let mut cc_by_index = std::collections::HashMap::new();
        let mut cc_index_of = std::collections::HashMap::new();
        for &s in &top {
            let Some(sg) = g.subgraphs.get(s) else {
                continue;
            };
            let i = sg
                .name
                .strip_prefix("_cc_")
                .and_then(|d| d.parse::<usize>().ok())
                .filter(|i| sg.name == format!("_cc_{i}"));
            if let Some(i) = i.filter(|_| !sg.html_name && !sg.is_anonymous()) {
                cc_by_index.entry(i).or_insert(s);
                cc_index_of.entry(s).or_insert(i);
            }
        }
        // Each subgraph's lists already hold its nested subgraphs' (`agsubnode`).
        Tree {
            g,
            top,
            nodes: g
                .subgraphs
                .iter()
                .map(|s| s.nodes.iter().map(|n| n.0 as usize).collect())
                .collect(),
            edges: g
                .subgraphs
                .iter()
                .map(|s| s.edges.iter().map(|e| e.0 as usize).collect())
                .collect(),
            cc_by_index,
            cc_index_of,
        }
    }

    /// The subgraph of the root that `agsubg(root, "_cc_<i>", 1)` finds: a top-level
    /// subgraph with that name as a plain string (an HTML name is another string).
    fn cc_subgraph(&self, i: usize) -> Option<usize> {
        self.cc_by_index.get(&i).copied()
    }

    /// `deriveClusters(dg, g)`: a derived node per top-level cluster, which
    /// every node of the cluster maps to.
    fn derive_clusters(&self, subs: &[usize], dg: &mut DerivedGraph, dn_of: &mut [Option<usize>]) {
        for &s in subs {
            let sg = &self.g.subgraphs[s];
            if is_a_cluster(sg) {
                let name = self.g.agnameof(Obj::Subgraph(s));
                let dn = dg.agnode(&name.name);
                dg.ptr[dn] = DPtr::Clust(s);
                for &n in &self.nodes[s] {
                    if let Some(old) = dn_of[n] {
                        let old_name = dg
                            .by_name
                            .iter()
                            .find(|(_, d)| **d == old)
                            .map_or("", |(k, _)| k.as_str());
                        // fprintf(stderr, ...): not agerr, so not filtered, counted or escaped.
                        // ccomps.c:275-277: the node's and the cluster's names share agnameof's
                        // static buffer when anonymous. The derived node keeps the name it was
                        // made with in `dg`'s internal map, which agnameof returns as it is.
                        let [node, sub, old] = graphviz_pure_core::agnameof_buffer([
                            self.g.agnameof(Obj::Node(NodeId(n as u32))),
                            name.clone(),
                            AgName::own(old_name),
                        ]);
                        graphviz_pure_core::diag::fprintf_stderr(&format!(
                            "Error: node \"{node}\" belongs to two non-nested clusters \"{sub}\" and \"{old}\"\n"
                        ));
                    }
                    dn_of[n] = Some(dn);
                }
            } else {
                let kids: Vec<usize> = sg.subgraphs.iter().map(|c| c.0 as usize).collect();
                self.derive_clusters(&kids, dg, dn_of);
            }
        }
    }

    /// `subgInduce(root, g, inCluster)` over the children `subs` of `parent`.
    fn subg_induce(
        &self,
        subs: &[usize],
        parent: Option<usize>,
        in_g: &BTreeSet<usize>,
        in_cluster: bool,
        out: &mut Vec<ProjSub>,
    ) {
        for &s in subs {
            // projectG finds each node of `subg` in `g` by `agfindnode(g, agnameof(n))`. An
            // anonymous node's `%<id>` is a local name, which `endgraph` removed from the
            // internal map, so the lookup fails and the node is not projected.
            let nodes: Vec<usize> = self.nodes[s]
                .iter()
                .copied()
                .filter(|&n| {
                    in_g.contains(&n)
                        && self
                            .g
                            .nodes
                            .get(n)
                            .is_some_and(|d| self.g.agfindnode(&d.name, d.html_name).is_some())
                })
                .collect();
            if nodes.is_empty() && !in_cluster {
                continue;
            }
            let member: BTreeSet<usize> = nodes.iter().copied().collect();
            let edges = self.edges[s]
                .iter()
                .copied()
                .filter(|&e| {
                    let ed = &self.g.edges[e];
                    member.contains(&(ed.tail.0 as usize)) && member.contains(&(ed.head.0 as usize))
                })
                .collect();
            let me = out.len();
            out.push(ProjSub {
                orig: s,
                parent,
                nodes,
                edges,
            });
            let kids: Vec<usize> = self.g.subgraphs[s]
                .subgraphs
                .iter()
                .map(|c| c.0 as usize)
                .collect();
            let inc = in_cluster || is_a_cluster(&self.g.subgraphs[s]);
            self.subg_induce(&kids, Some(me), &member, inc, out);
        }
    }
}

/// `cccomps(g, &ncc, pfx)`.
pub fn cccomps(g: &Graph) -> Vec<CComp> {
    let nn = g.nodes.len();
    if nn == 0 {
        return Vec::new();
    }
    let tree = Tree::new(g);
    // deriveGraph: cluster nodes first, then the remaining nodes. Derived nodes
    // are found by name (`agnode(dg, name, 1)`), so a node named like a
    // cluster shares the cluster's derived node and replaces its pointer.
    let mut dg = DerivedGraph::default();
    let mut dn_of: Vec<Option<usize>> = vec![None; nn];
    tree.derive_clusters(&tree.top, &mut dg, &mut dn_of);
    for n in 0..nn {
        if dn_of[n].is_some() {
            continue;
        }
        let dn = dg.agnode(&g.nodes[n].name);
        dg.ptr[dn] = DPtr::Node(n);
        dn_of[n] = Some(dn);
    }
    let ndn = dg.ptr.len();
    let dn_of: Vec<usize> = dn_of.into_iter().map(Option::unwrap).collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); ndn];
    for e in &g.edges {
        let (t, h) = (dn_of[e.tail.0 as usize], dn_of[e.head.0 as usize]);
        if t != h {
            adj[t].push(h);
            adj[h].push(t);
        }
    }
    // unionNodes: what each derived node contributes to its component.
    let members: Vec<Vec<usize>> = dg
        .ptr
        .iter()
        .map(|p| match *p {
            DPtr::Node(n) => vec![n],
            DPtr::Clust(s) => tree.nodes[s].iter().copied().collect(),
        })
        .collect();
    let mut mark = vec![false; ndn];
    let mut out = Vec::new();
    for d0 in 0..ndn {
        if mark[d0] {
            continue;
        }
        let mut stack = vec![d0];
        mark[d0] = true;
        let mut set: BTreeSet<usize> = BTreeSet::new();
        // dfs's count: the derived nodes of the component
        let mut n_cnt = 0usize;
        while let Some(d) = stack.pop() {
            n_cnt = n_cnt.wrapping_add(1);
            set.extend(members[d].iter().copied());
            for &o in &adj[d] {
                if !mark[o] {
                    mark[o] = true;
                    stack.push(o);
                }
            }
        }
        // `out = agsubg(g, "_cc_<i>", 1)` finds a subgraph of the root by that name. Its
        // own subgraphs come first (they are older than subGInduce's projections), and
        // subgInduce skips every `_cc_<j>` cccomps has made or taken so far (GD_cc_subg).
        // A node of the user's subgraph outside the component is in two components, which
        // share C's node record; the port, as for #101, lays the component out without it.
        let ci = out.len();
        let user = tree.cc_subgraph(ci);
        let mut subgraphs = Vec::new();
        if let Some(u) = user.and_then(|u| g.subgraphs.get(u)) {
            let kids: Vec<usize> = u.subgraphs.iter().map(|c| c.0 as usize).collect();
            tree.subg_induce(&kids, None, &set, false, &mut subgraphs);
        }
        let user_subgraphs = subgraphs.len();
        let top: Vec<usize> = tree
            .top
            .iter()
            .copied()
            .filter(|s| tree.cc_index_of.get(s).is_none_or(|&j| j > ci))
            .collect();
        tree.subg_induce(&top, None, &set, false, &mut subgraphs);
        if graphviz_pure_core::diag::verbose_on() {
            // graphviz_node_induce's count: the root's edges between the component's nodes
            let e_cnt = g
                .edges
                .iter()
                .filter(|e| {
                    set.contains(&(e.tail.0 as usize)) && set.contains(&(e.head.0 as usize))
                })
                .count();
            graphviz_pure_core::diag::fprintf_stderr(&format!(
                "({ci:4}) {n_cnt:7} nodes {e_cnt:7} edges\n"
            ));
        }
        out.push(CComp {
            nodes: set.into_iter().collect(),
            subgraphs,
            user,
            user_subgraphs,
        });
    }
    graphviz_pure_core::verbose_msg!(
        "       {:7} nodes {:7} edges {:7} components {}\n",
        nn,
        g.edges.len(),
        out.len(),
        g.root_name()
    );
    out
}

impl CComp {
    /// The component as a standalone graph: the root's graph attributes (a
    /// new subgraph inherits the root's values), its nodes and induced edges
    /// in sequence order, and the projected subgraphs (`agcopyattr`).
    pub fn to_graph(&self, root: &Graph) -> CompGraph {
        let mut graph = Graph::new(root.name.clone(), root.kind, root.strict);
        // A component is a subgraph of the root: its nodes are the root's, named as there
        // (`\G` is the root's agnameof), and its subgraphs are projectG's copies, which keep
        // their names as strings.
        graph.empty_name = root.empty_name;
        graph.local_subgraph_names_kept = true;
        graph.graph_attrs = root.graph_attrs.clone();
        // The user's `_cc_<i>` is the component: dot reads its values (`splines`, `phase`,
        // `rank`, `ordering`, ... with agget(sg)); initSubg copies the rest from the root.
        // Its values are those it had when made: an attribute the root declared later is
        // `""` there (agattr gives existing subgraphs the default), whatever the root set.
        if let Some(u) = self.user.and_then(|u| root.subgraphs.get(u)) {
            graph.graph_attrs = u.graph_attrs.clone();
            for k in root.graph_attrs.keys() {
                if !u.graph_attrs.contains_key(k) {
                    graph.graph_attrs.insert(k.clone(), String::new());
                }
            }
        }
        graph.node_defaults = root.node_defaults.clone();
        graph.edge_defaults = root.edge_defaults.clone();
        // A component is a subgraph of the root (`agsubg`), so it shares the root's
        // attribute dictionaries and symbol ids. make_flat_adj_edges' setState keeps some
        // main-graph Agsyms and reads them by id; it needs the root's declarations.
        graph.dicts = root.dicts.first().cloned().into_iter().collect();
        let mut local = vec![usize::MAX; root.nodes.len()];
        for (i, &n) in self.nodes.iter().enumerate() {
            local[n] = i;
            let mut nd = root.nodes[n].clone();
            nd.id = NodeId(i as u32);
            graph
                .node_lookup
                .insert(Graph::node_key(&nd.name, nd.html_name).into_owned(), nd.id);
            graph.nodes.push(nd);
        }
        let mut edges = Vec::new();
        let mut elocal = vec![usize::MAX; root.edges.len()];
        for (ei, e) in root.edges.iter().enumerate() {
            // graphviz_node_induce: both ends in the component.
            let (t, h) = (local[e.tail.0 as usize], local[e.head.0 as usize]);
            if t == usize::MAX || h == usize::MAX {
                continue;
            }
            let mut ed = e.clone();
            elocal[ei] = edges.len();
            ed.id = EdgeId(edges.len() as u32);
            ed.tail = NodeId(t as u32);
            ed.head = NodeId(h as u32);
            graph.edges.push(ed);
            edges.push(ei);
        }
        for (i, p) in self.subgraphs.iter().enumerate() {
            let s = &root.subgraphs[p.orig];
            graph.subgraphs.push(SubgraphData {
                id: SubgraphId(i as u32),
                name: s.name.clone(),
                is_cluster: s.is_cluster,
                graph_attrs: s.graph_attrs.clone(),
                node_defaults: s.node_defaults.clone(),
                edge_defaults: s.edge_defaults.clone(),
                nodes: p.nodes.iter().map(|&n| NodeId(local[n] as u32)).collect(),
                edges: p.edges.iter().map(|&e| EdgeId(elocal[e] as u32)).collect(),
                subgraphs: Vec::new(),
                bb: Default::default(),
                html_name: s.html_name,
                empty_name: s.empty_name,
                layout: Default::default(),
            });
        }
        for (i, p) in self.subgraphs.iter().enumerate() {
            // `projectG` names the copy `agnameof(subg)`: an anonymous original's `%<id>`.
            if let Some(&id) = root.anon_subgraph_ids.get(&(p.orig as u32)) {
                graph.anon_subgraph_ids.insert(i as u32, id);
            }
            if let Some(par) = p.parent {
                graph.subgraphs[par].subgraphs.push(SubgraphId(i as u32));
            }
        }
        CompGraph {
            graph,
            nodes: self.nodes.clone(),
            edges,
            subgraphs: self.subgraphs.iter().map(|p| p.orig).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::{AttrMap, GraphKind};

    /// `digraph { subgraph {cluster=true; a} subgraph {cluster=true; b} }` with the
    /// anonymous ids `%3` and `%5`.
    fn two_anonymous_clusters() -> Graph {
        let mut g = Graph::new("", GraphKind::Directed, false);
        for (i, n) in ["a", "b"].into_iter().enumerate() {
            let id = g.ensure_node(n);
            g.subgraphs.push(SubgraphData {
                id: SubgraphId(i as u32),
                name: String::new(),
                is_cluster: false,
                graph_attrs: AttrMap::from_plain([("cluster".to_string(), "true".to_string())]),
                node_defaults: AttrMap::new(),
                edge_defaults: AttrMap::new(),
                nodes: vec![id],
                edges: Vec::new(),
                subgraphs: Vec::new(),
                bb: Default::default(),
                html_name: false,
                empty_name: false,
                layout: Default::default(),
            });
            g.anon_subgraph_ids.insert(i as u32, 3 + 2 * i as u64);
        }
        g
    }

    // Issue #345: deriveClusters names a cluster's derived node `agnameof(subg)`, so two
    // anonymous clusters are two derived nodes, and two components.
    #[test]
    fn anonymous_clusters_are_derived_nodes_of_their_own() {
        let g = two_anonymous_clusters();
        let ccs = cccomps(&g);
        assert_eq!(ccs.len(), 2);
        assert_eq!(ccs[0].nodes, [0]);
        assert_eq!(ccs[1].nodes, [1]);
    }

    // Issue #345: projectG names the copy `agnameof(subg)`, the original's `%<id>`.
    #[test]
    fn projections_keep_the_anonymous_id_of_the_original() {
        let g = two_anonymous_clusters();
        let ccs = cccomps(&g);
        let cg = ccs[1].to_graph(&g);
        assert_eq!(cg.graph.subgraph_name(0), "%5");
        // The copy keeps the name as a string of its own (not agnameof's static buffer), and
        // the component's nodes are the root's, named as there: an empty-named root's `\G`
        // is "", not `%1`.
        assert_eq!(cg.graph.agnameof(Obj::Subgraph(0)), AgName::own("%5"));
        let mut g = g;
        g.empty_name = true;
        let cg = ccs[1].to_graph(&g);
        assert_eq!(cg.graph.agnameof(Obj::Root), AgName::own(""));
    }
}
