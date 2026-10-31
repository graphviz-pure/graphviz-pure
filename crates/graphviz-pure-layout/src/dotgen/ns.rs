//! dot's binding of `lib/common/ns.c` (network simplex, `rank`/`rank2`).
//!
//! The algorithm lives in [`crate::common::ns`], generic over [`NsGraph`].
//! This adapter copies each ranked graph's hot `NodeInfo`/`EdgeInfo` fields
//! into reusable flat arrays, numbered locally to the run, and gives the mutable
//! tree lists fixed-capacity storage. It writes the results back when `rank2` returns.

use super::Dot;
use super::types::*;
use super::utils::atoi;
use crate::common::ns::{self, NsGraph, SEARCHSIZE};

#[derive(Default)]
struct Adjacency {
    offsets: Vec<u32>,
    capacities: Vec<u32>,
    tree: Vec<E>,
    tree_len: Vec<u32>,
}

impl Adjacency {
    /// Empties the lists: the local numbering is new at every run.
    fn resize_nodes(&mut self, n: usize) {
        self.offsets.clear();
        self.offsets.resize(n, 0);
        self.capacities.clear();
        self.capacities.resize(n, 0);
        self.tree_len.clear();
        self.tree_len.resize(n, 0);
        self.tree.clear();
    }

    /// Room for lists that add up to `capacity`, before [`Adjacency::prepare`] hands it out.
    fn reserve(&mut self, capacity: usize) {
        self.tree.reserve_exact(capacity);
    }

    /// Gives node `n` room for `capacity` tree edges, at the end of the storage.
    fn prepare(&mut self, n: usize, capacity: usize) {
        let start = self.tree.len();
        if let (Some(offset), Some(stored)) = (self.offsets.get_mut(n), self.capacities.get_mut(n))
        {
            *offset = start as u32;
            *stored = capacity as u32;
        } else {
            debug_assert!(false, "adjacency node out of bounds");
            return;
        }
        self.tree.resize(start.wrapping_add(capacity), 0);
    }

    fn bounds(&self, n: usize) -> Option<(usize, usize)> {
        let start = self.offsets.get(n).copied()? as usize;
        let len = self.tree_len.get(n).copied()? as usize;
        Some((start, len))
    }

    fn edges(&self, n: usize) -> &[E] {
        let Some((start, len)) = self.bounds(n) else {
            debug_assert!(false, "adjacency node out of bounds");
            return &[];
        };
        let end = start.wrapping_add(len);
        match self.tree.get(start..end) {
            Some(edges) => edges,
            None => {
                debug_assert!(false, "adjacency segment out of bounds");
                &[]
            }
        }
    }

    fn push(&mut self, n: usize, e: E) -> bool {
        let Some(len) = self.tree_len.get(n).copied() else {
            debug_assert!(false, "adjacency node out of bounds");
            return false;
        };
        let Some(capacity) = self.capacities.get(n).copied() else {
            debug_assert!(false, "adjacency node out of bounds");
            return false;
        };
        let len = len as usize;
        if len >= capacity as usize {
            return false;
        }
        let Some(start) = self.offsets.get(n).copied() else {
            debug_assert!(false, "adjacency node out of bounds");
            return false;
        };
        let i = (start as usize).wrapping_add(len);
        let Some(slot) = self.tree.get_mut(i) else {
            debug_assert!(false, "adjacency segment out of bounds");
            return false;
        };
        *slot = e;
        if let Some(stored) = self.tree_len.get_mut(n) {
            *stored = len.wrapping_add(1) as u32;
        }
        true
    }

    fn remove(&mut self, n: usize, e: E) {
        let Some((start, len)) = self.bounds(n) else {
            debug_assert!(false, "adjacency node out of bounds");
            return;
        };
        let end = start.wrapping_add(len);
        let Some(segment) = self.tree.get_mut(start..end) else {
            debug_assert!(false, "adjacency segment out of bounds");
            return;
        };
        if let Some(i) = segment.iter().position(|&x| x == e) {
            segment.swap(i, len.wrapping_sub(1));
            if let Some(stored) = self.tree_len.get_mut(n) {
                *stored = len.wrapping_sub(1) as u32;
            }
        }
    }

