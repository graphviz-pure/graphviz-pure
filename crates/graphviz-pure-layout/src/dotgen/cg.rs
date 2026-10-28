//! The subset of cgraph (`lib/cgraph`) that dotgen uses: node/edge/subgraph
//! iteration in sequence order, membership tests, and the few mutations dot
//! performs on subgraphs (`agdelete` of a node from a cluster, `agsubedge`,
//! `agsubnode`) and on its private auxiliary graphs (newrank `Xg`, the
//! `checkLabelOrder` graph).

use super::Dot;
use super::types::*;

impl Dot<'_> {
    /// Creates a new empty cgraph root graph (`agopen`).
    pub fn cg_open(&mut self, name: &str) -> G {
        let g = self.graphs.len();
        self.graphs.push(GraphInfo {
            name: name.to_string(),
            root: g,
            next_anon_id: std::rc::Rc::new(std::cell::Cell::new(if name.starts_with('%') {
                3
            } else {
                1
            })),
            bound: true,
            ..GraphInfo::default()
        });
        g
    }

    /// `agsubg(g, name, 1)`: finds or creates a child subgraph.
    pub fn cg_subg(&mut self, g: G, name: &str) -> G {
        for &s in &self.graphs[g].subgs {
            if self.graphs[s].name == name {
                return s;
            }
        }
        let s = self.graphs.len();
        let root = self.graphs[g].root;
        let attrs = self.graphs[g].attrs.clone();
        self.graphs.push(GraphInfo {
            name: name.to_string(),
            root,
            cg_parent: Some(g),
            attrs,
            ..GraphInfo::default()
        });
        self.graphs[g].subgs.push(s);
        s
    }

    /// `agdelsubg` (only used for subgraphs with no remaining members).
    pub fn cg_delsubg(&mut self, g: G, s: G) {
        self.graphs[g].subgs.retain(|&x| x != s);
    }

    /// Nodes of `g` in `agfstnode`/`agnxtnode` order (snapshot).
    pub fn nodes_of(&self, g: G) -> Vec<N> {
        self.graphs[g].nodes.iter().copied().collect()
    }

    /// First node of `g` (`agfstnode`).
    pub fn agfstnode(&self, g: G) -> Option<N> {
        self.graphs[g].nodes.iter().next().copied()
    }

    /// `agnnodes(g)`
    pub fn agnnodes(&self, g: G) -> usize {
        self.graphs[g].nodes.len()
    }

    /// `agnedges(g)`
    pub fn agnedges(&self, g: G) -> usize {
        self.graphs[g].edges.len()
    }

    pub fn agcontains_node(&self, g: G, n: N) -> bool {
        self.graphs[g].nodes.contains(&n)
    }

    pub fn agcontains_edge(&self, g: G, e: E) -> bool {
        self.graphs[g].edges.contains(&e)
    }

    fn is_root(&self, g: G) -> bool {
        self.graphs[g].root == g
    }

    /// Out-edges of `n` in `g` (`agfstout`/`agnxtout` order).
    pub fn out_edges(&self, g: G, n: N) -> Vec<E> {
        if self.is_root(g) {
            self.nodes[n].cg_out.clone()
        } else {
            self.nodes[n]
                .cg_out
                .iter()
                .copied()
                .filter(|e| self.graphs[g].edges.contains(e))
                .collect()
        }
    }

    /// In-edges of `n` in `g` (`agfstin`/`agnxtin` order).
    pub fn in_edges(&self, g: G, n: N) -> Vec<E> {
        if self.is_root(g) {
            self.nodes[n].cg_in.clone()
        } else {
            self.nodes[n]
                .cg_in
                .iter()
                .copied()
                .filter(|e| self.graphs[g].edges.contains(e))
                .collect()
        }
    }

    /// `agfstout(g, n)`
    pub fn agfstout(&self, g: G, n: N) -> Option<E> {
        self.nodes[n]
            .cg_out
            .iter()
            .copied()
            .find(|e| self.is_root(g) || self.graphs[g].edges.contains(e))
    }

    /// `agfstin(g, n)`
    pub fn agfstin(&self, g: G, n: N) -> Option<E> {
        self.nodes[n]
            .cg_in
            .iter()
            .copied()
            .find(|e| self.is_root(g) || self.graphs[g].edges.contains(e))
    }

    /// `agfstedge`/`agnxtedge`: out-edges, then in-edges that are not loops.
    pub fn all_edges(&self, g: G, n: N) -> Vec<E> {
        let mut v = self.out_edges(g, n);
        for e in self.in_edges(g, n) {
            if self.edges[e].tail != n {
                v.push(e);
            }
        }
        v
    }

    /// `agdegree(g, n, in, out)`
    pub fn agdegree(&self, g: G, n: N, inn: bool, out: bool) -> usize {
        let mut d = 0;
        if inn {
            d += self.in_edges(g, n).len();
        }
        if out {
            d += self.out_edges(g, n).len();
        }
        d
    }

    /// Descendant subgraphs of `g` including `g`, post-order (as `agapply(.., false)`).
    fn subtree_postorder(&self, g: G, out: &mut Vec<G>) {
        for &s in &self.graphs[g].subgs {
            self.subtree_postorder(s, out);
        }
        out.push(g);
    }

    /// `agdelete(g, n)` for a node: removes the node image from `g` and all its
    /// subgraphs (and from the root entirely when `g` is a root).
    pub fn agdelnode(&mut self, g: G, n: N) {
        if !self.graphs[g].nodes.contains(&n) {
            return;
        }
        if self.is_root(g) {
            for e in self.all_edges(g, n) {
                self.agdeledge(g, e);
            }
        }
        let mut gs = Vec::new();
        self.subtree_postorder(g, &mut gs);
        for s in gs {
            if self.graphs[s].nodes.remove(&n) {
                let inc: Vec<E> = self.nodes[n]
                    .cg_out
                    .iter()
                    .chain(self.nodes[n].cg_in.iter())
                    .copied()
                    .collect();
                for e in inc {
                    self.graphs[s].edges.remove(&e);
                }
            }
        }
    }

    /// `agdeledge(g, e)`
    pub fn agdeledge(&mut self, g: G, e: E) {
        if !self.graphs[g].edges.contains(&e) {
            return;
        }
        let mut gs = Vec::new();
        self.subtree_postorder(g, &mut gs);
        for s in gs {
            self.graphs[s].edges.remove(&e);
        }
        if self.is_root(g) {
            let (t, h) = (self.edges[e].tail, self.edges[e].head);
            self.nodes[t].cg_out.retain(|&x| x != e);
            self.nodes[h].cg_in.retain(|&x| x != e);
        }
    }

    /// `agsubnode(g, n, 1)`: installs `n` in `g` and its ancestors.
    pub fn agsubnode(&mut self, g: G, n: N) {
        let mut cur = Some(g);
        while let Some(c) = cur {
            if !self.graphs[c].nodes.insert(n) {
                break;
            }
            cur = self.graphs[c].cg_parent;
        }
    }

    /// `agsubedge(g, e, 1)`: installs `e` (and its endpoints) in `g` and its ancestors.
    pub fn agsubedge(&mut self, g: G, e: E) {
        let (t, h) = (self.edges[e].tail, self.edges[e].head);
        self.agsubnode(g, t);
        self.agsubnode(g, h);
        let mut cur = Some(g);
        while let Some(c) = cur {
            if !self.graphs[c].edges.insert(e) {
                // cgraph found `e` in `c`'s dictionaries.
                self.agcontains_edge_hit(c, e);
                break;
            }
            cur = self.graphs[c].cg_parent;
        }
    }

    /// `agnode(g, name, 1)` creating a new cgraph node in `g`'s root.
    pub fn cg_new_node(&mut self, g: G, name: &str) -> N {
        let root = self.graphs[g].root;
        let n = self.nodes.len();
        self.nodes.push(NodeInfo {
            name: name.to_string(),
            cg_root: Some(root),
            ..NodeInfo::default()
        });
        self.agsubnode(g, n);
        n
    }

    /// `agnode(g, NULL, 1)`: a node named by the next anonymous id in `g`'s root.
    pub fn cg_new_anonymous_node(&mut self, g: G) -> N {
        let counter = self
            .graphs
            .get(g)
            .and_then(|graph| self.graphs.get(graph.root))
            .map(|graph| graph.next_anon_id.clone());
        let name = format!("%{}", counter.as_ref().map_or(0, |c| c.get()));
        if let Some(counter) = counter {
            counter.set(counter.get().wrapping_add(2));
        }
        self.cg_new_node(g, &name)
    }

    /// Finds a node of `g`'s root by name (`agnode(g, name, 0)` on a root).
    pub fn cg_find_node(&self, g: G, name: &str) -> Option<N> {
        self.graphs[g]
            .nodes
            .iter()
            .copied()
            .find(|&n| self.nodes[n].name == name)
    }

    /// `agnode(g, name, 1)`: find or create.
    pub fn cg_node(&mut self, g: G, name: &str) -> N {
        if let Some(n) = self.cg_find_node(g, name) {
            return n;
        }
        self.cg_new_node(g, name)
    }

    /// `agfindedge(g, t, h)` (`agedge(g, t, h, NULL, 0)`): for undirected
    /// semantics cgraph also matches `h -> t`; dot's private graphs are
    /// strict directed, so only `t -> h` is searched.
    pub fn cg_find_edge(&self, g: G, t: N, h: N) -> Option<E> {
        self.nodes[t]
            .cg_out
            .iter()
            .copied()
            .find(|&e| self.edges[e].head == h && self.graphs[g].edges.contains(&e))
    }

    /// `agfindedge(agroot, t, h)` on the input graph, as `arrow_flags` calls it.
    ///
    /// cgraph searches `h`'s `in_id` dictionary, a splay tree ordered by tail id and
    /// then edge id, with a key that compares equal to every edge from `t`
    /// (`agfindedge_by_key`). Among several `t -> h` edges it returns the first one on
    /// the search path, which depends on the tree's shape, not the one created first.
    /// Every search splays what it finds to the root, so the tree is kept between
    /// calls. In an undirected graph cgraph also tries `h -> t`.
    pub fn agfindedge_root(&self, t: N, h: N) -> Option<E> {
        let e = self.in_id_search(t, h);
        if e.is_none() && self.root_input.kind != graphviz_pure_core::GraphKind::Directed {
            return self.in_id_search(h, t);
        }
        e
    }

    fn in_id_search(&self, t: N, h: N) -> Option<E> {
        let t = self.cg_node_id(t);
        self.with_in_id(h, |dict| {
            let i = dict.find(&|&x: &E| t.cmp(&self.cg_in_id_key(x).0))?;
            Some(*dict.get(i))
        })?
    }

    /// The order of input node `n`'s cgraph id: its root node index.
    fn cg_node_id(&self, n: N) -> usize {
        self.cg_root_node
            .as_ref()
            .and_then(|m| m.get(n).copied())
            .unwrap_or(n)
    }

    /// The order of input edge `e` in its head's `in_id` dictionary: tail id, then edge id.
    fn cg_in_id_key(&self, e: E) -> (usize, u64) {
        self.edges.get(e).map_or((usize::MAX, 0), |ed| {
            (self.cg_node_id(ed.tail), self.cg_edge_id(e, ed.seq))
        })
    }

    /// The order of input edge `e`'s id. An anonymous edge's id counts up (`2k + 1`);
    /// the statement's `key` names the edge, whose id is then the address of the key
    /// string, above every anonymous id. Named edges are taken to grow in creation order
    /// too. A key starting with `%` (`LOCALNAMEPREFIX`) is not mapped, and the edge gets
    /// an anonymous id (`agmapnametoid`).
    fn cg_edge_id(&self, e: E, seq: u64) -> u64 {
        if self.cg_keyed(e) {
            seq | (1 << 62)
        } else {
            seq
        }
    }

    fn cg_keyed(&self, e: E) -> bool {
        self.input
            .edges
            .get(e)
            .and_then(|ed| ed.key.as_deref())
            .is_some_and(|k| !k.starts_with('%'))
    }

    fn cg_head(&self, e: E) -> Option<N> {
        self.edges.get(e).map(|ed| ed.head)
    }

    /// `agfindedge_by_key(root, t, h, AGTAG(e))` finding the input edge `e`.
    fn in_id_hit(&self, e: E) {
        let Some(h) = self.cg_head(e) else {
            return;
        };
        if self.cg_in_id.borrow().get(h).is_some_and(Option::is_some) {
            let k = self.cg_in_id_key(e);
            self.with_in_id(h, |d| d.find(&|&x: &E| k.cmp(&self.cg_in_id_key(x))));
        } else {
            self.cg_in_id_hits.borrow_mut().push(e);
        }
    }

    /// `agfindedge_by_key(g, t, h, AGTAG(e))` finding `e`, as `agsubedge` and
    /// `agcontains(g, e)` do: on the input root, it splays `e` in [`Dot::agfindedge_root`]'s
    /// dictionaries.
    pub fn agcontains_edge_hit(&self, g: G, e: E) {
        if self.concentrate && g == 0 && !self.comp && e < self.input.edges.len() {
            self.in_id_hit(e);
        }
    }

    /// Runs `f` on the root graph's `in_id` dictionary of input node `h`, building it
    /// first if needed. Node ids are the addresses of the node names, taken here to
    /// grow as the nodes were created. The parser creates each edge with `agedge`:
    /// for a named (`key`) edge, it first searches `g` and the root for it, and in an
    /// undirected graph also the other way round, in the tail's dictionary; then
    /// `installedge` searches for the edge (missing it) and inserts it. dot adds no
    /// edges to the root graph; the searches that find one come from `cccomps` and from
    /// `agsubedge`/`agcontains` ([`Dot::cg_in_id_hits`]).
    fn with_in_id<R>(&self, h: N, f: impl FnOnce(&mut crate::cdt::Cdt<E>) -> R) -> Option<R> {
        let mut dicts = self.cg_in_id.borrow_mut();
        if dicts.len() <= h {
            dicts.resize_with(h.saturating_add(1), || None);
        }
        let dict = dicts.get_mut(h)?.get_or_insert_with(|| {
            let mut d = crate::cdt::Cdt::new();
            let key = |e: E| self.cg_in_id_key(e);
            let search = |d: &mut crate::cdt::Cdt<E>, k: (usize, u64)| {
                d.find(&|&x: &E| k.cmp(&key(x)));
            };
            let real = self.input.edges.len();
            let node = self.nodes.get(h);
            let mut ins: Vec<E> = node
                .map(|n| n.cg_in.iter().copied().filter(|&e| e < real).collect())
                .unwrap_or_default();
            ins.sort_by_key(|&e| self.edges.get(e).map(|ed| ed.seq));
            let undirected = self.root_input.kind != graphviz_pure_core::GraphKind::Directed;
            // Named edges from `h`, probed the other way round in an undirected graph.
            let mut outs: Vec<E> = match node {
                Some(n) if undirected => n
                    .cg_out
                    .iter()
                    .copied()
                    .filter(|&e| e < real && self.cg_keyed(e) && self.cg_head(e) != Some(h))
                    .collect(),
                _ => Vec::new(),
            };
            outs.sort_by_key(|&e| self.edges.get(e).map(|ed| ed.seq));
            let mut events: Vec<(E, bool)> = ins.iter().map(|&e| (e, true)).collect();
            events.extend(outs.iter().map(|&e| (e, false)));
            events.sort_by_key(|&(e, _)| self.edges.get(e).map(|ed| ed.seq));
            for (e, into_h) in events {
                let Some(ed) = self.edges.get(e) else {
                    continue;
                };
                let id = self.cg_edge_id(e, ed.seq);
                if self.cg_keyed(e) {
                    // agedge probes `g`, then the root: both are the root unless the
                    // edge statement is in a subgraph.
                    let in_subgraph = self
                        .input
                        .subgraphs
                        .iter()
                        .any(|s| s.edges.iter().any(|x| x.0 as usize == e));
                    for _ in 0..if in_subgraph { 1 } else { 2 } {
                        if into_h {
                            search(&mut d, (self.cg_node_id(ed.tail), id));
                        }
                        if undirected && ed.tail == h {
                            search(&mut d, (self.cg_node_id(ed.head), id));
                        }
                    }
                }
                if into_h {
                    let k = key(e);
                    search(&mut d, k);
                    d.insert(e, &|&x: &E| k.cmp(&key(x)));
                }
            }
            if self.cg_in_id_packed {
                // graphviz_node_induce: the component's nodes in order, each one's
                // out-edges in order.
                ins.sort_by_key(|&e| (key(e).0, self.edges.get(e).map(|ed| ed.seq)));
                for &e in &ins {
                    search(&mut d, key(e));
                }
            }
            for &e in self
                .cg_in_id_hits
                .borrow()
                .iter()
                .filter(|&&e| self.cg_head(e) == Some(h))
            {
                search(&mut d, key(e));
            }
            d
        });
        Some(f(dict))
    }

    /// `agedge(g, t, h, NULL, 1)` on a strict directed graph.
    pub fn cg_edge(&mut self, g: G, t: N, h: N) -> E {
        if let Some(e) = self.cg_find_edge(g, t, h) {
            return e;
        }
        let root = self.graphs[g].root;
        // A strict graph also refuses an edge that exists in the root.
        if let Some(e) = self.cg_find_edge(root, t, h) {
            self.agsubedge(g, e);
            return e;
        }
        let e = self.edges.len();
        self.edge_seq += 1;
        self.edges.push(EdgeInfo {
            tail: t,
            head: h,
            seq: self.edge_seq,
            ..EdgeInfo::default()
        });
        self.cg_insert_out(t, e);
        self.cg_insert_in(h, e);
        self.agsubedge(g, e);
        e
    }

    /// Key of an out-edge in cgraph's `agedgeseqcmpf` order.
    pub fn out_key(&self, e: E) -> (N, u64) {
        (self.edges[e].head, self.edges[e].seq)
    }

    fn cg_insert_out(&mut self, t: N, e: E) {
        let k = self.out_key(e);
        let pos = self.nodes[t]
            .cg_out
            .iter()
            .position(|&x| self.out_key(x) > k)
            .unwrap_or(self.nodes[t].cg_out.len());
        self.nodes[t].cg_out.insert(pos, e);
    }

    fn cg_insert_in(&mut self, h: N, e: E) {
        let k = (self.edges[e].tail, self.edges[e].seq);
        let pos = self.nodes[h]
            .cg_in
            .iter()
            .position(|&x| (self.edges[x].tail, self.edges[x].seq) > k)
            .unwrap_or(self.nodes[h].cg_in.len());
        self.nodes[h].cg_in.insert(pos, e);
    }
}