    fn clear(&mut self, n: usize) {
        if let Some(len) = self.tree_len.get_mut(n) {
            *len = 0;
        }
    }
}

/// An edge or subtree index in a `u32` column.
const NONE: u32 = u32::MAX;
/// `ND_par` when the run began, if it names an edge outside this graph. C only overwrites it.
const FOREIGN: u32 = u32::MAX - 1;

#[inline]
fn pack(v: Option<usize>) -> u32 {
    v.map_or(NONE, |v| v as u32)
}

#[inline]
fn unpack(v: u32) -> Option<usize> {
    (v != NONE).then_some(v as usize)
}

/// A list per node, in one array: `list[offsets[n]..offsets[n + 1]]`.
#[derive(Default)]
struct Lists {
    offsets: Vec<u32>,
    list: Vec<E>,
}

impl Lists {
    fn clear(&mut self) {
        self.offsets.clear();
        self.list.clear();
    }

    fn reserve(&mut self, nodes: usize, edges: usize) {
        self.offsets.reserve_exact(nodes.saturating_add(1));
        self.list.reserve_exact(edges);
    }

    #[inline]
    fn of(&self, n: usize) -> &[E] {
        let (Some(&a), Some(&b)) = (self.offsets.get(n), self.offsets.get(n.wrapping_add(1)))
        else {
            return &[];
        };
        self.list.get(a as usize..b as usize).unwrap_or(&[])
    }
}

/// One run's graph, numbered locally: nodes by their place in `GD_nlist`, edges by their place in
/// the in-lists. Every list keeps its order, and `ns.c` never compares two numbers but to see
/// whether they are the same.
#[derive(Default)]
pub(super) struct NsScratch {
    nlist: Vec<usize>,
    /// Local node to arena node, and edge to arena edge.
    nodes: Vec<N>,
    edges: Vec<E>,
    /// Arena node to local node, and edge to local edge. Only this run's entries mean anything.
    node_local: Vec<u32>,
    edge_local: Vec<u32>,
    rank: Vec<i32>,
    mark: Vec<usize>,
    priority: Vec<i32>,
    low: Vec<i32>,
    lim: Vec<i32>,
    par: Vec<u32>,
    subtree: Vec<u32>,
    in_lists: Lists,
    out_lists: Lists,
    incoming: Adjacency,
    outgoing: Adjacency,
    tail: Vec<u32>,
    head: Vec<u32>,
    minlen: Vec<i32>,
    weight: Vec<i32>,
    cutvalue: Vec<i32>,
    tree_index: Vec<i32>,
    /// The cut-set pivots' storage (`common::ns::cut`), kept across calls.
    cut: ns::CutStore,
}

impl NsScratch {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// `graph_t *g` as seen by `ns.c`: dot's arenas plus the graph being ranked.
struct DotNs<'x, 'a> {
    d: &'x mut Dot<'a>,
    g: G,
    scratch: &'x mut NsScratch,
}

impl<'x, 'a> DotNs<'x, 'a> {
    fn new(d: &'x mut Dot<'a>, g: G, scratch: &'x mut NsScratch) -> Self {
        scratch.nlist.clear();
        scratch.nodes.clear();
        scratch.edges.clear();
        scratch.rank.clear();
        scratch.mark.clear();
        scratch.priority.clear();
        scratch.low.clear();
        scratch.lim.clear();
        scratch.subtree.clear();
        scratch.in_lists.clear();
        scratch.out_lists.clear();
        if scratch.node_local.len() < d.nodes.len() {
            scratch.node_local.resize(d.nodes.len(), 0);
        }
        if scratch.edge_local.len() < d.edges.len() {
            scratch.edge_local.resize(d.edges.len(), 0);
        }
        // sizes first, so that nothing below grows step by step
        let (mut count, mut edges) = (0usize, 0usize);
        let mut n = d.graphs[g].nlist;
        while let Some(node) = n.and_then(|x| d.nodes.get(x)) {
            count = count.wrapping_add(1);
            edges = edges.wrapping_add(node.in_.len());
            n = node.next;
        }
        scratch.nlist.reserve_exact(count);
        scratch.nodes.reserve_exact(count);
        scratch.rank.reserve_exact(count);
        scratch.mark.reserve_exact(count);
        scratch.priority.reserve_exact(count);
        scratch.low.reserve_exact(count);
        scratch.lim.reserve_exact(count);
        scratch.subtree.reserve_exact(count);
        scratch.par.clear();
        scratch.par.reserve_exact(count);
        scratch.edges.reserve_exact(edges);
        scratch.in_lists.reserve(count, edges);
        scratch.out_lists.reserve(count, edges);
        scratch.incoming.reserve(edges);
        scratch.outgoing.reserve(edges);
        let mut n = d.graphs[g].nlist;
        while let Some(x) = n {
            if let Some(local) = scratch.node_local.get_mut(x) {
                *local = scratch.nodes.len() as u32;
            }
            scratch.nlist.push(scratch.nodes.len());
            scratch.nodes.push(x);
            n = d.nodes.get(x).and_then(|node| node.next);
        }
        scratch.incoming.resize_nodes(count);
        scratch.outgoing.resize_nodes(count);
        scratch.in_lists.offsets.push(0);
        scratch.out_lists.offsets.push(0);
        for (local, &x) in scratch.nodes.iter().enumerate() {
            let Some(node) = d.nodes.get(x) else {
                debug_assert!(false, "NS node out of bounds");
                scratch.rank.push(0);
                scratch.mark.push(0);
                scratch.priority.push(0);
                scratch.low.push(0);
                scratch.lim.push(0);
                scratch.subtree.push(NONE);
                scratch
                    .in_lists
                    .offsets
                    .push(scratch.in_lists.list.len() as u32);
                scratch.out_lists.offsets.push(0);
                continue;
            };
            scratch.rank.push(node.rank);
            scratch.mark.push(node.mark);
            scratch.priority.push(node.priority);
            scratch.low.push(node.low);
            scratch.lim.push(node.lim);
            scratch.subtree.push(pack(node.subtree));
            scratch.incoming.prepare(local, node.in_.len());
            scratch.outgoing.prepare(local, node.out.len());
            for &e in &node.in_ {
                if let Some(slot) = scratch.edge_local.get_mut(e) {
                    *slot = scratch.edges.len() as u32;
                }
                scratch.in_lists.list.push(scratch.edges.len());
                scratch.edges.push(e);
            }
            scratch
                .in_lists
                .offsets
                .push(scratch.in_lists.list.len() as u32);
        }
        // The out-lists, now that every edge has its number.
        for &x in &scratch.nodes {
            if let Some(node) = d.nodes.get(x) {
                for &e in &node.out {
                    let local = scratch.edge_local.get(e).copied().unwrap_or(0) as usize;
                    debug_assert_eq!(
                        scratch.edges.get(local),
                        Some(&e),
                        "an out-edge in no in-list"
                    );
                    scratch.out_lists.list.push(local);
                }
            }
            scratch
                .out_lists
                .offsets
                .push(scratch.out_lists.list.len() as u32);
        }
        debug_assert_eq!(edges, scratch.edges.len());
        let edges = scratch.edges.len();
        scratch.tail.resize(edges, 0);
        scratch.head.resize(edges, 0);
        scratch.minlen.resize(edges, 0);
        scratch.weight.resize(edges, 0);
        scratch.cutvalue.resize(edges, 0);
        scratch.tree_index.resize(edges, 0);
        for (local, &e) in scratch.edges.iter().enumerate() {
            let Some(edge) = d.edges.get(e) else {
                debug_assert!(false, "NS edge out of bounds");
                continue;
            };
            if let (
                Some(tail),
                Some(head),
                Some(minlen),
                Some(weight),
                Some(cutvalue),
                Some(tree_index),
            ) = (
                scratch.tail.get_mut(local),
                scratch.head.get_mut(local),
                scratch.minlen.get_mut(local),
                scratch.weight.get_mut(local),
                scratch.cutvalue.get_mut(local),
                scratch.tree_index.get_mut(local),
            ) {
                *tail = scratch.node_local.get(edge.tail).copied().unwrap_or(0);
                *head = scratch.node_local.get(edge.head).copied().unwrap_or(0);
                *minlen = edge.minlen;
                *weight = edge.weight;
                *cutvalue = edge.cutvalue;
                *tree_index = edge.tree_index;
            }
        }
        for &x in &scratch.nodes {
            let old = d.nodes.get(x).and_then(|node| node.par);
            scratch.par.push(match old {
                None => NONE,
                Some(e) => {
                    let local = scratch.edge_local.get(e).copied().unwrap_or(FOREIGN);
                    if scratch.edges.get(local as usize) == Some(&e) {
                        local
                    } else {
                        FOREIGN
                    }
                }
            });
        }
        Self { d, g, scratch }
    }

    /// The arena's record of local node `n`.
    fn arena_node(&self, n: usize) -> Option<&NodeInfo> {
        self.scratch.nodes.get(n).and_then(|&x| self.d.nodes.get(x))
    }

    fn finish(self) {
        let scratch = &*self.scratch;
        for (local, &x) in scratch.nodes.iter().enumerate() {
            let tree_in = scratch.incoming.edges(local);
            let tree_out = scratch.outgoing.edges(local);
            let Some(node) = self.d.nodes.get_mut(x) else {
                debug_assert!(false, "NS node out of bounds");
                continue;
            };
            if let (
                Some(&rank),
                Some(&mark),
                Some(&priority),
                Some(&low),
                Some(&lim),
                Some(&par),
                Some(&subtree),
            ) = (
                scratch.rank.get(local),
                scratch.mark.get(local),
                scratch.priority.get(local),
                scratch.low.get(local),
                scratch.lim.get(local),
                scratch.par.get(local),
                scratch.subtree.get(local),
            ) {
                node.rank = rank;
                node.mark = mark;
                node.priority = priority;
                node.low = low;
                node.lim = lim;
                if par != FOREIGN {
                    node.par = unpack(par).and_then(|e| scratch.edges.get(e).copied());
                }
                node.subtree = unpack(subtree);
            }
            node.tree_in.clear();
            node.tree_in
                .extend(tree_in.iter().filter_map(|&e| scratch.edges.get(e)));
            node.tree_out.clear();
            node.tree_out
                .extend(tree_out.iter().filter_map(|&e| scratch.edges.get(e)));
        }
        for (local, &e) in scratch.edges.iter().enumerate() {
            let Some(edge) = self.d.edges.get_mut(e) else {
                debug_assert!(false, "NS edge out of bounds");
                continue;
            };
            if let (Some(&cutvalue), Some(&tree_index)) =
                (scratch.cutvalue.get(local), scratch.tree_index.get(local))
            {
                edge.cutvalue = cutvalue;
                edge.tree_index = tree_index;
            }
        }
    }
}

impl NsGraph for DotNs<'_, '_> {
    fn nlist(&self) -> &[usize] {
        &self.scratch.nlist
    }
    fn attr(&self, name: &str) -> Option<String> {
        self.d.agget(self.g, name).map(str::to_string)
    }
    fn is_normal(&self, n: usize) -> bool {
        self.arena_node(n).is_some_and(|x| x.node_type == NORMAL)
    }
    fn dump_name(&self, n: usize) -> String {
        self.arena_node(n)
            .map_or_else(String::new, |x| x.name.clone())
    }
    fn err_name(&self, n: usize) -> String {
        match self.arena_node(n) {
            Some(node) if node.node_type == NORMAL => node.name.clone(),
            _ => "%0".to_string(),
        }
    }
    fn rank(&self, n: usize) -> i32 {
        self.scratch.rank[n]
    }
    fn set_rank(&mut self, n: usize, r: i32) {
        self.scratch.rank[n] = r;
    }
    fn set_mark(&mut self, n: usize, m: bool) {
        self.scratch.mark[n] = m as usize;
    }
    fn priority(&self, n: usize) -> i32 {
        self.scratch.priority[n]
    }
    fn set_priority(&mut self, n: usize, p: i32) {
        self.scratch.priority[n] = p;
    }
    fn low(&self, n: usize) -> i32 {
        self.scratch.low[n]
    }
    fn set_low(&mut self, n: usize, v: i32) {
        self.scratch.low[n] = v;
    }
    fn lim(&self, n: usize) -> i32 {
        self.scratch.lim[n]
    }
    fn set_lim(&mut self, n: usize, v: i32) {
        self.scratch.lim[n] = v;
    }
    fn par(&self, n: usize) -> Option<usize> {
        unpack(self.scratch.par[n])
    }
    fn set_par(&mut self, n: usize, e: Option<usize>) {
        self.scratch.par[n] = pack(e);
    }
    fn subtree(&self, n: usize) -> Option<usize> {
        unpack(self.scratch.subtree[n])
    }
    fn set_subtree(&mut self, n: usize, s: Option<usize>) {
        self.scratch.subtree[n] = pack(s);
    }
    fn in_edges(&self, n: usize) -> &[usize] {
        self.scratch.in_lists.of(n)
    }
    fn out_edges(&self, n: usize) -> &[usize] {
        self.scratch.out_lists.of(n)
    }
    fn tree_in(&self, n: usize) -> &[usize] {
        self.scratch.incoming.edges(n)
    }
    fn tree_out(&self, n: usize) -> &[usize] {
        self.scratch.outgoing.edges(n)
    }
    fn tree_in_push(&mut self, n: usize, e: usize) -> bool {
        self.scratch.incoming.push(n, e)
    }
    fn tree_out_push(&mut self, n: usize, e: usize) -> bool {
        self.scratch.outgoing.push(n, e)
    }
    fn tree_in_remove(&mut self, n: usize, e: usize) {
        self.scratch.incoming.remove(n, e);
    }
    fn tree_out_remove(&mut self, n: usize, e: usize) {
        self.scratch.outgoing.remove(n, e);
    }
    fn tree_in_clear(&mut self, n: usize) {
        self.scratch.incoming.clear(n);
    }
    fn tree_out_clear(&mut self, n: usize) {
        self.scratch.outgoing.clear(n);
    }
    fn tail(&self, e: usize) -> usize {
        self.scratch.tail[e] as usize
    }
    fn head(&self, e: usize) -> usize {
        self.scratch.head[e] as usize
    }
    fn minlen(&self, e: usize) -> i32 {
        self.scratch.minlen[e]
    }
    fn weight(&self, e: usize) -> i32 {
        self.scratch.weight[e]
    }
    fn cutvalue(&self, e: usize) -> i32 {
        self.scratch.cutvalue[e]
    }
    fn set_cutvalue(&mut self, e: usize, v: i32) {
        self.scratch.cutvalue[e] = v;
    }
    fn tree_index(&self, e: usize) -> i32 {
        self.scratch.tree_index[e]
    }
    fn set_tree_index(&mut self, e: usize, v: i32) {
        self.scratch.tree_index[e] = v;
    }
    fn take_cut_store(&mut self) -> ns::CutStore {
        std::mem::take(&mut self.scratch.cut)
    }
    fn keep_cut_store(&mut self, store: ns::CutStore) {
        self.scratch.cut = store;
    }
}

impl Dot<'_> {
    /// `rank2(g, balance, maxiter, search_size)`
    pub fn rank2(
        &mut self,
        g: G,
        balance: i32,
        maxiter: i32,
        search_size: i32,
    ) -> Result<i32, String> {
        let mut scratch = std::mem::take(&mut self.ns_scratch);
        let result = {
            let mut ns = DotNs::new(self, g, &mut scratch);
            let result = ns::rank2(&mut ns, balance, maxiter, search_size);
            ns.finish();
            result
        };
        self.ns_scratch = scratch;
        result
    }

    /// `rank(g, balance, maxiter)`
    pub fn ns_rank(&mut self, g: G, balance: i32, maxiter: i32) -> Result<i32, String> {
        let search_size = match self.agget(g, "searchsize") {
            Some(s) => atoi(s),
            None => SEARCHSIZE,
        };
        self.rank2(g, balance, maxiter, search_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphviz_pure_core::{Graph, GraphKind};

    #[test]
    fn tree_list_preserves_swap_remove_order_and_capacity() {
        let mut adjacency = Adjacency::default();
        adjacency.resize_nodes(1);
        adjacency.prepare(0, 3);
        for e in [10, 20, 30] {
            assert!(adjacency.push(0, e));
        }
        assert!(!adjacency.push(0, 40));
        adjacency.remove(0, 20);
        assert_eq!(adjacency.edges(0), &[10, 30]);
    }

    /// A random DAG with its arena numbered unlike its list: unused nodes and edges between the
    /// used ones, as dot's arenas have after earlier phases.
    fn sparse_dag(seed: u64, input: &Graph) -> (Dot<'_>, ns::NsData) {
        let mut x = seed;
        let mut below = |n: usize| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_f491_4f6c_dd1d) % n as u64) as usize
        };
        let count = 70 + below(120);
        let mut d = Dot::empty(input);
        d.graphs.push(GraphInfo::default());
        // list order against arena order: every third arena node is unused
        let mut order: Vec<usize> = (0..count).map(|i| i * 3 / 2 + 1).collect();
        for i in (1..count).rev() {
            order.swap(i, below(i + 1));
        }
        d.nodes = vec![NodeInfo::default(); count * 3 / 2 + 3];
        d.graphs[0].nlist = Some(order[0]);
        for (a, b) in order.iter().zip(order.iter().skip(1)) {
            d.nodes[*a].next = Some(*b);
        }
        for &n in &order {
            d.nodes[n].node_type = NORMAL;
            // left by an earlier run, on an edge of no graph: C overwrites it unread
            d.nodes[n].par = Some(1);
        }
        let mut data = ns::NsData {
            nodes: vec![ns::NsNode::default(); d.nodes.len()],
            nlist: order.clone(),
            ..ns::NsData::default()
        };
        d.edges.push(EdgeInfo::default());
        d.edges.push(EdgeInfo::default());
        data.edges.push(ns::NsEdge::default());
        data.edges.push(ns::NsEdge::default());
        let mut add = |d: &mut Dot<'_>, t: usize, h: usize, minlen: i32, weight: i32| {
            let e = d.edges.len();
            d.edges.push(EdgeInfo {
                tail: t,
                head: h,
                minlen,
                weight,
                ..EdgeInfo::default()
            });
            d.nodes[t].out.push(e);
            d.nodes[h].in_.push(e);
            data.edges.push(ns::NsEdge {
                tail: t,
                head: h,
                minlen,
                weight,
                ..ns::NsEdge::default()
            });
            data.nodes[t].out.push(e);
            data.nodes[h].in_.push(e);
        };
        let ids: Vec<usize> = (0..count).map(|i| i * 3 / 2 + 1).collect();
        for h in 1..count {
            let t = below(h);
            add(&mut d, ids[t], ids[h], 1 + below(3) as i32, below(4) as i32);
        }
        for _ in 0..count {
            let (a, b) = (below(count), below(count));
            if a != b {
                add(&mut d, ids[a.min(b)], ids[a.max(b)], 1, below(4) as i32);
            }
        }
        (d, data)
    }

    #[test]
    fn local_numbers_rank_like_the_arena_numbers() {
        for seed in 1..=120u64 {
            for balance in [0, 1, 2] {
                let input = Graph::new("G", GraphKind::Directed, false);
                let (mut d, mut data) = sparse_dag(seed.wrapping_mul(0x9e37_79b9) | 1, &input);
                let a = d.rank2(0, balance, i32::MAX, SEARCHSIZE);
                let b = ns::rank2(&mut data, balance, i32::MAX, SEARCHSIZE);
                assert_eq!(a, b, "seed {seed}, balance {balance}");
                for &n in &data.nlist {
                    let (node, other) = (&d.nodes[n], &data.nodes[n]);
                    assert_eq!(node.rank, other.rank, "rank of {n}, seed {seed}");
                    assert_eq!(node.low, other.low, "low of {n}, seed {seed}");
                    assert_eq!(node.lim, other.lim, "lim of {n}, seed {seed}");
                    assert_eq!(node.tree_in, other.tree_in, "tree_in of {n}, seed {seed}");
                    assert_eq!(
                        node.tree_out, other.tree_out,
                        "tree_out of {n}, seed {seed}"
                    );
                }
                for (e, other) in data.edges.iter().enumerate().skip(2) {
                    assert_eq!(d.edges[e].cutvalue, other.cutvalue, "edge {e}, seed {seed}");
                    assert_eq!(
                        d.edges[e].tree_index, other.tree_index,
                        "edge {e}, seed {seed}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_exit_leaves_the_tree_lists_empty() {
        // so their numbering is never seen outside a run, on success or on error
        let input = Graph::new("G", GraphKind::Directed, false);
        for balance in [0, 1, 2] {
            for weight in [1, 2_000_000_000] {
                let (mut d, _) = sparse_dag(0x9e37_79b9_7f4a_7c15, &input);
                for edge in d.edges.iter_mut().skip(2) {
                    edge.weight = weight;
                }
                let _ = d.rank2(0, balance, i32::MAX, SEARCHSIZE);
                for (n, node) in d.nodes.iter().enumerate() {
                    assert!(node.tree_in.is_empty(), "tree_in of {n}, balance {balance}");
                    assert!(
                        node.tree_out.is_empty(),
                        "tree_out of {n}, balance {balance}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_stale_parent_outside_the_run_is_kept_if_the_run_does_not_reach_it() {
        let input = Graph::new("G", GraphKind::Directed, false);
        let mut d = Dot::empty(&input);
        d.graphs.push(GraphInfo {
            nlist: Some(1),
            ..GraphInfo::default()
        });
        d.nodes = vec![NodeInfo::default(); 3];
        d.edges = vec![EdgeInfo::default(); 8];
        d.nodes[1].node_type = NORMAL;
        d.nodes[1].par = Some(7);
        // maxiter 0 with one node: the run writes nothing into the parent pointer
        let mut scratch = NsScratch::default();
        let ns = DotNs::new(&mut d, 0, &mut scratch);
        assert_eq!(ns.scratch.par[0], FOREIGN);
        ns.finish();
        assert_eq!(d.nodes[1].par, Some(7));
    }

    #[test]
    fn rank_error_still_writes_back_scratch() {
        let input = Graph::new("G", GraphKind::Directed, false);
        let mut d = Dot::empty(&input);
        d.graphs.push(GraphInfo {
            nlist: Some(0),
            ..GraphInfo::default()
        });
        d.nodes = vec![NodeInfo::default(); 4];
        for (i, node) in d.nodes.iter_mut().enumerate() {
            node.node_type = NORMAL;
            node.next = (i < 3).then_some(i + 1);
        }
        d.nodes[0].out = vec![0, 1];
        d.nodes[1].in_ = vec![0];
        d.nodes[1].out = vec![2];
        d.nodes[2].in_ = vec![1];
        d.nodes[2].out = vec![3];
        d.nodes[3].in_ = vec![2, 3];
        for (tail, head) in [(0, 1), (0, 2), (1, 3), (2, 3)] {
            d.edges.push(EdgeInfo {
                tail,
                head,
                minlen: 1,
                weight: 2_000_000_000,
                ..EdgeInfo::default()
            });
        }

        let result = d.rank2(0, 0, i32::MAX, SEARCHSIZE);
        assert_eq!(result, Err(ns::OVERFLOW_EDGE_WEIGHT_SUM.to_string()));
        assert!(d.nodes.iter().any(|node| node.rank != 0));
        for (i, node) in d.nodes.iter().enumerate() {
            assert_eq!(node.rank, d.ns_scratch.rank[i]);
            assert_eq!(node.low, d.ns_scratch.low[i]);
            assert_eq!(node.lim, d.ns_scratch.lim[i]);
            assert_eq!(node.par, unpack(d.ns_scratch.par[i]));
            assert_eq!(node.tree_in.as_slice(), d.ns_scratch.incoming.edges(i));
            assert_eq!(node.tree_out.as_slice(), d.ns_scratch.outgoing.edges(i));
        }
        for (i, edge) in d.edges.iter().enumerate() {
            assert_eq!(edge.cutvalue, d.ns_scratch.cutvalue[i]);
            assert_eq!(edge.tree_index, d.ns_scratch.tree_index[i]);
        }
    }
}
